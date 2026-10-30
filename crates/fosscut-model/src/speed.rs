//! Clip speed as a time map from timeline time to source time (ADR-0023).

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The slowest rate a clip plays at, other than a freeze (rate 0).
pub const MIN_RATE: f64 = 0.05;

/// The fastest rate a clip plays at.
pub const MAX_RATE: f64 = 100.0;

/// What happens to the pitch of sound played at another speed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Pitch {
    /// The sound keeps its pitch: it is time-stretched (WSOLA).
    #[default]
    Preserve,
    /// The sound is resampled, so its pitch follows the speed.
    Shift,
}

/// How frames between source frames are made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Interpolate {
    /// The source frame displayed at that time.
    #[default]
    Nearest,
    /// The two nearest source frames mixed by the position between them: smooth slow
    /// motion.
    Blend,
}

/// A point of a speed curve.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeedPoint {
    /// Where on the clip, from 0 (its start) to 1 (its end).
    pub at: f64,
    /// The rate there, from 0.05 to 100.
    pub rate: f64,
}

/// How a clip plays its source: faster, slower, along a curve, backwards or frozen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Speed {
    /// A constant rate from 0.05 to 100, or 0 to hold the first frame (a freeze). Ignored
    /// when `curve` is given, and then 1.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub rate: f64,
    /// A speed curve: the rate changes linearly between points and holds before the first
    /// and after the last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub curve: Vec<SpeedPoint>,
    /// Play the same source range backwards.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reverse: bool,
    /// Keep the sound's pitch, or let it follow the speed.
    #[serde(default, skip_serializing_if = "is_default")]
    pub pitch: Pitch,
    /// Show the nearest source frame, or blend the two around each time.
    #[serde(default, skip_serializing_if = "is_default")]
    pub interpolate: Interpolate,
}

impl Default for Speed {
    fn default() -> Speed {
        Speed {
            rate: 1.0,
            curve: Vec::new(),
            reverse: false,
            pitch: Pitch::default(),
            interpolate: Interpolate::default(),
        }
    }
}

fn one() -> f64 {
    1.0
}

fn is_one(value: &f64) -> bool {
    *value == 1.0
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

impl Speed {
    /// A constant rate.
    pub fn constant(rate: f64) -> Speed {
        Speed {
            rate,
            ..Speed::default()
        }
    }

    /// True when the clip plays its source one to one, forwards: the same as no speed.
    pub fn is_normal(&self) -> bool {
        self.curve.is_empty() && self.rate == 1.0 && !self.reverse
    }

    /// True for a freeze frame.
    pub fn is_frozen(&self) -> bool {
        self.curve.is_empty() && self.rate == 0.0
    }

    /// The rate at fraction `u` of the clip, holding the end rates outside 0 to 1.
    pub fn rate_at(&self, u: f64) -> f64 {
        let Some(first) = self.curve.first() else {
            return self.rate;
        };
        if u <= first.at {
            return first.rate;
        }
        for pair in self.curve.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if u <= b.at {
                let width = b.at - a.at;
                if width <= 0.0 {
                    return b.rate;
                }
                return a.rate + (b.rate - a.rate) * (u - a.at) / width;
            }
        }
        self.curve.last().map_or(self.rate, |p| p.rate)
    }

    /// The integral of the rate from 0 to `x`, for any `x` (the end rates hold outside 0
    /// to 1), in closed form per segment.
    pub fn integral(&self, x: f64) -> f64 {
        if self.curve.is_empty() {
            return self.rate * x;
        }
        self.since_first(x) - self.since_first(0.0)
    }

    /// The integral of the rate from the first curve point to `x`.
    fn since_first(&self, x: f64) -> f64 {
        let first = self.curve[0];
        if x <= first.at {
            return first.rate * (x - first.at);
        }
        let mut sum = 0.0;
        for pair in self.curve.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let width = b.at - a.at;
            if width <= 0.0 {
                continue;
            }
            if x <= b.at {
                let t = x - a.at;
                return sum + a.rate * t + (b.rate - a.rate) * t * t / (2.0 * width);
            }
            sum += (a.rate + b.rate) / 2.0 * width;
        }
        let last = self.curve[self.curve.len() - 1];
        sum + last.rate * (x - last.at)
    }

    /// The mean rate over the clip: the source it uses divided by its duration.
    pub fn mean_rate(&self) -> f64 {
        self.integral(1.0)
    }

    /// How much source a clip `duration` long uses, rounded to the nearest flick.
    pub fn source_length(&self, duration: Flicks) -> Flicks {
        Flicks::new((duration.get() as f64 * self.mean_rate()).round() as i64)
    }

    /// The forward position in the source, in flicks after the clip's `source_in`, at
    /// clip offset `offset` of a clip `duration` long, before reversing.
    fn forward(&self, offset: Flicks, duration: Flicks) -> f64 {
        if duration.get() <= 0 {
            return 0.0;
        }
        let d = duration.get() as f64;
        d * self.integral(offset.get() as f64 / d)
    }

    /// The source position, in flicks after the clip's `source_in`, shown at `offset` of
    /// a clip `duration` long. Exact, so callers can tell where it falls between frames.
    pub fn source_offset(&self, offset: Flicks, duration: Flicks) -> f64 {
        if self.is_frozen() {
            return 0.0;
        }
        let forward = self.forward(offset, duration);
        if self.reverse {
            self.source_length(duration).get() as f64 - forward
        } else {
            forward
        }
    }

    /// The clip offset where source position `position` (in flicks after `source_in`)
    /// plays, for a clip `duration` long; `None` outside the source it uses or when
    /// frozen.
    pub fn offset_of(&self, position: f64, duration: Flicks) -> Option<Flicks> {
        let length = self.source_length(duration).get() as f64;
        if self.is_frozen() || !(0.0..=length).contains(&position) || duration.get() <= 0 {
            return None;
        }
        let target = if self.reverse {
            length - position
        } else {
            position
        };
        let d = duration.get() as f64;
        if self.curve.is_empty() {
            return Some(Flicks::new((target / self.rate).round() as i64));
        }
        // The integral only grows (every rate is positive), so bisection finds it.
        let (mut low, mut high) = (0.0f64, 1.0f64);
        for _ in 0..64 {
            let mid = (low + high) / 2.0;
            if d * self.integral(mid) < target {
                low = mid;
            } else {
                high = mid;
            }
        }
        Some(Flicks::new((d * (low + high) / 2.0).round() as i64))
    }

    /// The speed of the part from `from` lasting `length` of a clip `duration` long: the
    /// curve keeps its shape over that part (ADR-0023 item 7). `from` may be negative and
    /// the part may reach past the clip's end, where the end rates hold.
    pub fn part(&self, duration: Flicks, from: Flicks, length: Flicks) -> Speed {
        if self.curve.is_empty() || duration.get() <= 0 || length.get() <= 0 {
            return self.clone();
        }
        let d = duration.get() as f64;
        let start = from.get() as f64 / d;
        let end = (from + length).get() as f64 / d;
        let span = end - start;
        let mut curve = vec![SpeedPoint {
            at: 0.0,
            rate: self.rate_at(start),
        }];
        for point in &self.curve {
            let at = (point.at - start) / span;
            if at > 1e-9 && at < 1.0 - 1e-9 {
                curve.push(SpeedPoint {
                    at,
                    rate: point.rate,
                });
            }
        }
        curve.push(SpeedPoint {
            at: 1.0,
            rate: self.rate_at(end),
        });
        let mut part = Speed {
            curve,
            ..self.clone()
        };
        if part.curve.iter().all(|p| p.rate == part.curve[0].rate) {
            part.rate = part.curve[0].rate;
            part.curve.clear();
        }
        part
    }

    /// The source position, after the clip's `source_in`, where the part from `from`
    /// lasting `length` of a clip `duration` long begins: what splits and trims set as the
    /// part's new `source_in`.
    pub fn part_source_in(&self, duration: Flicks, from: Flicks, length: Flicks) -> Flicks {
        if self.is_frozen() {
            return Flicks::ZERO;
        }
        let round = |v: f64| Flicks::new(v.round() as i64);
        if self.reverse {
            let total = self.source_length(duration).get() as f64;
            round(total - self.forward(from + length, duration))
        } else {
            round(self.forward(from, duration))
        }
    }
}

/// A built-in speed curve: its name, what it does, and its points as `(at, rate)`.
pub type SpeedPreset = (&'static str, &'static str, &'static [(f64, f64)]);

/// Built-in speed curves, written for FossCut; `clip.speed` copies a preset's points into
/// the clip.
pub const SPEED_PRESETS: &[SpeedPreset] = &[
    (
        "montage",
        "alternates fast and slow, for a rhythmic sequence",
        &[(0.0, 1.5), (0.25, 0.5), (0.5, 2.0), (0.75, 0.5), (1.0, 1.5)],
    ),
    (
        "hero",
        "fast, slow motion through the middle, then fast again",
        &[
            (0.0, 2.0),
            (0.35, 2.0),
            (0.45, 0.25),
            (0.6, 0.25),
            (0.7, 2.0),
            (1.0, 2.0),
        ],
    ),
    (
        "bullet",
        "normal speed, then very slow through the middle, then normal",
        &[
            (0.0, 1.0),
            (0.3, 1.0),
            (0.4, 0.1),
            (0.65, 0.1),
            (0.75, 1.0),
            (1.0, 1.0),
        ],
    ),
    (
        "jump_cut",
        "normal speed with a sudden fast skip in the middle",
        &[(0.0, 1.0), (0.45, 1.0), (0.5, 5.0), (0.55, 1.0), (1.0, 1.0)],
    ),
    (
        "flash_in",
        "starts very fast and settles to normal speed",
        &[(0.0, 5.0), (0.3, 1.0), (1.0, 1.0)],
    ),
    (
        "flash_out",
        "normal speed that speeds up at the end",
        &[(0.0, 1.0), (0.7, 1.0), (1.0, 5.0)],
    ),
];

/// The points of the built-in speed curve called `name`.
pub fn speed_preset(name: &str) -> Option<Vec<SpeedPoint>> {
    SPEED_PRESETS
        .iter()
        .find(|(n, ..)| *n == name)
        .map(|(_, _, points)| {
            points
                .iter()
                .map(|&(at, rate)| SpeedPoint { at, rate })
                .collect()
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn seconds(s: f64) -> Flicks {
        Flicks::from_seconds_f64(s).unwrap()
    }

    fn curve(points: &[(f64, f64)]) -> Speed {
        Speed {
            curve: points
                .iter()
                .map(|&(at, rate)| SpeedPoint { at, rate })
                .collect(),
            ..Speed::default()
        }
    }

    /// The integral by the midpoint rule, as a reference.
    fn numeric(speed: &Speed, x: f64) -> f64 {
        let steps = 200_000;
        let h = x / steps as f64;
        (0..steps)
            .map(|i| speed.rate_at((i as f64 + 0.5) * h) * h)
            .sum()
    }

    #[test]
    fn constant_curved_reversed_and_frozen_maps() {
        let d = seconds(4.0);
        let double = Speed::constant(2.0);
        assert_eq!(double.source_length(d), seconds(8.0));
        assert_eq!(
            double.source_offset(seconds(1.0), d),
            seconds(2.0).get() as f64
        );
        let back = Speed {
            reverse: true,
            ..Speed::constant(0.5)
        };
        assert_eq!(back.source_length(d), seconds(2.0));
        assert_eq!(
            back.source_offset(Flicks::ZERO, d),
            seconds(2.0).get() as f64
        );
        assert_eq!(back.source_offset(d, d), 0.0, "ends on the first frame");
        let frozen = Speed::constant(0.0);
        assert!(frozen.is_frozen());
        assert_eq!(frozen.source_length(d), Flicks::ZERO);
        assert_eq!(frozen.source_offset(seconds(3.0), d), 0.0);
        let ramp = curve(&[(0.25, 1.0), (0.75, 3.0)]);
        // Holds 1 for a quarter, ramps to 3 over half, holds 3 for a quarter: mean 2.
        assert!((ramp.mean_rate() - 2.0).abs() < 1e-12);
        for x in [-0.3, 0.1, 0.4, 0.75, 1.0, 1.4] {
            let exact = ramp.integral(x);
            let reference = if x < 0.0 {
                ramp.rate_at(0.0) * x
            } else {
                numeric(&ramp, x)
            };
            assert!(
                (exact - reference).abs() < 1e-6,
                "{x}: {exact} vs {reference}"
            );
        }
        assert!(Speed::default().is_normal() && !back.is_normal());
    }

    #[test]
    fn the_inverse_finds_where_source_plays() {
        let d = seconds(6.0);
        for speed in [
            Speed::constant(1.5),
            curve(&[(0.0, 0.5), (0.5, 4.0), (1.0, 1.0)]),
            Speed {
                reverse: true,
                ..curve(&[(0.0, 2.0), (1.0, 0.25)])
            },
        ] {
            for offset in [seconds(0.0), seconds(1.3), seconds(5.9)] {
                let position = speed.source_offset(offset, d);
                let back = speed.offset_of(position, d).unwrap();
                assert!(
                    (back - offset).get().abs() <= 2,
                    "{speed:?} {offset:?} {back:?}"
                );
            }
            let length = speed.source_length(d).get() as f64;
            assert_eq!(speed.offset_of(length + 1e6, d), None);
        }
        assert_eq!(Speed::constant(0.0).offset_of(0.0, d), None);
    }

    #[test]
    fn parts_keep_the_curve_and_meet_in_the_source() {
        let d = seconds(10.0);
        let speed = curve(&[(0.0, 1.0), (0.5, 3.0), (1.0, 1.0)]);
        let at = seconds(4.0);
        let left = speed.part(d, Flicks::ZERO, at);
        let right = speed.part(d, at, d - at);
        // The rate at each timeline moment is the same as before the split.
        for t in [1.0, 3.9] {
            let u = t / 4.0;
            assert!((left.rate_at(u) - speed.rate_at(t / 10.0)).abs() < 1e-9);
        }
        for t in [4.1, 7.0, 9.5] {
            let u = (t - 4.0) / 6.0;
            assert!((right.rate_at(u) - speed.rate_at(t / 10.0)).abs() < 1e-9);
        }
        let total = speed.source_length(d).get();
        let (l, r) = (
            left.source_length(at).get(),
            right.source_length(d - at).get(),
        );
        assert!((l + r - total).abs() <= 1, "{l} + {r} vs {total}");
        assert_eq!(speed.part_source_in(d, at, d - at), Flicks::new(l));
        // Reversed, the right part shows the start of the source.
        let back = Speed {
            reverse: true,
            ..speed.clone()
        };
        assert_eq!(back.part_source_in(d, at, d - at), Flicks::ZERO);
        assert_eq!(back.part_source_in(d, Flicks::ZERO, at).get(), total - l);
        // A flat part becomes a constant rate, and constant speeds split unchanged.
        let flat = curve(&[(0.0, 2.0), (0.5, 2.0), (1.0, 1.0)]).part(d, Flicks::ZERO, seconds(5.0));
        assert!(flat.curve.is_empty() && flat.rate == 2.0, "{flat:?}");
        assert_eq!(Speed::constant(3.0).part(d, at, at), Speed::constant(3.0));
        assert_eq!(Speed::constant(0.0).part_source_in(d, at, at), Flicks::ZERO);
    }

    #[test]
    fn presets_are_valid_curves() {
        for (name, meaning, points) in SPEED_PRESETS {
            assert!(!meaning.is_empty());
            let curve = speed_preset(name).unwrap();
            assert_eq!(curve.len(), points.len());
            assert!(curve.windows(2).all(|p| p[0].at < p[1].at), "{name}");
            assert!(curve.first().unwrap().at == 0.0 && curve.last().unwrap().at == 1.0);
            assert!(
                curve
                    .iter()
                    .all(|p| (MIN_RATE..=MAX_RATE).contains(&p.rate))
            );
        }
        assert!(speed_preset("warp").is_none());
    }
}
