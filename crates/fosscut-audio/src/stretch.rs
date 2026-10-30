//! The sound of clips with speed (ADR-0023 item 9): resampled when the pitch follows the
//! speed, time-stretched with WSOLA (waveform-similarity overlap-add) when it is kept.
//!
//! WSOLA cuts the source into overlapping windows placed where the speed map says, and
//! nudges each window within a small tolerance to where its waveform best continues the
//! previous one, so the overlap-add keeps the pitch (Verhelst and Roelands, 1993). Windows
//! sit on a fixed grid of the clip's timeline, and the search restarts at fixed anchors, so
//! any stretch of the output comes out the same however the mix is cut into windows.

use fosscut_media::AudioBuffer;
use fosscut_model::{Clip, Id, Pitch};
use fosscut_time::{FLICKS_PER_SECOND, Flicks};

use crate::{AudioError, AudioSource};

/// Windows between search anchors.
const ANCHOR_EVERY: i64 = 64;

/// The clip's sound for `count` sample frames from sample `first` after its start (which
/// may be negative, for transitions), at `sample_rate` with `channels` channels,
/// interleaved. The clip must have a speed; frozen clips are silent.
pub(crate) fn clip_audio(
    source: &mut dyn AudioSource,
    asset: &Id,
    clip: &Clip,
    first: i64,
    count: usize,
    sample_rate: u32,
    channels: u16,
) -> Result<Vec<f32>, AudioError> {
    let channels_n = usize::from(channels);
    let mut out = vec![0.0f32; count * channels_n];
    let Some(speed) = &clip.speed else {
        return Ok(out);
    };
    if speed.is_frozen() || count == 0 {
        return Ok(out);
    }
    let rate = f64::from(sample_rate);
    // The source sample at clip sample `j`, through the speed map.
    let position = |j: f64| -> f64 {
        let offset = Flicks::new((j / rate * FLICKS_PER_SECOND as f64).round() as i64);
        clip.source_position(offset).unwrap_or(0.0) / FLICKS_PER_SECOND as f64 * rate
    };
    match speed.pitch {
        Pitch::Shift => {
            let positions: Vec<f64> = (0..count)
                .map(|i| position((first + i as i64) as f64))
                .collect();
            let span = Span::read(source, asset, &positions, 2, sample_rate, channels)?;
            for (i, p) in positions.iter().enumerate() {
                for c in 0..channels_n {
                    out[i * channels_n + c] = span.at(*p, c);
                }
            }
        }
        Pitch::Preserve => {
            let backwards = speed.reverse;
            let window = window_length(sample_rate);
            let hop = window / 2;
            let tolerance = window / 4;
            let last = first + count as i64;
            let (k_first, k_last) = (first.div_euclid(hop) - 1, (last - 1).div_euclid(hop));
            let anchor = k_first.div_euclid(ANCHOR_EVERY) * ANCHOR_EVERY;
            let centres: Vec<f64> = (anchor..=k_last)
                .map(|k| position((k * hop + window / 2) as f64))
                .collect();
            let reach = (window + hop + tolerance) as usize;
            let span = Span::read(source, asset, &centres, reach, sample_rate, channels)?;
            let direction = if backwards { -1 } else { 1 };
            let hann: Vec<f32> = (0..window)
                .map(|n| {
                    let x = std::f64::consts::TAU * n as f64 / window as f64;
                    (0.5 - 0.5 * x.cos()) as f32
                })
                .collect();
            // The sample a window starting at source sample `start` plays at its `n`th
            // place: forwards, or backwards from its far end.
            let segment_start = |centre: f64| centre.round() as i64 - direction * (window / 2);
            let mut previous: Option<i64> = None;
            for (index, k) in (anchor..=k_last).enumerate() {
                let nominal = segment_start(centres[index]);
                let shift = match previous {
                    // The natural continuation of the previous window, one hop on.
                    Some(start) if (k - anchor) % ANCHOR_EVERY != 0 => {
                        let natural = start + direction * hop;
                        best_shift(&span, nominal, natural, direction, hop, tolerance)
                    }
                    _ => 0,
                };
                let start = nominal + shift;
                previous = Some(start);
                if k < k_first {
                    continue;
                }
                for n in 0..window {
                    let j = k * hop + n;
                    if j < first || j >= last {
                        continue;
                    }
                    let i = (j - first) as usize;
                    let at = start + direction * n;
                    let weight = hann[n as usize];
                    for c in 0..channels_n {
                        out[i * channels_n + c] += weight * span.sample(at, c);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Samples per WSOLA window: 20 milliseconds, an even number.
fn window_length(sample_rate: u32) -> i64 {
    (i64::from(sample_rate) / 100).max(8) * 2
}

/// How far to move the window starting at `nominal` so its first half best matches the
/// samples from `natural` on, within `tolerance`.
fn best_shift(
    span: &Span,
    nominal: i64,
    natural: i64,
    direction: i64,
    overlap: i64,
    tolerance: i64,
) -> i64 {
    let mut best: (f32, i64) = (f32::MIN, 0);
    for shift in -tolerance..=tolerance {
        let (mut dot, mut energy) = (0.0f32, 0.0f32);
        for n in (0..overlap).step_by(4) {
            let a = span.mono(nominal + shift + direction * n);
            let b = span.mono(natural + direction * n);
            dot += a * b;
            energy += a * a;
        }
        // Normalized by the candidate's energy, so the continuation itself scores highest
        // (Cauchy and Schwarz) and louder stretches do not win by being louder.
        let score = dot / (energy + 1e-9).sqrt();
        // Ties go to the smallest move, so unchanged audio stays where it is.
        if score > best.0 || (score == best.0 && shift.abs() < best.1.abs()) {
            best = (score, shift);
        }
    }
    best.1
}

/// A stretch of a source's sound around the positions a clip plays.
struct Span {
    /// The source sample of `samples[0]`.
    first: i64,
    frames: usize,
    channels: usize,
    samples: Vec<f32>,
}

impl Span {
    /// Reads the source from the lowest to the highest of `positions`, `reach` samples
    /// more each way.
    fn read(
        source: &mut dyn AudioSource,
        asset: &Id,
        positions: &[f64],
        reach: usize,
        sample_rate: u32,
        channels: u16,
    ) -> Result<Span, AudioError> {
        let low = positions.iter().copied().fold(f64::INFINITY, f64::min);
        let high = positions.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let first = low.floor() as i64 - reach as i64;
        let last = high.ceil() as i64 + reach as i64 + 1;
        let to_flicks = |sample: i64| {
            Flicks::new(
                (i128::from(sample) * i128::from(FLICKS_PER_SECOND) / i128::from(sample_rate))
                    as i64,
            )
        };
        let buffer: AudioBuffer = source.read(
            asset,
            to_flicks(first),
            to_flicks(last) - to_flicks(first),
            sample_rate,
            channels,
        )?;
        Ok(Span {
            first,
            frames: buffer.frames(),
            channels: usize::from(channels),
            samples: buffer.samples,
        })
    }

    /// Source sample `at` of `channel`; silence outside the span.
    fn sample(&self, at: i64, channel: usize) -> f32 {
        let i = at - self.first;
        if i < 0 || i as usize >= self.frames {
            return 0.0;
        }
        self.samples[i as usize * self.channels + channel]
    }

    /// The channels of source sample `at` summed, for comparing waveforms.
    fn mono(&self, at: i64) -> f32 {
        (0..self.channels).map(|c| self.sample(at, c)).sum()
    }

    /// The source at fractional sample `at`, interpolated linearly.
    fn at(&self, at: f64, channel: usize) -> f32 {
        let low = at.floor();
        let t = (at - low) as f32;
        let low = low as i64;
        self.sample(low, channel) * (1.0 - t) + self.sample(low + 1, channel) * t
    }
}
