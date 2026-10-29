//! Hard shot cuts: each frame's luma histogram (64 bins, sampled on a grid about 160
//! points wide) is compared with the previous frame's. A cut is where the share of pixels
//! that moved between bins reaches the threshold, at least a minimum shot length after the
//! previous cut.

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Moment;

/// Histogram bins.
const BINS: usize = 64;
/// Sample points across a frame's width.
const GRID: u32 = 160;

/// Options of the `scenes` analyzer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneOptions {
    /// Histogram change, from 0 to 1, that counts as a cut (default 0.35).
    #[serde(default = "default_threshold")]
    pub threshold: f64,
    /// Shortest shot, in milliseconds: a cut sooner than this after the previous one (or
    /// the start) is ignored (default 500).
    #[serde(default = "default_min_shot")]
    pub min_shot_ms: u32,
}

fn default_threshold() -> f64 {
    0.35
}

fn default_min_shot() -> u32 {
    500
}

impl Default for SceneOptions {
    fn default() -> SceneOptions {
        SceneOptions {
            threshold: default_threshold(),
            min_shot_ms: default_min_shot(),
        }
    }
}

/// One cut: the first frame of a new shot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Cut {
    /// The frame's index in the stream, from 0.
    pub frame: u64,
    /// Its presentation time.
    pub time: Moment,
    /// How much the picture changed, from 0 to 1.
    pub score: f64,
}

/// Shot cuts of a stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Scenes {
    /// The threshold used.
    pub threshold: f64,
    /// The shortest shot allowed, in milliseconds.
    pub min_shot_ms: u32,
    /// Frames examined.
    pub frames: u64,
    /// Each cut, in order. The first shot starts at the stream's start and is not listed.
    pub cuts: Vec<Cut>,
}

/// An 8-bit luma plane, rows top to bottom.
#[derive(Clone, Copy, Debug)]
pub struct Luma<'a> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes from one row to the next.
    pub stride: usize,
    /// The plane, at least `stride * height` bytes.
    pub data: &'a [u8],
}

impl Luma<'_> {
    /// The normalised histogram of a grid of samples.
    fn histogram(&self) -> [f64; BINS] {
        let mut bins = [0u32; BINS];
        let step = (self.width / GRID).max(1) as usize;
        let mut count = 0u32;
        for y in (0..self.height as usize).step_by(step) {
            let row = &self.data[y * self.stride..][..self.width as usize];
            for &value in row.iter().step_by(step) {
                bins[usize::from(value) * BINS / 256] += 1;
                count += 1;
            }
        }
        let total = f64::from(count.max(1));
        bins.map(|n| f64::from(n) / total)
    }
}

/// Streams frames and reports shot cuts.
#[derive(Clone, Debug)]
pub struct SceneDetector {
    options: SceneOptions,
    min_shot: Flicks,
    previous: Option<[f64; BINS]>,
    shot_start: Option<Flicks>,
    frames: u64,
    cuts: Vec<Cut>,
}

impl SceneDetector {
    /// A detector with `options`.
    pub fn new(options: SceneOptions) -> SceneDetector {
        SceneDetector {
            min_shot: Flicks::new(i64::from(options.min_shot_ms) * 705_600),
            options,
            previous: None,
            shot_start: None,
            frames: 0,
            cuts: Vec::new(),
        }
    }

    /// Adds the next frame, shown at `pts`.
    pub fn push(&mut self, pts: Flicks, luma: Luma<'_>) {
        let histogram = luma.histogram();
        let start = *self.shot_start.get_or_insert(pts);
        if let Some(previous) = &self.previous {
            let score = previous
                .iter()
                .zip(&histogram)
                .map(|(a, b)| (a - b).abs())
                .sum::<f64>()
                / 2.0;
            if score >= self.options.threshold && pts - start >= self.min_shot {
                self.cuts.push(Cut {
                    frame: self.frames,
                    time: Moment::new(pts),
                    score: (score * 1000.0).round() / 1000.0,
                });
                self.shot_start = Some(pts);
            }
        }
        self.previous = Some(histogram);
        self.frames += 1;
    }

    /// The cuts of everything pushed.
    pub fn finish(self) -> Scenes {
        Scenes {
            threshold: self.options.threshold,
            min_shot_ms: self.options.min_shot_ms,
            frames: self.frames,
            cuts: self.cuts,
        }
    }
}

#[cfg(test)]
mod tests {
    use fosscut_time::Rate;

    use super::*;

    /// A 320x180 plane: a horizontal gradient offset by `shift`.
    fn plane(shift: u8, flat: Option<u8>) -> Vec<u8> {
        (0..180)
            .flat_map(|_| {
                (0..320u32).map(move |x| flat.unwrap_or(((x / 2) as u8).wrapping_add(shift)))
            })
            .collect()
    }

    fn luma(data: &[u8]) -> Luma<'_> {
        Luma {
            width: 320,
            height: 180,
            stride: 320,
            data,
        }
    }

    fn run(frames: &[Vec<u8>], options: SceneOptions) -> Scenes {
        let mut detector = SceneDetector::new(options);
        for (n, data) in frames.iter().enumerate() {
            detector.push(Rate::FPS_30.frames_to_flicks(n as i64), luma(data));
        }
        detector.finish()
    }

    #[test]
    fn cuts_land_on_the_first_frame_of_each_shot() {
        let mut frames = vec![plane(0, Some(40)); 30];
        frames.extend(vec![plane(0, Some(200)); 30]);
        frames.extend((0..30).map(|n| plane(n, None)));
        let scenes = run(&frames, SceneOptions::default());
        let at: Vec<(u64, String)> = scenes
            .cuts
            .iter()
            .map(|c| (c.frame, c.time.seconds.clone()))
            .collect();
        assert_eq!(at, [(30, "1".to_string()), (60, "2".to_string())]);
        assert_eq!(scenes.cuts[0].score, 1.0);
        assert_eq!(scenes.frames, 90);
    }

    #[test]
    fn gradual_change_and_short_flashes_are_not_cuts() {
        // A slow pan through the gradient: small differences every frame.
        let pan: Vec<Vec<u8>> = (0..60).map(|n| plane(n, None)).collect();
        assert!(run(&pan, SceneOptions::default()).cuts.is_empty());
        // A 5-frame flash at frames 5 to 9: both of its edges come less than 0.5 s after the
        // shot starts at frame 0, so neither is a cut.
        let mut flash = vec![plane(0, Some(40)); 5];
        flash.extend(vec![plane(0, Some(250)); 5]);
        flash.extend(vec![plane(0, Some(40)); 20]);
        assert!(run(&flash, SceneOptions::default()).cuts.is_empty());
        // With no minimum shot length, both edges of the flash are cuts.
        let eager = SceneOptions {
            threshold: 0.35,
            min_shot_ms: 0,
        };
        let frames: Vec<u64> = run(&flash, eager).cuts.iter().map(|c| c.frame).collect();
        assert_eq!(frames, [5, 10]);
        let defaults: SceneOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, SceneOptions::default());
    }
}
