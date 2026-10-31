//! Loudness over time (the `levels` analyzer): the K-weighted loudness of consecutive
//! windows of the sound, in LUFS, for finding where it peaks and how much it varies.
//!
//! Each channel is K-weighted with the two filters of ITU-R BS.1770-4 (the high-shelf
//! pre-filter and the RLB high-pass, with the coefficients the standard gives for 48 kHz),
//! the channels' mean squares over a window are summed with weight 1, and the window reads
//! `-0.691 + 10 log10(sum)`: the momentary loudness of BS.1770 over the window's length,
//! without gating. Windows quieter than -70 LUFS, the standard's absolute gate, read as
//! silent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The sample rate the meter expects.
pub const RATE: u32 = 48_000;
/// Windows below this read as silent, in LUFS.
const GATE: f64 = -70.0;

/// Options of the `levels` analyzer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LevelOptions {
    /// The window, in milliseconds, from 100 to 10000 (default 1000).
    pub window_ms: u32,
}

impl Default for LevelOptions {
    fn default() -> LevelOptions {
        LevelOptions { window_ms: 1000 }
    }
}

impl LevelOptions {
    /// The options as they are used, inside their ranges.
    pub fn clamped(&self) -> LevelOptions {
        LevelOptions {
            window_ms: self.window_ms.clamp(100, 10_000),
        }
    }
}

/// Loudness over time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Levels {
    /// The window length in milliseconds; window `i` starts at `i * window_ms`.
    pub window_ms: u32,
    /// Each window's loudness in LUFS, to 0.1; `null` for a silent window (below -70).
    pub lufs: Vec<Option<f64>>,
}

impl Levels {
    /// The loudness at `seconds` from the start: the window holding it.
    pub fn at(&self, seconds: f64) -> Option<f64> {
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        let index = (seconds * 1000.0 / f64::from(self.window_ms.max(1))) as usize;
        self.lufs.get(index).copied().flatten()
    }

    /// The loudness of the windows from `from` to `to` seconds (any window they touch).
    pub fn between(&self, from: f64, to: f64) -> Vec<Option<f64>> {
        let window = f64::from(self.window_ms.max(1)) / 1000.0;
        if !(from.is_finite() && to.is_finite()) || to <= from {
            return Vec::new();
        }
        let first = (from.max(0.0) / window).floor() as usize;
        let last = ((to / window).ceil() as usize).min(self.lufs.len());
        self.lufs.get(first..last).unwrap_or(&[]).to_vec()
    }
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

/// BS.1770-4's K-weighting at 48 kHz.
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

/// Streams interleaved 48 kHz samples and reports their loudness per window.
#[derive(Clone, Debug)]
pub struct LevelMeter {
    window_ms: u32,
    window: usize,
    channels: usize,
    filters: Vec<[Biquad; 2]>,
    acc: f64,
    count: usize,
    lufs: Vec<Option<f64>>,
}

impl LevelMeter {
    /// A meter for `channels` interleaved channels.
    pub fn new(channels: u16, options: &LevelOptions) -> LevelMeter {
        let options = options.clamped();
        let channels = usize::from(channels.max(1));
        LevelMeter {
            window_ms: options.window_ms,
            window: (RATE as usize * options.window_ms as usize) / 1000,
            channels,
            filters: vec![k_weighting(); channels],
            acc: 0.0,
            count: 0,
            lufs: Vec::new(),
        }
    }

    /// Adds interleaved samples; a trailing partial frame is ignored.
    pub fn push(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channels) {
            for (sample, filters) in frame.iter().zip(&mut self.filters) {
                let mut y = f64::from(*sample);
                for filter in filters.iter_mut() {
                    y = filter.process(y);
                }
                self.acc += y * y;
            }
            self.count += 1;
            if self.count == self.window {
                self.close();
            }
        }
    }

    fn close(&mut self) {
        let power = self.acc / self.count.max(1) as f64;
        let loudness = -0.691 + 10.0 * power.max(1e-30).log10();
        self.lufs
            .push((loudness >= GATE).then(|| (loudness * 10.0).round() / 10.0));
        self.acc = 0.0;
        self.count = 0;
    }

    /// The levels of everything pushed; a last partial window counts when at least a tenth
    /// of a window long.
    pub fn finish(mut self) -> Levels {
        if self.count * 10 >= self.window {
            self.close();
        }
        Levels {
            window_ms: self.window_ms,
            lufs: self.lufs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f64, amplitude: f32) -> Vec<f32> {
        let n = (seconds * f64::from(RATE)) as usize;
        (0..n)
            .map(|i| {
                amplitude * (2.0 * std::f32::consts::PI * 997.0 * i as f32 / RATE as f32).sin()
            })
            .collect()
    }

    #[test]
    fn windows_read_their_loudness_and_silence_reads_null() {
        // A -20 dBFS sine at 997 Hz reads -23 LUFS (BS.1770 calibration).
        let mut signal = tone(2.0, 0.1);
        signal.extend(vec![0.0; 2 * RATE as usize]);
        signal.extend(tone(1.05, 0.2));
        let mut meter = LevelMeter::new(1, &LevelOptions::default());
        for chunk in signal.chunks(3_333) {
            meter.push(chunk);
        }
        let levels = meter.finish();
        assert_eq!(levels.lufs.len(), 5, "the 50 ms tail is dropped");
        let first = levels.lufs[1].unwrap();
        assert!((first - -23.0).abs() < 0.15, "{first}");
        // The filters ring for a moment after the tone stops, far below it.
        assert!(
            levels.lufs[2].is_none_or(|l| l < -60.0),
            "{:?}",
            levels.lufs[2]
        );
        assert_eq!(levels.lufs[3], None);
        let loud = levels.lufs[4].unwrap();
        assert!((loud - first - 6.02).abs() < 0.2, "{loud}");
        assert_eq!(levels.at(1.5), levels.lufs[1]);
        assert_eq!(levels.at(-1.0), None);
        assert_eq!(levels.between(0.5, 2.5).len(), 3);
        assert_eq!(levels.at(3.5), None);
        assert!(levels.between(3.0, 2.0).is_empty());
        let short = LevelOptions { window_ms: 5 }.clamped();
        assert_eq!(short.window_ms, 100);
        let defaults: LevelOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, LevelOptions::default());
    }
}
