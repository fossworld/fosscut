//! Integrated loudness after ITU-R BS.1770-4 (public specification): K-weighting, mean
//! square over 400 ms blocks overlapping by 75%, an absolute gate at -70 LUFS and a
//! relative gate 10 LU below the absolutely gated loudness. Input is 48 kHz.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The sample rate the meter expects.
pub const RATE: u32 = 48_000;
/// Samples per 100 ms step: a quarter of a 400 ms block.
const STEP: usize = (RATE / 10) as usize;

/// Loudness of a stream, to 0.01 dB.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Loudness {
    /// Integrated loudness in LUFS; absent when the stream is silent or shorter than 400 ms.
    pub integrated_lufs: Option<f64>,
    /// Largest sample magnitude in dBFS; absent for an empty or all-zero stream.
    pub sample_peak_dbfs: Option<f64>,
}

/// `value` to the nearest 0.01.
fn hundredths(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// A second-order filter section, transposed direct form II.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    const fn new(b: [f64; 3], a: [f64; 2]) -> Biquad {
        Biquad { b, a, z: [0.0; 2] }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// The K-weighting filter at 48 kHz: the high-shelf pre-filter, then the RLB high-pass, with
/// the coefficients given in BS.1770-4.
fn k_weighting() -> [Biquad; 2] {
    [
        Biquad::new(
            [
                1.535_124_859_586_97,
                -2.691_696_189_406_38,
                1.198_392_810_852_85,
            ],
            [-1.690_659_293_182_41, 0.732_480_774_215_85],
        ),
        Biquad::new(
            [1.0, -2.0, 1.0],
            [-1.990_047_454_833_98, 0.990_072_250_366_21],
        ),
    ]
}

/// Streams interleaved samples and reports their loudness.
#[derive(Clone, Debug)]
pub struct LoudnessMeter {
    channels: usize,
    filters: Vec<[Biquad; 2]>,
    /// Sum over channels of the K-weighted mean square of each completed 100 ms step.
    steps: Vec<f64>,
    acc: f64,
    count: usize,
    peak: f32,
}

impl LoudnessMeter {
    /// A meter for `channels` interleaved channels at 48 kHz. Every channel has weight 1,
    /// as BS.1770 gives left, right and centre.
    pub fn new(channels: u16) -> LoudnessMeter {
        let channels = usize::from(channels.max(1));
        LoudnessMeter {
            channels,
            filters: vec![k_weighting(); channels],
            steps: Vec::new(),
            acc: 0.0,
            count: 0,
            peak: 0.0,
        }
    }

    /// Adds interleaved samples; a trailing partial frame is ignored.
    pub fn push(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channels) {
            for (channel, &sample) in frame.iter().enumerate() {
                self.peak = self.peak.max(sample.abs());
                let filters = &mut self.filters[channel];
                let shelved = filters[0].process(f64::from(sample));
                let weighted = filters[1].process(shelved);
                self.acc += weighted * weighted;
            }
            self.count += 1;
            if self.count == STEP {
                self.steps.push(self.acc / STEP as f64);
                self.acc = 0.0;
                self.count = 0;
            }
        }
    }

    /// The loudness of everything pushed.
    pub fn finish(self) -> Loudness {
        let loudness = |power: f64| -0.691 + 10.0 * power.log10();
        // 400 ms blocks: four consecutive 100 ms steps.
        let blocks: Vec<f64> = self
            .steps
            .windows(4)
            .map(|w| w.iter().sum::<f64>() / 4.0)
            .collect();
        let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len() as f64;
        let absolute: Vec<f64> = blocks
            .into_iter()
            .filter(|&p| p > 0.0 && loudness(p) > -70.0)
            .collect();
        let integrated_lufs = (!absolute.is_empty()).then(|| {
            let gate = loudness(mean(&absolute)) - 10.0;
            let relative: Vec<f64> = absolute
                .iter()
                .copied()
                .filter(|&p| loudness(p) > gate)
                .collect();
            hundredths(loudness(mean(&relative)))
        });
        Loudness {
            integrated_lufs,
            sample_peak_dbfs: (self.peak > 0.0)
                .then(|| hundredths(20.0 * f64::from(self.peak).log10())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amplitude: f32, seconds: f64, channels: u16) -> Vec<f32> {
        let n = (seconds * f64::from(RATE)) as usize;
        (0..n)
            .flat_map(|i| {
                let t = i as f64 / f64::from(RATE);
                let s = amplitude * (2.0 * std::f64::consts::PI * 997.0 * t).sin() as f32;
                std::iter::repeat_n(s, usize::from(channels))
            })
            .collect()
    }

    fn measure(samples: &[f32], channels: u16) -> Loudness {
        let mut meter = LoudnessMeter::new(channels);
        // Uneven chunks: the meter must not depend on how samples arrive.
        for chunk in samples.chunks(7_001 * usize::from(channels)) {
            meter.push(chunk);
        }
        meter.finish()
    }

    #[test]
    fn a_full_scale_sine_reads_minus_three_lufs() {
        // BS.1770 calibration: a 997 Hz sine at 0 dBFS in one channel reads -3.01 LUFS.
        let full = measure(&sine(1.0, 5.0, 1), 1);
        assert!(
            (full.integrated_lufs.unwrap() + 3.01).abs() < 0.05,
            "{full:?}"
        );
        let quiet = measure(&sine(0.1, 5.0, 1), 1);
        assert!(
            (quiet.integrated_lufs.unwrap() + 23.01).abs() < 0.05,
            "{quiet:?}"
        );
        assert!((quiet.sample_peak_dbfs.unwrap() + 20.0).abs() < 0.01);
    }

    #[test]
    fn channels_add_power() {
        let stereo = measure(&sine(0.1, 5.0, 2), 2);
        assert!(
            (stereo.integrated_lufs.unwrap() + 20.0).abs() < 0.05,
            "{stereo:?}"
        );
    }

    #[test]
    fn gates_ignore_silence_and_quiet_passages() {
        let mut samples = sine(0.1, 4.0, 1);
        samples.extend(std::iter::repeat_n(0.0, 4 * RATE as usize));
        samples.extend(sine(0.001, 4.0, 1));
        // The silence falls below the absolute gate and the -60 dBFS passage below the
        // relative gate (it would pull the result to about -26 LUFS otherwise). The tone's 37
        // whole blocks count fully and the three blocks straddling its end at 3/4, 1/2 and
        // 1/4 of its power: -23.01 + 10 log10(38.5 / 40) = -23.18 LUFS.
        let gated = measure(&samples, 1);
        let expected = -23.01 + 10.0 * (38.5f64 / 40.0).log10();
        assert!(
            (gated.integrated_lufs.unwrap() - expected).abs() < 0.02,
            "{gated:?}"
        );

        let silent = measure(&vec![0.0; RATE as usize], 1);
        assert_eq!(
            silent,
            Loudness {
                integrated_lufs: None,
                sample_peak_dbfs: None
            }
        );
        let short = measure(&sine(0.5, 0.3, 1), 1);
        assert!(short.integrated_lufs.is_none(), "shorter than one block");
        assert!(short.sample_peak_dbfs.is_some());
    }
}
