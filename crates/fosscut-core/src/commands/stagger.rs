//! Staggered animations (ADR-0011, clip animations): one animation preset on several
//! clips, each waiting a step longer than the one before it, in a chosen order, so cards,
//! words and interface parts pop in (or leave, or pulse) one after another. The delays are
//! the clips' own animation delays, so each clip stays an ordinary clip with one animation.

use fosscut_model::{ClipAnimations, Id};
use fosscut_time::{Flicks, TimeInput, TimeReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::motion::{AnimationInput, animation};
use super::util::{ensure_unlocked, locate_clip, resolve, timeline_mut};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// Which of a clip's animations a stagger sets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StaggerPlace {
    /// The entrance: each clip comes in a step after the one before.
    #[default]
    In,
    /// The exit: each clip leaves a step after the one before, all done by their ends.
    Out,
    /// The repeating combo: each clip starts its cycles a step later, a wave.
    Combo,
}

impl StaggerPlace {
    fn name(self) -> &'static str {
        match self {
            StaggerPlace::In => "in",
            StaggerPlace::Out => "out",
            StaggerPlace::Combo => "combo",
        }
    }
}

/// The order staggered clips move in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StaggerOrder {
    /// As the clips are listed.
    #[default]
    Listed,
    /// The list backwards.
    Reverse,
    /// By start time on their timeline, then as listed.
    Timeline,
    /// By where each sits on the canvas at its start: leftmost first.
    LeftToRight,
    /// Rightmost first.
    RightToLeft,
    /// Highest first.
    TopToBottom,
    /// Lowest first.
    BottomToTop,
    /// Nearest the canvas centre first, outwards.
    CenterOut,
    /// Shuffled by `seed`, the same every time.
    Random,
}

/// Animates several clips with the same preset, each a step after the one before.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipsStagger {
    /// The clips, of any timelines; each at most once.
    pub clips: Vec<Id>,
    /// The animation preset, one that fits `place` (capabilities lists them under
    /// animations), such as `pop_in`, `rise_in`, `blur_in` or `glow_in`.
    pub preset: String,
    /// Which animation it sets: `in` (the default), `out` or `combo`.
    #[serde(default)]
    pub place: StaggerPlace,
    /// How long each clip's animation takes; half a second for in and out, one second per
    /// cycle for combo.
    #[serde(default)]
    pub duration: Option<TimeInput>,
    /// The wait between one clip and the next; a tenth of a second when omitted.
    #[serde(default)]
    pub step: Option<TimeInput>,
    /// The first clip's wait; none when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// The order they move in: `listed` (the default), `reverse`, `timeline`,
    /// `left_to_right`, `right_to_left`, `top_to_bottom`, `bottom_to_top`, `center_out` or
    /// `random`.
    #[serde(default)]
    pub order: StaggerOrder,
    /// The shuffle of `random`; 0 when omitted.
    #[serde(default)]
    pub seed: u64,
}

/// One clip of a stagger.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StaggeredClip {
    /// The clip.
    pub clip: Id,
    /// How long its animation waits, at its timeline's rate.
    pub delay: TimeReport,
    /// Its animations now.
    pub animations: ClipAnimations,
}

/// What `clips.stagger` set.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Staggered {
    /// The preset.
    pub preset: String,
    /// Which animation: `in`, `out` or `combo`.
    pub place: StaggerPlace,
    /// The clips in the order they move.
    pub clips: Vec<StaggeredClip>,
}

/// A 64-bit integer hash (the SplitMix64 finalizer): shuffles the same on every machine.
fn mix(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Command for ClipsStagger {
    const NAME: &'static str = "clips.stagger";
    const SUMMARY: &'static str = "Animate several clips with one preset, each a step after the one before (step 0.1s by default, after an optional start), in an order: as listed, reversed, by start time, by place on the canvas (left_to_right, top_to_bottom, center_out and others) or shuffled. Sets each clip's in, out or combo animation with a growing delay; for exits the first in order leaves first.";
    type Output = Staggered;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Staggered, ApiError> {
        if self.clips.is_empty() {
            return Err(invalid("clips.stagger: name one or more clips"));
        }
        let place = self.place.name();
        // Where each clip is and what orders it.
        struct Entry {
            listed: usize,
            id: Id,
            start: Flicks,
            position: [f64; 2],
        }
        let mut entries = Vec::with_capacity(self.clips.len());
        for (listed, id) in self.clips.iter().enumerate() {
            if self.clips[..listed].contains(id) {
                return Err(invalid(format!(
                    "clips.stagger: {id} is listed more than once"
                )));
            }
            let location = locate_clip(ctx.project, id)?;
            let clip = &ctx.project.timelines[&location.timeline].tracks[location.track].clips
                [location.clip];
            entries.push(Entry {
                listed,
                id: id.clone(),
                start: clip.start,
                position: clip.transform_at(Flicks::ZERO).position,
            });
        }
        let key = |e: &Entry| -> f64 {
            let [x, y] = e.position;
            match self.order {
                StaggerOrder::Listed | StaggerOrder::Timeline | StaggerOrder::Random => 0.0,
                StaggerOrder::Reverse => -(e.listed as f64),
                StaggerOrder::LeftToRight => x,
                StaggerOrder::RightToLeft => -x,
                StaggerOrder::TopToBottom => y,
                StaggerOrder::BottomToTop => -y,
                StaggerOrder::CenterOut => x.hypot(y),
            }
        };
        match self.order {
            StaggerOrder::Timeline => entries.sort_by_key(|e| e.start),
            StaggerOrder::Random => {
                entries.sort_by_key(|e| mix(self.seed ^ mix(e.listed as u64)));
            }
            // A stable sort keeps the listed order between clips that tie.
            _ => entries.sort_by(|a, b| key(a).total_cmp(&key(b))),
        }

        let default_ms = if self.place == StaggerPlace::Combo {
            1000
        } else {
            500
        };
        let count = entries.len();
        let mut clips = Vec::with_capacity(count);
        for (turn, entry) in entries.iter().enumerate() {
            let location = locate_clip(ctx.project, &entry.id)?;
            let timeline = timeline_mut(ctx.project, &location.timeline)?;
            let rate = timeline.format.rate;
            let step = match &self.step {
                Some(step) => resolve(step, rate)?,
                None => rate.snap(Flicks::from_millis(100)),
            };
            let first = match &self.start {
                Some(start) => resolve(start, rate)?,
                None => Flicks::ZERO,
            };
            if step.is_negative() || first.is_negative() {
                return Err(invalid("clips.stagger: step and start cannot be negative"));
            }
            // An exit's delay counts back from the clip's end: the first in order waits
            // the longest so it leaves first.
            let steps = match self.place {
                StaggerPlace::Out => count - 1 - turn,
                _ => turn,
            };
            let delay = first + Flicks::new(step.get().saturating_mul(steps as i64));
            let input = AnimationInput {
                preset: self.preset.clone(),
                duration: self.duration.clone(),
                delay: Some(TimeInput::Flicks(delay.get())),
            };
            let made = animation(Self::NAME, &input, place, rate, default_ms)?;
            let track = &mut timeline.tracks[location.track];
            ensure_unlocked(track)?;
            let clip = &mut track.clips[location.clip];
            if self.place != StaggerPlace::Combo && made.delay + made.duration > clip.duration {
                return Err(invalid(format!(
                    "clips.stagger: {} lasts {}, too short for its turn: a {} delay and a {} animation",
                    clip.id,
                    TimeReport::new(clip.duration, rate).timecode,
                    TimeReport::new(made.delay, rate).timecode,
                    TimeReport::new(made.duration, rate).timecode,
                ))
                .with_hint("Use a shorter step or duration, or lengthen the clips."));
            }
            match self.place {
                StaggerPlace::In => clip.animations.entrance = Some(made),
                StaggerPlace::Out => clip.animations.exit = Some(made),
                StaggerPlace::Combo => clip.animations.combo = Some(made),
            }
            clips.push(StaggeredClip {
                clip: clip.id.clone(),
                delay: TimeReport::new(delay, rate),
                animations: clip.animations.clone(),
            });
        }
        Ok(Staggered {
            preset: self.preset,
            place: self.place,
            clips,
        })
    }
}
