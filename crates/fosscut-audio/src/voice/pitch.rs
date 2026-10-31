//! Pitch shifting without changing the length.
//!
//! **Grains.** The output is an overlap-add of Hann grains two hops long (24 ms) on a fixed
//! grid of the stream. Grain `k` centred at output frame `c_k` reads the input around
//! `p_k`, resampled by the pitch ratio `r`: output frame `o` of the grain is input position
//! `p_k + r (o - c_k)`, read by cubic interpolation, so each grain's pitch is `r` times the
//! input's while its centre stays near `c_k` and the length is kept. Above `r = 1` the input
//! is first low-passed at 0.45 of the sample rate divided by `r` against aliasing.
//!
//! **Splices.** Where two grains overlap they must carry the same waveform in phase. As in
//! WSOLA (W. Verhelst and M. Roelands, "An overlap-add technique based on waveform
//! similarity (WSOLA) for high quality time-scale modification of speech", ICASSP 1993), grain
//! `k` is moved within 8 ms of its nominal place `c_k` to where the input best matches the
//! natural continuation of grain `k - 1`, `p_{k-1} + r H`: the normalised cross-correlation
//! is searched on a 12 kHz copy of the input and refined at the full rate.
//!
//! **Bounded memory.** Each choice depends on the one before, so the chain would remember
//! the whole clip. The grains are grouped in segments of 16 (0.192 s) on the stream's grid.
//! In a segment whose quietest grain is a real dip (a pause, a consonant: at most a tenth
//! of the segment's mean energy), the search restarts there, where a splice out of phase is
//! least audible: that grain sits at its nominal place. Every third segment restarts at its
//! quietest grain whatever its level, so a held note cannot keep the chain going. A restart
//! sets a held tone's phase back to the input's, which quantizes its frequency to steps of
//! one cycle per 0.576 s (0.9 Hz either way at most); speech restarts in its dips, where
//! nothing is heard. Any grain's placement depends only on the input since the restart
//! before it, at most four segments back, and a window of the stream fed [`warm_up`] frames
//! before and [`lookahead`] frames after a range places every grain there exactly as the
//! whole clip.
//!
//! **Formants.** Resampling grains moves the spectral envelope with the pitch, which is
//! what a chipmunk or giant voice should do. To keep it, the input is analysed in frames of
//! about 43 ms (75% overlap, Hann windows, on the stream's grid); each frame's envelope `E`
//! is the cepstrally smoothed log magnitude spectrum (liftered below 0.9 ms, which separates
//! it from harmonics of pitches up to about 1 kHz). The shifted sound's envelope is `E`
//! stretched by the ratio `r`, so the same frame of the shifted sound is multiplied by
//! `E(f) / E(f / r)`, limited to 18 dB either way, and scaled to keep the frame's energy,
//! keeping its phases (weighted overlap-add).

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::{BUTTERWORTH_4, Biquad, Chain, cubic, deinterleave, sample, seconds};

/// Hop between grains; grains are two hops long.
const HOP_SECONDS: f64 = 0.012;
/// How far a grain may move from its nominal place.
const SEARCH_SECONDS: f64 = 0.008;
/// The shortest stretch of waveform compared when placing a grain.
const COMPARE_SECONDS: f64 = 0.008;
/// Grains per segment: the search may restart once in each.
const SEGMENT: i64 = 16;
/// Every this many segments the search restarts whatever the level.
const FORCED_EVERY: i64 = 3;
/// A segment restarts at its quietest grain when that grain's energy is at most this share
/// of the segment's mean.
const DIP: f64 = 0.1;
/// Rate of the coarse search (whole frames of the full rate are summed down to it).
const COARSE_RATE: f64 = 12_000.0;
/// Formant frames: about this long, rounded up to a power of two, at 75% overlap.
const FORMANT_SECONDS: f64 = 0.04;
/// The envelope keeps quefrencies below this.
const LIFTER_SECONDS: f64 = 0.0009;
/// Most the formant correction raises or lowers a frequency, as a factor (18 dB).
const FORMANT_LIMIT: f64 = 8.0;
/// Time for the anti-aliasing filter to forget its past.
const FILTER_SETTLE_SECONDS: f64 = 0.01;

/// The pitch ratio for `semitones` (clamped to -12 to 12, NaN as 0), or `None` when the
/// pitch does not move.
pub(super) fn ratio(semitones: f32) -> Option<f64> {
    let semitones = if semitones.is_nan() {
        0.0
    } else {
        f64::from(semitones).clamp(-12.0, 12.0)
    };
    (semitones.abs() >= 1e-3).then(|| 2f64.powf(semitones / 12.0))
}

/// Sizes in frames at a sample rate and ratio.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    hop: i64,
    search: i64,
    compare: i64,
    /// Frames summed into one coarse sample.
    step: i64,
    /// The formant frame length.
    frame: i64,
    /// How far the input under a grain reaches from its centre (`r H`, rounded up).
    reach: i64,
}

impl Geometry {
    fn new(sample_rate: u32, ratio: f64) -> Self {
        let rate = f64::from(sample_rate);
        let hop = ((HOP_SECONDS * rate).round() as i64).max(4);
        let reach = (ratio * hop as f64).ceil() as i64;
        Geometry {
            hop,
            search: ((SEARCH_SECONDS * rate).round() as i64).max(1),
            compare: reach.max(seconds(COMPARE_SECONDS, rate) as i64).max(2),
            step: ((rate / COARSE_RATE).round() as i64).max(1),
            frame: ((FORMANT_SECONDS * rate).ceil() as usize)
                .next_power_of_two()
                .clamp(256, 8192) as i64,
            reach,
        }
    }

    /// Frames after an output frame that the grains covering it read, including the rest of
    /// their segment, whose quietest grain decides where the search restarts.
    fn grain_lookahead(&self) -> i64 {
        (SEGMENT + 2) * self.hop + self.search + self.reach + self.compare + 2 * self.step + 4
    }

    /// Frames before an output frame that the grains covering it depend on: back to the
    /// restart at most `FORCED_EVERY + 1` segments before, and what that grain reads.
    fn grain_memory(&self) -> i64 {
        ((FORCED_EVERY + 1) * SEGMENT + 3) * self.hop
            + self.search
            + self.reach
            + self.compare
            + 2 * self.step
            + 4
    }
}

/// Frames of input after a frame that the shifted output there depends on.
pub(super) fn lookahead(sample_rate: u32, ratio: f64, keep_formants: bool) -> usize {
    let g = Geometry::new(sample_rate, ratio);
    let formants = if keep_formants { g.frame } else { 0 };
    (g.grain_lookahead() + formants) as usize
}

/// Frames of input before a frame that the shifted output there depends on.
pub(super) fn warm_up(sample_rate: u32, ratio: f64, keep_formants: bool) -> usize {
    let g = Geometry::new(sample_rate, ratio);
    let formants = if keep_formants { g.frame } else { 0 };
    (g.grain_memory() + formants) as usize + seconds(FILTER_SETTLE_SECONDS, f64::from(sample_rate))
}

/// The input of one call, by stream position: silence outside the buffer.
struct Signal<'a> {
    start: i64,
    mono: &'a [f64],
    coarse: &'a [f64],
    coarse_first: i64,
}

impl Signal<'_> {
    fn mono(&self, at: i64) -> f64 {
        let i = at - self.start;
        if i < 0 {
            0.0
        } else {
            self.mono.get(i as usize).copied().unwrap_or(0.0)
        }
    }

    fn coarse(&self, at: i64) -> f64 {
        let i = at - self.coarse_first;
        if i < 0 {
            0.0
        } else {
            self.coarse.get(i as usize).copied().unwrap_or(0.0)
        }
    }

    /// Energy of the input under grain `k`.
    fn energy(&self, g: &Geometry, k: i64) -> f64 {
        let (from, to) = (
            (k * g.hop).div_euclid(g.step),
            (k * g.hop + 2 * g.hop - 1).div_euclid(g.step),
        );
        (from..=to).map(|u| self.coarse(u).powi(2)).sum()
    }

    /// The normalised correlation of `len` frames from `target` with `len` from
    /// `candidate`, read by `read` every `stride` positions.
    fn score(
        read: impl Fn(i64) -> f64,
        target: i64,
        candidate: i64,
        len: i64,
        stride: usize,
    ) -> f64 {
        let (mut dot, mut energy) = (0.0f64, 0.0f64);
        for j in (0..len).step_by(stride) {
            let b = read(candidate + j);
            dot += read(target + j) * b;
            energy += b * b;
        }
        dot / (energy + 1e-12).sqrt()
    }

    /// The move within `search` of `nominal` whose waveform best continues `natural`.
    /// Ties go to the smallest move, so silence keeps grains at their nominal places.
    fn best_shift(&self, g: &Geometry, ratio: f64, nominal: i64, natural: f64) -> i64 {
        // The compared stretch is centred on the middle of the overlap, half a hop before
        // the grain's centre (`r H / 2` in input frames).
        let half = (ratio * g.hop as f64 / 2.0).round() as i64;
        let target = (natural.round() as i64) - half - g.compare / 2;
        let base = nominal - half - g.compare / 2;
        let better = |score: f64, shift: i64, best: (f64, i64)| {
            score > best.0 || (score == best.0 && shift.abs() < best.1.abs())
        };
        let mut best = (f64::NEG_INFINITY, 0i64);
        if g.step > 1 {
            let len = g.compare / g.step;
            let (t, b) = (target.div_euclid(g.step), base.div_euclid(g.step));
            let reach = g.search / g.step;
            for shift in -reach..=reach {
                let score = Self::score(|u| self.coarse(u), t, b + shift, len, 1);
                if better(score, shift * g.step, best) {
                    best = (score, shift * g.step);
                }
            }
            let coarse = best.1;
            best = (f64::NEG_INFINITY, 0);
            let (lo, hi) = (
                (coarse - g.step).max(-g.search),
                (coarse + g.step).min(g.search),
            );
            for shift in lo..=hi {
                let score = Self::score(|a| self.mono(a), target, base + shift, g.compare, 2);
                if better(score, shift, best) {
                    best = (score, shift);
                }
            }
        } else {
            for shift in -g.search..=g.search {
                let score = Self::score(|a| self.mono(a), target, base + shift, g.compare, 1);
                if better(score, shift, best) {
                    best = (score, shift);
                }
            }
        }
        best.1
    }
}

/// Shifts the pitch of interleaved `samples` (first frame at stream frame `start`) by
/// `ratio`, keeping the spectral envelope when `keep_formants` is set.
pub(super) fn shift(
    samples: &mut [f32],
    channels: usize,
    sample_rate: u32,
    start: i64,
    ratio: f64,
    keep_formants: bool,
) {
    let rate = f64::from(sample_rate);
    let g = Geometry::new(sample_rate, ratio);
    let frames = (samples.len() / channels) as i64;
    let source: Vec<Vec<f64>> = (0..channels)
        .map(|c| deinterleave(samples, channels, c))
        .collect();
    let mut input = source.clone();
    if ratio > 1.0 {
        let cutoff = 0.45 * rate / ratio;
        let filter = Chain::new(vec![
            Biquad::low_pass(rate, cutoff, BUTTERWORTH_4[0]),
            Biquad::low_pass(rate, cutoff, BUTTERWORTH_4[1]),
        ]);
        for channel in &mut input {
            let mut filter = filter.clone();
            for v in channel.iter_mut() {
                *v = filter.run(*v);
            }
        }
    }
    let mono: Vec<f64> = (0..frames as usize)
        .map(|i| input.iter().map(|ch| ch[i]).sum::<f64>() / channels as f64)
        .collect();
    let coarse_first = start.div_euclid(g.step);
    let coarse_last = (start + frames - 1).div_euclid(g.step);
    let signal = Signal {
        start,
        mono: &mono,
        coarse: &[],
        coarse_first,
    };
    let coarse: Vec<f64> = (coarse_first..=coarse_last)
        .map(|u| (0..g.step).map(|j| signal.mono(u * g.step + j)).sum())
        .collect();
    let signal = Signal {
        coarse: &coarse,
        ..signal
    };

    let hop = g.hop;
    let hann: Vec<f64> = (0..2 * hop)
        .map(|m| 0.5 - 0.5 * (std::f64::consts::PI * m as f64 / hop as f64).cos())
        .collect();
    // Grains that overlap the buffer, and the restart the first of them follows.
    let first = (start - 2 * hop).div_euclid(hop) + 1;
    let last = (start + frames - 1).div_euclid(hop);
    let restart = |segment: i64| -> Option<i64> {
        let mut best = (f64::INFINITY, segment * SEGMENT);
        let mut total = 0.0;
        for k in segment * SEGMENT..(segment + 1) * SEGMENT {
            let energy = signal.energy(&g, k);
            total += energy;
            if energy < best.0 {
                best = (energy, k);
            }
        }
        let dip = best.0 <= DIP * total / SEGMENT as f64;
        (dip || segment.rem_euclid(FORCED_EVERY) == 0).then_some(best.1)
    };
    // The latest restart at or before the first grain; a forced one is at most
    // `FORCED_EVERY + 1` segments back.
    let mut segment = first.div_euclid(SEGMENT);
    let begin = loop {
        match restart(segment) {
            Some(k) if k <= first => break k,
            _ => segment -= 1,
        }
    };
    let mut restart_at = restart(begin.div_euclid(SEGMENT));
    segment = begin.div_euclid(SEGMENT);
    let mut out = vec![vec![0.0f64; frames as usize]; channels];
    let mut previous: Option<i64> = None;
    for k in begin..=last {
        if k.div_euclid(SEGMENT) != segment {
            segment = k.div_euclid(SEGMENT);
            restart_at = restart(segment);
        }
        let centre = k * hop + hop;
        let place = match previous {
            Some(p) if Some(k) != restart_at => {
                let natural = p as f64 + ratio * hop as f64;
                centre + signal.best_shift(&g, ratio, centre, natural)
            }
            _ => centre,
        };
        previous = Some(place);
        if k < first {
            continue;
        }
        let (lo, hi) = (
            (k * hop).max(start),
            (k * hop + 2 * hop).min(start + frames),
        );
        for o in lo..hi {
            let weight = hann[(o - k * hop) as usize];
            let at = place as f64 + ratio * (o - centre) as f64 - start as f64;
            let i = (o - start) as usize;
            for (channel, out) in input.iter().zip(out.iter_mut()) {
                out[i] += weight * cubic(channel, at);
            }
        }
    }
    if keep_formants {
        Formants::new(g.frame as usize, sample_rate).correct(&source, &mut out, start, ratio);
    }
    for (c, channel) in out.iter().enumerate() {
        for (i, v) in channel.iter().enumerate() {
            samples[i * channels + c] = sample(*v);
        }
    }
}

/// Frame-by-frame correction of a shifted signal's spectral envelope.
struct Formants {
    len: usize,
    hop: usize,
    rate: f64,
    window: Vec<f64>,
    lifter: Vec<f64>,
    forward: Arc<dyn Fft<f64>>,
    inverse: Arc<dyn Fft<f64>>,
}

impl Formants {
    fn new(len: usize, sample_rate: u32) -> Self {
        let rate = f64::from(sample_rate);
        let mut planner = FftPlanner::new();
        let window = (0..len)
            .map(|n| 0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / len as f64).cos())
            .collect();
        // Keep quefrencies below the cut-off, with a raised-cosine edge over its upper half.
        let cut = (LIFTER_SECONDS * rate).max(2.0);
        let lifter = (0..len)
            .map(|q| {
                let q = q.min(len - q) as f64;
                if q < cut / 2.0 {
                    1.0
                } else if q < cut {
                    0.5 + 0.5 * (std::f64::consts::PI * (q - cut / 2.0) / (cut / 2.0)).cos()
                } else {
                    0.0
                }
            })
            .collect();
        Formants {
            len,
            hop: len / 4,
            rate,
            window,
            lifter,
            forward: planner.plan_fft_forward(len),
            inverse: planner.plan_fft_inverse(len),
        }
    }

    /// Replaces `shifted` (channels of the same length as `source`, first frame at stream
    /// frame `start`) by its frames with the envelope of `source`.
    ///
    /// The shifted sound's envelope is the source's stretched by `ratio`, so the gain at
    /// frequency `f` is `E(f) / E(f / ratio)`, both read from the source's envelope `E`. That
    /// keeps the estimate's bias from the spacing of harmonics out of the ratio, since both
    /// ends of it see the same harmonics.
    fn correct(&self, source: &[Vec<f64>], shifted: &mut [Vec<f64>], start: i64, ratio: f64) {
        let (len, hop) = (self.len as i64, self.hop as i64);
        let frames = shifted.first().map_or(0, Vec::len) as i64;
        let channels = shifted.len();
        let at = |signal: &[f64], i: i64| {
            if i < 0 {
                0.0
            } else {
                signal.get(i as usize).copied().unwrap_or(0.0)
            }
        };
        // The inverse transform is unnormalised (a factor `len`) and the squared windows of
        // four frames sum to 3/2.
        let scale = 1.0 / (len as f64 * 1.5);
        let eps = 1e-12 * len as f64;
        let n = self.len;
        let half = n / 2;
        let limit = FORMANT_LIMIT.ln();
        let mut z = vec![Complex::default(); n];
        let mut envelope = vec![0.0f64; half + 1];
        let mut gains = vec![0.0f64; n];
        let mut spectra = vec![vec![Complex::default(); n]; channels.div_ceil(2)];
        let mut out = vec![vec![0.0f64; frames as usize]; channels];
        let mono: Vec<f64> = (0..frames as usize)
            .map(|i| source.iter().map(|s| s[i]).sum::<f64>() / channels as f64)
            .collect();
        let first = (start - len).div_euclid(hop) + 1;
        let last = (start + frames - 1).div_euclid(hop);
        for j in first..=last {
            let a = j * hop - start;
            for (m, slot) in z.iter_mut().enumerate() {
                *slot = Complex::new(at(&mono, a + m as i64) * self.window[m], 0.0);
            }
            self.forward.process(&mut z);
            // The log magnitude, liftered in the cepstrum: the smoothed log envelope.
            for slot in z.iter_mut() {
                *slot = Complex::new((slot.norm() + eps).ln(), 0.0);
            }
            self.inverse.process(&mut z);
            for (v, l) in z.iter_mut().zip(&self.lifter) {
                *v *= *l / n as f64;
            }
            self.forward.process(&mut z);
            for (e, v) in envelope.iter_mut().zip(&z) {
                *e = v.re;
            }
            let read = |bin: f64| {
                let bin = bin.min(half as f64);
                let low = bin.floor() as usize;
                let high = (low + 1).min(half);
                let t = bin - low as f64;
                envelope[low] + t * (envelope[high] - envelope[low])
            };
            for (k, g) in gains.iter_mut().enumerate() {
                let bin = k.min(n - k);
                let difference = (envelope[bin] - read(bin as f64 / ratio)).clamp(-limit, limit);
                let mut gain = difference.exp();
                // Lowered, the shifted sound has nothing above its new top: do not lift noise.
                let hz = bin as f64 * self.rate / n as f64;
                if ratio < 1.0 && hz > 0.45 * ratio * self.rate {
                    gain = gain.min(1.0);
                }
                *g = gain;
            }
            // The channels in pairs, one packed transform each. The gains change the
            // balance, not the loudness: the frame keeps the energy it had.
            let (mut before, mut after) = (0.0f64, 0.0f64);
            for (p, spectrum) in spectra.iter_mut().enumerate() {
                let (pair, other) = (2 * p, 2 * p + 1);
                for (m, slot) in spectrum.iter_mut().enumerate() {
                    let w = self.window[m];
                    let i = a + m as i64;
                    let im = if other < channels {
                        at(&shifted[other], i)
                    } else {
                        0.0
                    };
                    *slot = Complex::new(at(&shifted[pair], i) * w, im * w);
                }
                self.forward.process(spectrum);
                for (v, g) in spectrum.iter().zip(&gains) {
                    before += v.norm_sqr();
                    after += g * g * v.norm_sqr();
                }
            }
            let level = if after > 0.0 {
                (before / after).sqrt()
            } else {
                1.0
            };
            for (p, spectrum) in spectra.iter_mut().enumerate() {
                let (pair, other) = (2 * p, 2 * p + 1);
                for (v, g) in spectrum.iter_mut().zip(&gains) {
                    *v *= *g * level;
                }
                self.inverse.process(spectrum);
                for (m, v) in spectrum.iter().enumerate() {
                    let i = a + m as i64;
                    if i < 0 || i >= frames {
                        continue;
                    }
                    let w = self.window[m] * scale;
                    out[pair][i as usize] += v.re * w;
                    if other < channels {
                        out[other][i as usize] += v.im * w;
                    }
                }
            }
        }
        for (s, o) in shifted.iter_mut().zip(out) {
            *s = o;
        }
    }
}
