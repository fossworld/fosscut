//! Musical beats (the `beats` analyzer): the tempo, every beat with its strength, and the
//! downbeats when the accents make them plain.
//!
//! 1. **Onset strength.** The mono mix is averaged down to 12 kHz (four 48 kHz samples
//!    each, a box anti-aliasing filter that is enough for onsets) and cut into 32 ms Hann
//!    windows every 4 ms, so the envelope has 250 frames a second. Each window's magnitude
//!    spectrum is summed into 40 triangular mel bands from 30 Hz to 5.8 kHz and compressed
//!    as `ln(1 + 1000 m)`, with `m` scaled so that a full-scale sine reads 1. The onset
//!    strength is the mean over bands of the increase from the previous window (the
//!    positive spectral flux): Bello, Daudet, Abdallah, Duxbury, Davies and Sandler, "A
//!    tutorial on onset detection in music signals", IEEE TSAP 2005; Müller, "Fundamentals
//!    of Music Processing", Springer 2015, section 6.1. Its local average over 200 ms is
//!    subtracted, negative values are dropped, it is smoothed with a Gaussian of 8 ms and
//!    divided by its standard deviation. A frame is timed 8 ms before the end of its
//!    window: on clicks and drum hits, the envelope peaks that long after a sharp onset.
//! 2. **Tempo.** The envelope's autocorrelation over the lags of `min_bpm` to `max_bpm`,
//!    weighted by a log-Gaussian of one octave around `prior_bpm` (the perceptual weighting
//!    of Ellis 2007, below), gives the beat period at its highest point, refined between
//!    lags with a parabola. A given `bpm` replaces the estimate.
//! 3. **Beats** by dynamic programming (Ellis, "Beat tracking by dynamic programming",
//!    Journal of New Music Research 36(1), 2007): each frame's best score is its onset
//!    strength plus the best score of a previous beat between half and twice the period
//!    before it, less `tightness` times the squared logarithm of the gap over the period. The
//!    best-scoring frame in the last two periods ends the chain, which is traced back.
//!    Leading and trailing beats weaker than half the envelope's root mean square are
//!    dropped, as onsets that are not there.
//! 4. **Timing.** Each beat moves to the envelope's peak within two frames, refined with a
//!    parabola, and then to the 1 ms block within 30 ms where the energy of the 48 kHz mix
//!    jumps most over the 8 ms before it, when that jump is at least fourfold (6 dB): the
//!    attack of the drum or click itself.
//! 5. **Tempo reported** from the beats: the slope of a line fitted through their times
//!    against their count. **Confidence** is the normalized autocorrelation of the envelope
//!    at the beat period, from 0 (no periodicity) to 1.
//! 6. **Downbeats.** Each beat's accent is the loudest 1 ms level in the 45 ms around its
//!    attack. For bars of 4 and of 3 beats, the accents are averaged by position in the
//!    bar; when the strongest position stands out from the others by at least a fifth of
//!    its level, the bar length with the clearer contrast is taken and its strongest
//!    position marks the downbeats. That contrast is reported as their confidence.
//!
//! Everything is deterministic: the same samples and options give the same beats.

use std::sync::Arc;

use fosscut_time::{FLICKS_PER_SECOND, Flicks};
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Moment;

/// The sample rate the tracker reads.
pub const RATE: u32 = 48_000;
/// Envelope frames per second.
pub const FRAMES_PER_SECOND: f64 = 250.0;
/// 48 kHz samples averaged into one 12 kHz sample.
const DECIMATION: usize = 4;
/// The rate the spectra are taken at.
const WORK_RATE: f64 = 12_000.0;
/// Window length at 12 kHz: 32 ms.
const WINDOW: usize = 384;
/// FFT length, the window padded with zeros.
const FFT_SIZE: usize = 512;
/// Window step at 12 kHz: 4 ms.
const HOP: usize = 48;
/// Mel bands.
const BANDS: usize = 40;
/// Lowest and highest band edges in hertz.
const BAND_RANGE: (f64, f64) = (30.0, 5_800.0);
/// Compression of band magnitudes.
const GAMMA: f32 = 1000.0;
/// 48 kHz samples in a block of the fine energy envelope: 1 ms.
const BLOCK: usize = 48;
/// Width of the tempo prior, in octaves.
const PRIOR_OCTAVES: f64 = 1.0;
/// How far a beat may move to an attack, in milliseconds.
const REFINE_MS: i64 = 30;
/// How much the energy must jump in a block, over the blocks before it, to be an attack.
const ATTACK_RISE: f64 = 4.0;
/// The blocks an attack is compared with.
const ATTACK_HISTORY: usize = 8;
/// The shortest audio that is tracked, in envelope frames (1.5 s).
const MIN_FRAMES: usize = 375;
/// How long after a sharp onset the envelope peaks, in frames (8 ms), measured on clicks
/// and noise bursts.
const LATENCY_FRAMES: f64 = 2.0;
/// The contrast of accents that makes downbeats.
const DOWNBEAT_CONTRAST: f64 = 0.2;

/// Options of the `beats` analyzer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BeatOptions {
    /// Slowest tempo considered, in beats per minute (default 50).
    pub min_bpm: f64,
    /// Fastest tempo considered (default 220).
    pub max_bpm: f64,
    /// The tempo the estimate leans towards when the music allows two, such as 60 or 120
    /// for a 120 bpm song (default 120).
    pub prior_bpm: f64,
    /// A known tempo, used instead of estimating one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    /// How strictly beats keep an even tempo against following the onsets, from 1 to 1000
    /// (default 100).
    pub tightness: f64,
}

impl Default for BeatOptions {
    fn default() -> BeatOptions {
        BeatOptions {
            min_bpm: 50.0,
            max_bpm: 220.0,
            prior_bpm: 120.0,
            bpm: None,
            tightness: 100.0,
        }
    }
}

impl BeatOptions {
    /// The options as they are used: every value inside its range, so equal results share
    /// one cache key.
    pub fn clamped(&self) -> BeatOptions {
        let defaults = BeatOptions::default();
        let finite = |v: f64, d: f64| if v.is_finite() { v } else { d };
        let min_bpm = finite(self.min_bpm, defaults.min_bpm).clamp(20.0, 300.0);
        let max_bpm = finite(self.max_bpm, defaults.max_bpm).clamp(min_bpm + 1.0, 400.0);
        BeatOptions {
            min_bpm,
            max_bpm,
            prior_bpm: finite(self.prior_bpm, defaults.prior_bpm).clamp(min_bpm, max_bpm),
            bpm: self
                .bpm
                .filter(|b| b.is_finite())
                .map(|b| b.clamp(20.0, 400.0)),
            tightness: finite(self.tightness, defaults.tightness).clamp(1.0, 1000.0),
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One beat.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Beat {
    /// When it falls, from the start of the file.
    pub time: Moment,
    /// How strong its onset is next to the others, from 0 to 1.
    pub strength: f64,
    /// Whether it starts a bar.
    #[serde(default, skip_serializing_if = "is_false")]
    pub downbeat: bool,
}

/// The beats of a stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Beats {
    /// The tempo in beats per minute, to 0.01; absent when no beat was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo_bpm: Option<f64>,
    /// How periodic the onsets are at that tempo, from 0 to 1.
    pub confidence: f64,
    /// Beats in a bar (3 or 4) when downbeats were found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beats_per_bar: Option<u32>,
    /// How clearly the downbeats stand out from the other beats, from 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downbeat_confidence: Option<f64>,
    /// Every beat, in order.
    pub beats: Vec<Beat>,
}

impl Beats {
    /// No beats.
    fn none() -> Beats {
        Beats {
            tempo_bpm: None,
            confidence: 0.0,
            beats_per_bar: None,
            downbeat_confidence: None,
            beats: Vec::new(),
        }
    }
}

/// Streams interleaved 48 kHz samples and reports their beats.
pub struct BeatTracker {
    options: BeatOptions,
    channels: usize,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    bands: Vec<Vec<(usize, f32)>>,
    /// The last [`WINDOW`] samples at 12 kHz, oldest at `at`.
    ring: Vec<f32>,
    at: usize,
    since_hop: usize,
    decimated: (f32, usize),
    previous: Vec<f32>,
    novelty: Vec<f32>,
    block: (f64, usize),
    /// Mean square of each 1 ms block of the 48 kHz mix.
    blocks: Vec<f32>,
    buffer: Vec<Complex32>,
    scratch: Vec<Complex32>,
}

/// The mel scale (O'Shaughnessy's formula).
fn mel(hz: f64) -> f64 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn hz(mel: f64) -> f64 {
    700.0 * (10f64.powf(mel / 2595.0) - 1.0)
}

/// Triangular mel bands over the FFT bins, each with weights summing to 1.
fn mel_bands() -> Vec<Vec<(usize, f32)>> {
    let (low, high) = (mel(BAND_RANGE.0), mel(BAND_RANGE.1));
    let edges: Vec<f64> = (0..BANDS + 2)
        .map(|i| hz(low + (high - low) * i as f64 / (BANDS + 1) as f64))
        .collect();
    let bin_hz = WORK_RATE / FFT_SIZE as f64;
    (0..BANDS)
        .map(|b| {
            let (lo, centre, hi) = (edges[b], edges[b + 1], edges[b + 2]);
            let mut weights: Vec<(usize, f32)> = (1..=FFT_SIZE / 2)
                .filter_map(|k| {
                    let f = k as f64 * bin_hz;
                    let w = if f <= lo || f >= hi {
                        0.0
                    } else if f <= centre {
                        (f - lo) / (centre - lo)
                    } else {
                        (hi - f) / (hi - centre)
                    };
                    (w > 0.0).then_some((k, w as f32))
                })
                .collect();
            if weights.is_empty() {
                let nearest = ((centre / bin_hz).round() as usize).clamp(1, FFT_SIZE / 2);
                weights.push((nearest, 1.0));
            }
            let total: f32 = weights.iter().map(|w| w.1).sum();
            for w in &mut weights {
                w.1 /= total;
            }
            weights
        })
        .collect()
}

impl BeatTracker {
    /// A tracker for `channels` interleaved channels at 48 kHz.
    pub fn new(channels: u16, options: BeatOptions) -> BeatTracker {
        let window = (0..WINDOW)
            .map(|n| {
                let phase = std::f32::consts::PI * (n as f32 + 0.5) / WINDOW as f32;
                phase.sin() * phase.sin()
            })
            .collect();
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let scratch = vec![Complex32::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        BeatTracker {
            options: options.clamped(),
            channels: usize::from(channels.max(1)),
            fft,
            window,
            bands: mel_bands(),
            ring: vec![0.0; WINDOW],
            at: 0,
            since_hop: 0,
            decimated: (0.0, 0),
            previous: vec![0.0; BANDS],
            novelty: Vec::new(),
            block: (0.0, 0),
            blocks: Vec::new(),
            buffer: vec![Complex32::new(0.0, 0.0); FFT_SIZE],
            scratch,
        }
    }

    /// Adds interleaved samples; a trailing partial frame is ignored.
    pub fn push(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channels) {
            let mono = frame.iter().sum::<f32>() / frame.len() as f32;
            self.block.0 += f64::from(mono) * f64::from(mono);
            self.block.1 += 1;
            if self.block.1 == BLOCK {
                self.blocks.push((self.block.0 / BLOCK as f64) as f32);
                self.block = (0.0, 0);
            }
            self.decimated.0 += mono;
            self.decimated.1 += 1;
            if self.decimated.1 == DECIMATION {
                let sample = self.decimated.0 / DECIMATION as f32;
                self.decimated = (0.0, 0);
                self.ring[self.at] = sample;
                self.at = (self.at + 1) % WINDOW;
                self.since_hop += 1;
                if self.since_hop == HOP {
                    self.since_hop = 0;
                    self.spectrum();
                }
            }
        }
    }

    /// The onset strength of the window that just ended.
    fn spectrum(&mut self) {
        for (k, value) in self.buffer.iter_mut().enumerate() {
            *value = if k < WINDOW {
                Complex32::new(self.ring[(self.at + k) % WINDOW] * self.window[k], 0.0)
            } else {
                Complex32::new(0.0, 0.0)
            };
        }
        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);
        // A full-scale sine's peak bin is a quarter of the window length.
        let scale = 4.0 / WINDOW as f32;
        let mut flux = 0.0;
        for (band, previous) in self.bands.iter().zip(&mut self.previous) {
            let magnitude: f32 = band
                .iter()
                .map(|&(k, w)| w * self.buffer[k].norm())
                .sum::<f32>()
                * scale;
            let level = (1.0 + GAMMA * magnitude).ln();
            flux += (level - *previous).max(0.0);
            *previous = level;
        }
        self.novelty.push(flux / BANDS as f32);
    }

    /// The beats of everything pushed.
    pub fn finish(self) -> Beats {
        let options = self.options.clone();
        let blocks = self.blocks;
        analyze_envelope(&self.novelty, &blocks, &options)
    }
}

/// The beats of mono or interleaved 48 kHz `samples` with `channels` channels.
pub fn beats(samples: &[f32], channels: u16, options: &BeatOptions) -> Beats {
    let mut tracker = BeatTracker::new(channels, options.clone());
    tracker.push(samples);
    tracker.finish()
}

/// The time of envelope frame `frame` (fractional), in seconds: the end of its window,
/// less the envelope's latency.
fn frame_seconds(frame: f64) -> f64 {
    (frame + 1.0 - LATENCY_FRAMES) * HOP as f64 / WORK_RATE
}

fn flicks(seconds: f64) -> Flicks {
    Flicks::new((seconds * FLICKS_PER_SECOND as f64).round() as i64)
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// The onset envelope after local-average subtraction, rectification, smoothing and
/// normalization; `None` when it is flat (silence).
fn envelope(novelty: &[f32]) -> Option<Vec<f32>> {
    let n = novelty.len();
    let mut prefix = vec![0f64; n + 1];
    for (i, v) in novelty.iter().enumerate() {
        prefix[i + 1] = prefix[i] + f64::from(*v);
    }
    let reach = 25;
    let detrended: Vec<f64> = (0..n)
        .map(|i| {
            let (lo, hi) = (i.saturating_sub(reach), (i + reach + 1).min(n));
            let mean = (prefix[hi] - prefix[lo]) / (hi - lo) as f64;
            (f64::from(novelty[i]) - mean).max(0.0)
        })
        .collect();
    let sigma = 2.0f64;
    let radius = (3.0 * sigma).ceil() as i64;
    let kernel: Vec<f64> = (-radius..=radius)
        .map(|d| (-(d * d) as f64 / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f64 = kernel.iter().sum();
    let smooth: Vec<f64> = (0..n as i64)
        .map(|i| {
            kernel
                .iter()
                .enumerate()
                .map(|(k, w)| {
                    let j = i + k as i64 - radius;
                    if (0..n as i64).contains(&j) {
                        w * detrended[j as usize]
                    } else {
                        0.0
                    }
                })
                .sum::<f64>()
                / total
        })
        .collect();
    let mean = smooth.iter().sum::<f64>() / n.max(1) as f64;
    let deviation =
        (smooth.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n.max(1) as f64).sqrt();
    // The raw flux of digital silence is exactly zero; anything this flat has no onsets.
    if deviation.is_nan() || deviation <= 1e-6 {
        return None;
    }
    Some(smooth.iter().map(|v| (v / deviation) as f32).collect())
}

/// The beat period in envelope frames: from `bpm`, or the prior-weighted autocorrelation's
/// highest lag, refined with a parabola.
fn period(envelope: &[f32], options: &BeatOptions) -> f64 {
    if let Some(bpm) = options.bpm {
        return 60.0 * FRAMES_PER_SECOND / bpm;
    }
    let shortest = ((60.0 * FRAMES_PER_SECOND / options.max_bpm).floor() as usize).max(2);
    let longest = ((60.0 * FRAMES_PER_SECOND / options.min_bpm).ceil() as usize)
        .min(envelope.len().saturating_sub(1))
        .max(shortest + 2);
    let prior = 60.0 * FRAMES_PER_SECOND / options.prior_bpm;
    let correlation = |lag: usize| -> f64 {
        envelope
            .iter()
            .zip(envelope.iter().skip(lag))
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum::<f64>()
    };
    let raw: Vec<f64> = (shortest - 1..=longest + 1).map(correlation).collect();
    let weight = |lag: f64| (-0.5 * ((lag / prior).log2() / PRIOR_OCTAVES).powi(2)).exp();
    let mut best = 1;
    for i in 1..raw.len() - 1 {
        let lag = (shortest - 1 + i) as f64;
        if raw[i] * weight(lag) > raw[best] * weight((shortest - 1 + best) as f64) {
            best = i;
        }
    }
    let (a, b, c) = (raw[best - 1], raw[best], raw[best + 1]);
    let curvature = a - 2.0 * b + c;
    let offset = if curvature < 0.0 {
        (0.5 * (a - c) / curvature).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    (shortest - 1 + best) as f64 + offset
}

/// The normalized autocorrelation of the mean-removed envelope at `lag`, from 0 to 1.
fn periodicity(envelope: &[f32], lag: f64) -> f64 {
    let n = envelope.len();
    let mean = envelope.iter().map(|v| f64::from(*v)).sum::<f64>() / n.max(1) as f64;
    let centred: Vec<f64> = envelope.iter().map(|v| f64::from(*v) - mean).collect();
    let energy: f64 = centred.iter().map(|v| v * v).sum();
    if energy <= 0.0 {
        return 0.0;
    }
    let at = |lag: usize| {
        centred
            .iter()
            .zip(centred.iter().skip(lag))
            .map(|(a, b)| a * b)
            .sum::<f64>()
    };
    let low = lag.floor() as usize;
    let t = lag - low as f64;
    let value = at(low) * (1.0 - t) + at(low + 1) * t;
    // The lag shortens the overlap; compensate so a perfect pulse train reads close to 1.
    let overlap = (n as f64 - lag).max(1.0) / n as f64;
    (value / energy / overlap).clamp(0.0, 1.0)
}

/// Beat frames by dynamic programming (Ellis 2007), traced back from the best-scoring
/// frame of the last two periods.
fn dynamic_beats(envelope: &[f32], period: f64, tightness: f64) -> Vec<usize> {
    let n = envelope.len();
    let far = ((2.0 * period).round() as usize).max(2);
    let near = ((period / 2.0).round() as usize).max(1);
    let penalty: Vec<f64> = (0..=far)
        .map(|gap| -tightness * (gap as f64 / period).ln().powi(2))
        .collect();
    let mut score = vec![0f64; n];
    let mut back: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        // A chain may start at any frame; one that keeps going scores more.
        let mut best = 0.0;
        let mut from = None;
        if i >= near {
            for j in i.saturating_sub(far)..=i - near {
                let candidate = score[j] + penalty[i - j];
                if candidate > best {
                    best = candidate;
                    from = Some(j);
                }
            }
        }
        score[i] = f64::from(envelope[i]) + best;
        back[i] = from;
    }
    let start = n.saturating_sub(far);
    let mut last = start;
    for i in start..n {
        if score[i] >= score[last] {
            last = i;
        }
    }
    let mut beats = vec![last];
    let mut at = last;
    while let Some(previous) = back[at] {
        beats.push(previous);
        at = previous;
    }
    beats.reverse();
    beats
}

/// The envelope frame with the highest value within two frames of `frame`, and the
/// peak's position refined with a parabola.
fn peak_near(envelope: &[f32], frame: usize) -> (f64, f32) {
    let n = envelope.len();
    let lo = frame.saturating_sub(2);
    let hi = (frame + 2).min(n - 1);
    let mut best = frame.min(n - 1);
    for i in lo..=hi {
        if envelope[i] > envelope[best] {
            best = i;
        }
    }
    let value = envelope[best];
    if best == 0 || best + 1 >= n {
        return (best as f64, value);
    }
    let (a, b, c) = (
        f64::from(envelope[best - 1]),
        f64::from(value),
        f64::from(envelope[best + 1]),
    );
    let curvature = a - 2.0 * b + c;
    let offset = if curvature < 0.0 {
        (0.5 * (a - c) / curvature).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    (best as f64 + offset, value)
}

/// The middle of the 1 ms block near `seconds` where the energy jumps most over the
/// blocks before it, when it jumps at least [`ATTACK_RISE`] times.
fn attack_near(blocks: &[f32], seconds: f64) -> Option<f64> {
    let centre = (seconds * 1000.0).round() as i64;
    let lo = (centre - REFINE_MS).max(ATTACK_HISTORY as i64);
    let hi = (centre + REFINE_MS).min(blocks.len() as i64 - 1);
    let mut best: Option<(f64, usize)> = None;
    for j in lo..=hi {
        let j = j as usize;
        let before = blocks[j - ATTACK_HISTORY..j]
            .iter()
            .map(|v| f64::from(*v))
            .sum::<f64>()
            / ATTACK_HISTORY as f64;
        let rise = f64::from(blocks[j]) / before.max(1e-10);
        if rise >= ATTACK_RISE && best.is_none_or(|(r, _)| rise > r) {
            best = Some((rise, j));
        }
    }
    best.map(|(_, j)| (j as f64 + 0.5) / 1000.0)
}

/// The loudest 1 ms level from 5 ms before to 40 ms after `seconds`, as an amplitude.
fn accent(blocks: &[f32], seconds: f64) -> f64 {
    let at = (seconds * 1000.0).round() as i64;
    let lo = (at - 5).max(0) as usize;
    let hi = ((at + 40).max(0) as usize).min(blocks.len());
    blocks
        .get(lo..hi)
        .unwrap_or(&[])
        .iter()
        .map(|v| f64::from(*v).sqrt())
        .fold(0.0, f64::max)
}

/// Bar length, downbeat position and contrast from the accents, when one stands out.
fn downbeats(accents: &[f64]) -> Option<(usize, usize, f64)> {
    let mut found: Option<(usize, usize, f64)> = None;
    for meter in [4usize, 3] {
        if accents.len() < 2 * meter {
            continue;
        }
        let means: Vec<f64> = (0..meter)
            .map(|phase| {
                let values: Vec<f64> = accents.iter().skip(phase).step_by(meter).copied().collect();
                values.iter().sum::<f64>() / values.len().max(1) as f64
            })
            .collect();
        let (phase, strongest) = means
            .iter()
            .copied()
            .enumerate()
            .fold((0, f64::MIN), |b, (i, m)| if m > b.1 { (i, m) } else { b });
        if strongest <= 0.0 {
            continue;
        }
        let others = (means.iter().sum::<f64>() - strongest) / (meter - 1) as f64;
        let contrast = (strongest - others) / strongest;
        if contrast >= DOWNBEAT_CONTRAST && found.is_none_or(|f| contrast > f.2) {
            found = Some((meter, phase, contrast));
        }
    }
    found
}

/// The beats from an onset envelope (`novelty`, one value per frame) and the fine energy
/// envelope (`blocks`, mean square per 1 ms).
fn analyze_envelope(novelty: &[f32], blocks: &[f32], options: &BeatOptions) -> Beats {
    if novelty.len() < MIN_FRAMES {
        return Beats::none();
    }
    let Some(envelope) = envelope(novelty) else {
        return Beats::none();
    };
    let period = period(&envelope, options);
    let frames = dynamic_beats(&envelope, period, options.tightness);
    // Leading and trailing beats without an onset are dropped.
    let rms = (envelope.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / envelope.len() as f64)
        .sqrt();
    let threshold = 0.5 * rms;
    let peaks: Vec<(f64, f32)> = frames.iter().map(|&f| peak_near(&envelope, f)).collect();
    let strong = |p: &(f64, f32)| f64::from(p.1) >= threshold;
    let (Some(first), Some(last)) = (
        peaks.iter().position(strong),
        peaks.iter().rposition(strong),
    ) else {
        return Beats::none();
    };
    let peaks = &peaks[first..=last];
    let times: Vec<f64> = peaks
        .iter()
        .map(|(frame, _)| {
            let coarse = frame_seconds(*frame);
            attack_near(blocks, coarse).unwrap_or(coarse)
        })
        .collect();
    // Strength next to the strong beats: the 95th percentile reads 1.
    let mut sorted: Vec<f64> = peaks.iter().map(|p| f64::from(p.1)).collect();
    sorted.sort_by(f64::total_cmp);
    let reference = sorted[((sorted.len() - 1) as f64 * 0.95).round() as usize].max(1e-9);
    let accents: Vec<f64> = times.iter().map(|&t| accent(blocks, t)).collect();
    let bars = downbeats(&accents);
    let beats: Vec<Beat> = times
        .iter()
        .zip(peaks)
        .enumerate()
        .map(|(i, (&t, peak))| Beat {
            time: Moment::new(flicks(t)),
            strength: round((f64::from(peak.1) / reference).min(1.0), 3),
            downbeat: bars.is_some_and(|(meter, phase, _)| i % meter == phase),
        })
        .collect();
    Beats {
        tempo_bpm: Some(round(tempo(&times, period), 2)),
        confidence: round(periodicity(&envelope, period), 3),
        beats_per_bar: bars.map(|(meter, _, _)| meter as u32),
        downbeat_confidence: bars.map(|(_, _, c)| round(c.min(1.0), 3)),
        beats,
    }
}

/// The tempo from beat times: the slope of a least-squares line through them against
/// their count, counting a gap of two periods as two beats; the envelope's period when
/// there are fewer than four.
fn tempo(times: &[f64], period: f64) -> f64 {
    let fallback = 60.0 * FRAMES_PER_SECOND / period;
    if times.len() < 4 {
        return fallback;
    }
    let mut gaps: Vec<f64> = times.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(f64::total_cmp);
    let typical = gaps[gaps.len() / 2];
    if typical <= 0.0 {
        return fallback;
    }
    let counts: Vec<f64> = times
        .iter()
        .map(|t| ((t - times[0]) / typical).round())
        .collect();
    let n = times.len() as f64;
    let (mc, mt) = (
        counts.iter().sum::<f64>() / n,
        times.iter().sum::<f64>() / n,
    );
    let covariance: f64 = counts
        .iter()
        .zip(times)
        .map(|(c, t)| (c - mc) * (t - mt))
        .sum();
    let variance: f64 = counts.iter().map(|c| (c - mc).powi(2)).sum();
    if variance <= 0.0 || covariance <= 0.0 {
        return fallback;
    }
    60.0 / (covariance / variance)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic pseudo-random sequence in -1..1 (a linear congruential generator).
    struct Noise(u64);

    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
        }
    }

    /// `seconds` of 48 kHz silence.
    fn silence(seconds: f64) -> Vec<f32> {
        vec![0.0; (seconds * f64::from(RATE)) as usize]
    }

    /// Adds `sound` to `signal` from `at` seconds.
    fn mix(signal: &mut [f32], at: f64, sound: &[f32]) {
        let start = (at * f64::from(RATE)).round() as usize;
        for (s, v) in signal.iter_mut().skip(start).zip(sound) {
            *s += v;
        }
    }

    /// A 5 ms click: a decaying 2 kHz tone.
    fn click(amplitude: f32) -> Vec<f32> {
        (0..240)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                amplitude * (2.0 * std::f32::consts::PI * 2000.0 * t).sin() * (-t / 0.0015).exp()
            })
            .collect()
    }

    /// A noise burst with an instant attack and a decay of `decay` seconds, optionally
    /// smoothed (`lowpass` near 1 darkens it) for a kick, or bright for a hat.
    fn burst(noise: &mut Noise, amplitude: f32, decay: f32, lowpass: f32) -> Vec<f32> {
        let length = (decay * 6.0 * RATE as f32) as usize;
        let mut state = 0.0f32;
        (0..length)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                state = lowpass * state + (1.0 - lowpass) * noise.next();
                amplitude * state * (-t / decay).exp()
            })
            .collect()
    }

    /// A click track at `bpm` for `seconds`, starting at `offset`, every click moved by
    /// up to `jitter` seconds either way, and every fourth twice as loud.
    fn clicks(bpm: f64, seconds: f64, offset: f64, jitter: f64) -> (Vec<f32>, Vec<f64>) {
        let mut signal = silence(seconds);
        let mut noise = Noise(11);
        let mut times = Vec::new();
        let period = 60.0 / bpm;
        let mut k = 0;
        loop {
            let t = offset + k as f64 * period + jitter * f64::from(noise.next());
            if t > seconds - 0.1 {
                break;
            }
            let amplitude = if k % 4 == 0 { 0.8 } else { 0.4 };
            mix(&mut signal, t, &click(amplitude));
            times.push(t);
            k += 1;
        }
        (signal, times)
    }

    /// For each true beat, the distance to the nearest found one, in seconds.
    fn errors(found: &Beats, truth: &[f64]) -> Vec<f64> {
        let at: Vec<f64> = found
            .beats
            .iter()
            .map(|b| b.time.flicks.as_seconds_f64())
            .collect();
        truth
            .iter()
            .map(|t| at.iter().map(|a| (a - t).abs()).fold(f64::MAX, f64::min))
            .collect()
    }

    #[test]
    fn a_jittered_click_track_gives_its_beats_tempo_and_downbeats() {
        let (signal, truth) = clicks(120.0, 20.0, 0.37, 0.012);
        let found = beats(&signal, 1, &BeatOptions::default());
        assert_eq!(found.beats.len(), truth.len(), "{found:?}");
        let worst = errors(&found, &truth).into_iter().fold(0.0, f64::max);
        assert!(worst < 0.010, "worst beat error {worst} s");
        let tempo = found.tempo_bpm.unwrap();
        assert!((tempo - 120.0).abs() < 1.0, "tempo {tempo}");
        assert!(found.confidence > 0.5, "confidence {}", found.confidence);
        assert_eq!(found.beats_per_bar, Some(4));
        let downbeats: Vec<usize> = found
            .beats
            .iter()
            .enumerate()
            .filter(|(_, b)| b.downbeat)
            .map(|(i, _)| i)
            .collect();
        assert!(downbeats.iter().all(|i| i % 4 == 0), "{downbeats:?}");
        assert!(found.downbeat_confidence.unwrap() > 0.3);
        // Stereo reads the same, and so does the stream fed in odd chunks.
        let stereo: Vec<f32> = signal.iter().flat_map(|&s| [s, s]).collect();
        let mut tracker = BeatTracker::new(2, BeatOptions::default());
        for chunk in stereo.chunks(2 * 4_801) {
            tracker.push(chunk);
        }
        assert_eq!(tracker.finish(), found);
    }

    #[test]
    fn drum_like_noise_bursts_at_90_bpm() {
        let seconds = 24.0;
        let mut signal = silence(seconds);
        let mut noise = Noise(5);
        let period = 60.0 / 90.0;
        let mut truth = Vec::new();
        let mut k = 0;
        loop {
            let t = 0.5 + k as f64 * period;
            if t > seconds - 0.5 {
                break;
            }
            // Kick on one and three, snare on two and four, quieter hats between.
            let hit = if k % 2 == 0 {
                burst(&mut noise, 0.9, 0.06, 0.95)
            } else {
                burst(&mut noise, 0.6, 0.04, 0.2)
            };
            mix(&mut signal, t, &hit);
            let hat = burst(&mut noise, 0.15, 0.01, 0.0);
            mix(&mut signal, t + period / 2.0, &hat);
            truth.push(t);
            k += 1;
        }
        let found = beats(&signal, 1, &BeatOptions::default());
        let tempo = found.tempo_bpm.unwrap();
        assert!((tempo - 90.0).abs() < 1.0, "tempo {tempo}");
        let errors = errors(&found, &truth);
        let worst = errors.iter().copied().fold(0.0, f64::max);
        assert!(worst < 0.010, "worst beat error {worst} s");
        assert!(found.beats.len() >= truth.len() && found.beats.len() <= truth.len() + 1);
    }

    #[test]
    fn silence_and_short_sound_give_no_beats() {
        let found = beats(&silence(10.0), 1, &BeatOptions::default());
        assert_eq!(found, Beats::none());
        assert!(found.beats.is_empty() && found.tempo_bpm.is_none());
        let (short, _) = clicks(120.0, 1.0, 0.1, 0.0);
        assert!(beats(&short, 1, &BeatOptions::default()).beats.is_empty());
    }

    #[test]
    fn a_given_tempo_is_kept_and_options_are_clamped() {
        let (signal, truth) = clicks(100.0, 15.0, 0.2, 0.0);
        let fixed = BeatOptions {
            bpm: Some(100.0),
            ..BeatOptions::default()
        };
        let found = beats(&signal, 1, &fixed);
        let worst = errors(&found, &truth).into_iter().fold(0.0, f64::max);
        assert!(worst < 0.005, "worst {worst}");
        let wild = BeatOptions {
            min_bpm: f64::NAN,
            max_bpm: 10.0,
            prior_bpm: 1e9,
            bpm: Some(f64::INFINITY),
            tightness: -3.0,
        }
        .clamped();
        assert_eq!(wild.min_bpm, 50.0);
        assert_eq!(wild.max_bpm, 51.0);
        assert_eq!(wild.prior_bpm, 51.0);
        assert_eq!(wild.bpm, None);
        assert_eq!(wild.tightness, 1.0);
        let defaults: BeatOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, BeatOptions::default());
        assert!(
            serde_json::to_value(&defaults)
                .unwrap()
                .get("bpm")
                .is_none()
        );
    }

    #[test]
    fn mel_bands_cover_the_spectrum_with_unit_weights() {
        let bands = mel_bands();
        assert_eq!(bands.len(), BANDS);
        for band in &bands {
            let total: f32 = band.iter().map(|w| w.1).sum();
            assert!((total - 1.0).abs() < 1e-4);
            assert!(band.iter().all(|w| w.0 >= 1 && w.0 <= FFT_SIZE / 2));
        }
        assert!((hz(mel(1000.0)) - 1000.0).abs() < 1e-6);
    }
}
