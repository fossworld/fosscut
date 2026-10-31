//! Aligning recordings of the same scene by their sound.
//!
//! [`audio_offset`] compares onset envelopes rather than waveforms, so that different
//! microphones, distances, gains and background noise still agree: each mono signal's level
//! is measured in blocks of about a millisecond (squared and averaged, a low-pass filter),
//! compressed with a logarithm above a floor at its median level, and differenced with
//! negative changes dropped, which leaves the sound's onsets (an energy-based novelty
//! function, as in onset detection: Bello, Daudet, Abdallah, Duxbury, Davies and Sandler, "A
//! tutorial on onset detection in music signals", IEEE TSAP 2005). The two envelopes,
//! normalised to zero mean and unit variance, are cross-correlated with an FFT; the highest
//! peak within the allowed offsets gives the offset to the block, a parabola through its
//! neighbours a finer one, and the raw signals' cross-correlation around it, over stretches
//! at the strongest onsets, the offset to the sample (time-delay estimation by
//! cross-correlation: Knapp and Carter, "The generalized correlation method for estimation
//! of time delay", IEEE TASSP 1976).
//!
//! The confidence combines how far the peak stands above the best peak elsewhere with how
//! strongly the envelopes agree at it.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Envelope blocks per second, roughly.
const ENVELOPE_RATE: f64 = 1000.0;
/// Peaks closer than this to the highest, in seconds, belong to it.
const PEAK_WIDTH: f64 = 0.025;
/// The envelopes' normalised correlation at which their agreement counts in full.
const FULL_AGREEMENT: f64 = 0.5;
/// Length of each stretch of the raw signals compared, in seconds.
const STRETCH: f64 = 0.25;
/// Stretches compared, at the strongest onsets.
const STRETCHES: usize = 8;
/// The raw signals' normalised correlation below which the block estimate stands.
const MIN_RAW_AGREEMENT: f64 = 0.1;

/// How far `other` lags `reference`, and how sure the match is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SyncResult {
    /// How far `other` lags `reference`, in seconds. Positive: `other` started later in the
    /// scene, so it must be placed later; its sound at time `t` is the reference's at
    /// `t + offset_seconds`.
    pub offset_seconds: f64,
    /// How sure the match is, from 0 (no agreement, or nothing to compare) to 1: the geometric
    /// mean of how far the correlation peak stands above the best one elsewhere
    /// (1 - second / first) and how strongly the envelopes agree at it (their normalised
    /// correlation, in full from 0.5).
    pub confidence: f64,
}

/// How far `other` lags `reference` (positive: `other` starts later in the scene, so it must
/// be placed later), in seconds, and how sure the match is, from 0 to 1. Both are mono at
/// `sample_rate`; offsets beyond `max_offset_seconds` either way are not considered (0 or
/// less, or not finite, allows any offset).
pub fn audio_offset(
    reference: &[f32],
    other: &[f32],
    sample_rate: u32,
    max_offset_seconds: f64,
) -> SyncResult {
    let unknown = SyncResult {
        offset_seconds: 0.0,
        confidence: 0.0,
    };
    if sample_rate == 0 {
        return unknown;
    }
    let rate = f64::from(sample_rate);
    let block = ((rate / ENVELOPE_RATE).round() as usize).max(1);
    let blocks_per_second = rate / block as f64;
    let (Some(ours), Some(theirs)) = (onsets(reference, block), onsets(other, block)) else {
        return unknown;
    };
    let limit = if max_offset_seconds.is_finite() && max_offset_seconds > 0.0 {
        (max_offset_seconds * blocks_per_second).ceil() as i64
    } else {
        i64::MAX
    };
    let (ours_len, theirs_len) = (ours.len() as i64, theirs.len() as i64);
    let lowest = (1 - theirs_len).max(-limit);
    let highest = (ours_len - 1).min(limit);
    if lowest > highest {
        return unknown;
    }

    // correlation[lag] = sum over k of ours[k + lag] * theirs[k].
    let correlation = cross_correlation(&ours, &theirs);
    let size = correlation.len() as i64;
    let at = |lag: i64| f64::from(correlation[lag.rem_euclid(size) as usize]);
    let (mut lag, mut peak) = (lowest, f64::NEG_INFINITY);
    for candidate in lowest..=highest {
        let value = at(candidate);
        if value > peak {
            (lag, peak) = (candidate, value);
        }
    }
    if !(peak > 0.0 && peak.is_finite()) {
        return unknown;
    }
    let width = (PEAK_WIDTH * blocks_per_second).ceil() as i64;
    let second = (lowest..=highest)
        .filter(|&candidate| (candidate - lag).abs() > width)
        .map(at)
        .fold(0.0, f64::max);
    let stands_out = (1.0 - second / peak).clamp(0.0, 1.0);
    // The envelopes' energy where they overlap at this lag.
    let energy = |values: &[f32], from: i64, to: i64| -> f64 {
        let from = from.clamp(0, values.len() as i64);
        values[from as usize..to.clamp(from, values.len() as i64) as usize]
            .iter()
            .map(|&v| f64::from(v) * f64::from(v))
            .sum()
    };
    let overlap = energy(&ours, lag, theirs_len + lag) * energy(&theirs, -lag, ours_len - lag);
    let agreement = if overlap > 0.0 {
        (peak / overlap.sqrt() / FULL_AGREEMENT).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let confidence = (stands_out * agreement).sqrt();

    let fraction = if lag > lowest && lag < highest {
        parabola_peak(at(lag - 1), peak, at(lag + 1))
    } else {
        0.0
    };
    let coarse = (lag as f64 + fraction) * block as f64;
    let samples = refine(reference, other, &ours, coarse, block, rate).unwrap_or(coarse);
    SyncResult {
        offset_seconds: samples / rate,
        confidence,
    }
}

/// The onset envelope of `samples` in blocks of `block` samples, normalised to zero mean and
/// unit variance; `None` when it is shorter than two blocks or has no onsets.
fn onsets(samples: &[f32], block: usize) -> Option<Vec<f32>> {
    let levels: Vec<f32> = samples
        .chunks_exact(block)
        .map(|chunk| {
            let power: f32 = chunk
                .iter()
                .map(|&s| if s.is_finite() { s * s } else { 0.0 })
                .sum();
            (power / block as f32).sqrt()
        })
        .collect();
    if levels.len() < 2 {
        return None;
    }
    let mut sorted = levels.clone();
    let middle = sorted.len() / 2;
    let (_, median, _) = sorted.select_nth_unstable_by(middle, f32::total_cmp);
    let floor = (*median).max(1e-6);
    let mut envelope = Vec::with_capacity(levels.len());
    envelope.push(0.0f32);
    envelope.extend(
        levels
            .windows(2)
            .map(|pair| ((pair[1] + floor).ln() - (pair[0] + floor).ln()).max(0.0)),
    );
    let n = envelope.len() as f64;
    let mean = envelope.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
    let variance = envelope
        .iter()
        .map(|&v| (f64::from(v) - mean).powi(2))
        .sum::<f64>()
        / n;
    if variance.is_nan() || variance <= 1e-12 {
        return None;
    }
    let deviation = variance.sqrt();
    Some(
        envelope
            .into_iter()
            .map(|v| ((f64::from(v) - mean) / deviation) as f32)
            .collect(),
    )
}

/// The cross-correlation of `a` with `b` at every lag, by FFT: entry `lag` (negative lags
/// from the end) is the sum over `k` of `a[k + lag] * b[k]`.
fn cross_correlation(a: &[f32], b: &[f32]) -> Vec<f32> {
    let size = (a.len() + b.len()).next_power_of_two();
    let mut planner = FftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(size);
    let inverse = planner.plan_fft_inverse(size);
    let spectrum = |values: &[f32]| {
        let mut buffer: Vec<Complex32> = values.iter().map(|&v| Complex32::new(v, 0.0)).collect();
        buffer.resize(size, Complex32::new(0.0, 0.0));
        forward.process(&mut buffer);
        buffer
    };
    let mut product = spectrum(a);
    for (x, y) in product.iter_mut().zip(spectrum(b)) {
        *x *= y.conj();
    }
    inverse.process(&mut product);
    let scale = 1.0 / size as f32;
    product.into_iter().map(|c| c.re * scale).collect()
}

/// The offset in samples, near `coarse`, at which the raw signals correlate best, over
/// stretches at the reference's strongest onsets (`onsets`, in blocks of `block` samples);
/// `None` when they hardly agree or do not overlap.
fn refine(
    reference: &[f32],
    other: &[f32],
    onsets: &[f32],
    coarse: f64,
    block: usize,
    rate: f64,
) -> Option<f64> {
    let centre = coarse.round() as i64;
    let (first, last) = (centre - block as i64, centre + block as i64);
    // Reference positions whose counterpart exists at every lag tried.
    let start = last.max(0);
    let end = (other.len() as i64 + first).min(reference.len() as i64);
    let stretch = ((STRETCH * rate) as i64).max(1);
    if end - start < stretch {
        return None;
    }
    let mut stretches: Vec<(i64, i64)> = Vec::new();
    if end - start <= stretch * STRETCHES as i64 {
        stretches.push((start, end));
    } else {
        let mut strongest: Vec<(f32, i64)> = onsets
            .iter()
            .enumerate()
            .map(|(k, &v)| (v, (k * block + block / 2) as i64))
            .filter(|&(_, at)| at - stretch / 2 >= start && at + stretch / 2 <= end)
            .collect();
        let keep = strongest.len().min(STRETCHES * 64);
        if keep < strongest.len() {
            strongest.select_nth_unstable_by(keep, |a, b| b.0.total_cmp(&a.0));
            strongest.truncate(keep);
        }
        strongest.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, at) in strongest {
            let candidate = (at - stretch / 2, at + stretch / 2);
            if stretches
                .iter()
                .all(|&(a, b)| candidate.1 <= a || candidate.0 >= b)
            {
                stretches.push(candidate);
                if stretches.len() == STRETCHES {
                    break;
                }
            }
        }
    }
    let lags = (last - first + 1) as usize;
    let mut sums = vec![0.0f64; lags];
    for &(a, b) in &stretches {
        let ours = &reference[a as usize..b as usize];
        for (sum, lag) in sums.iter_mut().zip(first..=last) {
            let theirs = &other[(a - lag) as usize..(b - lag) as usize];
            let products: f32 = ours.iter().zip(theirs).map(|(&x, &y)| x * y).sum();
            *sum += f64::from(products);
        }
    }
    let (index, best) = sums
        .iter()
        .copied()
        .enumerate()
        .fold((0, f64::NEG_INFINITY), |b, c| if c.1 > b.1 { c } else { b });
    if !best.is_finite() || best <= 0.0 {
        return None;
    }
    let lag = first + index as i64;
    let power = |values: &[f32]| values.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>();
    let (mut ours_power, mut theirs_power) = (0.0, 0.0);
    for &(a, b) in &stretches {
        ours_power += power(&reference[a as usize..b as usize]);
        theirs_power += power(&other[(a - lag) as usize..(b - lag) as usize]);
    }
    if best / (ours_power * theirs_power).sqrt() < MIN_RAW_AGREEMENT {
        return None;
    }
    let fraction = if index > 0 && index + 1 < lags {
        parabola_peak(sums[index - 1], best, sums[index + 1])
    } else {
        0.0
    };
    Some(lag as f64 + fraction)
}

/// Where a parabola through `(-1, before)`, `(0, at)` and `(1, after)` peaks, within half a
/// step of 0; 0 when it does not open downwards.
fn parabola_peak(before: f64, at: f64, after: f64) -> f64 {
    let curvature = before - 2.0 * at + after;
    if curvature < 0.0 && curvature.is_finite() {
        (0.5 * (before - after) / curvature).clamp(-0.5, 0.5)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::gray::testing::Random;

    /// `seconds` of quiet noise with bursts of louder noise at random times.
    fn bursts(seconds: f64, rate: u32, seed: u64) -> Vec<f32> {
        let mut random = Random(seed);
        let n = (seconds * f64::from(rate)) as usize;
        let mut samples: Vec<f32> = (0..n).map(|_| (0.01 * random.normal()) as f32).collect();
        let mut t = 0.05 + 0.3 * random.unit();
        while t < seconds {
            let length = 0.02 + 0.1 * random.unit();
            let amplitude = 0.2 + 0.6 * random.unit();
            let start = (t * f64::from(rate)) as usize;
            let count = (length * f64::from(rate)) as usize;
            for (i, sample) in samples.iter_mut().skip(start).take(count).enumerate() {
                let decay = (-3.0 * i as f64 / count as f64).exp();
                *sample += (amplitude * decay * random.normal()) as f32;
            }
            t += length + 0.1 + 0.5 * random.unit();
        }
        samples
    }

    /// `length` samples of `source` from `start` (silence before its beginning), scaled by
    /// `gain`, with noise of deviation `noise` added.
    fn recording(source: &[f32], start: i64, length: usize, gain: f32, noise: f64) -> Vec<f32> {
        let mut random = Random(start as u64 ^ 0xABCD);
        (0..length as i64)
            .map(|t| {
                let index = start + t;
                let clean = if index >= 0 && (index as usize) < source.len() {
                    source[index as usize]
                } else {
                    0.0
                };
                gain * clean + (noise * random.normal()) as f32
            })
            .collect()
    }

    #[test]
    fn a_later_recording_is_placed_later() {
        let rate = 48_000;
        let scene = bursts(20.0, rate, 1);
        // The second camera started 1.234 s after the first: its sound at t is the scene's
        // at t + 1.234 s. Quieter, with more noise.
        let delay = 59_232; // 1.234 s
        let other = recording(&scene, delay, 15 * rate as usize, 0.5, 0.05);
        let result = audio_offset(&scene, &other, rate, 10.0);
        println!("{result:?}");
        assert!((result.offset_seconds - 1.234).abs() < 0.002, "{result:?}");
        assert!(result.confidence > 0.7, "{result:?}");
    }

    #[test]
    fn an_earlier_recording_is_placed_earlier() {
        let rate = 48_000;
        let scene = bursts(20.0, rate, 2);
        // The second camera started 1.234 s before the first.
        let other = recording(&scene, -59_232, 18 * rate as usize, 2.0, 0.05);
        let result = audio_offset(&scene, &other, rate, 0.0);
        println!("{result:?}");
        assert!((result.offset_seconds + 1.234).abs() < 0.002, "{result:?}");
        assert!(result.confidence > 0.7, "{result:?}");
    }

    #[test]
    fn offsets_land_on_the_sample_at_other_rates() {
        // 16 kHz blocks are 16 samples; 8219 samples is not a whole number of blocks.
        let rate = 16_000;
        let scene = bursts(12.0, rate, 3);
        let other = recording(&scene, -8_219, 10 * rate as usize, 0.8, 0.02);
        let result = audio_offset(&scene, &other, rate, 5.0);
        println!("{result:?}");
        assert!(
            (result.offset_seconds * 16_000.0 + 8_219.0).abs() < 1.0,
            "{result:?}"
        );
        assert!(result.confidence > 0.7, "{result:?}");
    }

    #[test]
    fn unrelated_recordings_have_low_confidence() {
        let rate = 48_000;
        let result = audio_offset(&bursts(20.0, rate, 4), &bursts(15.0, rate, 5), rate, 0.0);
        println!("{result:?}");
        assert!(result.confidence < 0.3, "{result:?}");
        // The true offset of 3 s lies outside a window of 1 s.
        let scene = bursts(20.0, rate, 6);
        let other = recording(&scene, 3 * i64::from(rate), 12 * rate as usize, 1.0, 0.01);
        let result = audio_offset(&scene, &other, rate, 1.0);
        println!("{result:?}");
        assert!(result.offset_seconds.abs() <= 1.01, "{result:?}");
        assert!(result.confidence < 0.3, "{result:?}");
    }

    #[test]
    fn nothing_to_compare_has_no_confidence() {
        let unknown = SyncResult {
            offset_seconds: 0.0,
            confidence: 0.0,
        };
        let scene = bursts(2.0, 8_000, 7);
        assert_eq!(audio_offset(&scene, &[], 8_000, 1.0), unknown);
        assert_eq!(
            audio_offset(&scene, &vec![0.0; 16_000], 8_000, 1.0),
            unknown
        );
        assert_eq!(audio_offset(&scene, &scene, 0, 1.0), unknown);
        let mut broken = scene.clone();
        broken[100] = f32::NAN;
        let result = audio_offset(&scene, &broken, 8_000, 1.0);
        assert!(
            result.offset_seconds.abs() < 1e-3 && result.confidence > 0.9,
            "{result:?}"
        );
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(serde_json::from_value::<SyncResult>(json).unwrap(), result);
    }

    #[test]
    #[ignore = "timing means something only in an optimized build: run with --release -- --ignored --nocapture"]
    fn sync_time_for_ten_minutes() {
        let rate = 48_000;
        let scene = bursts(600.0, rate, 8);
        let other = recording(&scene, 12 * i64::from(rate), 500 * rate as usize, 0.7, 0.02);
        let start = Instant::now();
        let result = audio_offset(&scene, &other, rate, 0.0);
        let took = start.elapsed().as_secs_f64() * 1000.0;
        println!("sync of 10 and 8.3 minutes at 48 kHz: {took:.0} ms, {result:?}");
        assert!((result.offset_seconds - 12.0).abs() < 0.002);
    }
}
