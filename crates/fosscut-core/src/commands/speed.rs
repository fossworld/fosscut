//! Clip speed and freeze frames (ADR-0023).

use fosscut_model::{
    ClipContent, Id, IdKind, Interpolate, MAX_RATE, MIN_RATE, Pitch, SPEED_PRESETS, Speed,
    SpeedPoint, speed_preset,
};
use fosscut_time::{Flicks, TimeInput, TimeReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::clip::{ClipPlacement, ClipSplit};
use super::util::{ensure_unlocked, locate_clip, renew_effect_ids, resolve, timeline_mut};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// A clip after its speed changed.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SpeedChanged {
    /// Where the clip is now; its length follows the speed.
    pub placement: ClipPlacement,
    /// Its speed; absent at normal speed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<Speed>,
    /// How much of its media it plays.
    pub source_used: TimeReport,
    /// Later clips on the track moved by `ripple`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub moved: Vec<Id>,
}

/// Plays a clip faster, slower, along a speed curve, or backwards.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipSpeed {
    /// The video or audio clip.
    pub clip: Id,
    /// A constant rate from 0.05 to 100: 2 plays twice as fast, 0.5 at half speed. For a
    /// still frame use `clip.freeze`.
    #[serde(default)]
    pub rate: Option<f64>,
    /// A speed curve: two or more points `{at, rate}` with `at` from 0 (the clip's start)
    /// to 1 (its end); the rate changes linearly between them.
    #[serde(default)]
    pub curve: Option<Vec<SpeedPoint>>,
    /// A built-in curve: `montage`, `hero`, `bullet`, `jump_cut`, `flash_in` or
    /// `flash_out` (capabilities lists them under speed_presets).
    #[serde(default)]
    pub preset: Option<String>,
    /// Play backwards (true) or forwards (false); unchanged when omitted.
    #[serde(default)]
    pub reverse: Option<bool>,
    /// `preserve` keeps the sound's pitch (the default); `shift` lets it follow the speed.
    #[serde(default)]
    pub pitch: Option<Pitch>,
    /// `blend` mixes neighbouring source frames for smooth slow motion; `nearest` shows the
    /// nearest frame (the default).
    #[serde(default)]
    pub interpolate: Option<Interpolate>,
    /// Move the later clips on the track when the clip gets longer or shorter. Without it,
    /// a clip that would grow into the next one fails with `fosscut.clip.overlap`.
    #[serde(default)]
    pub ripple: bool,
    /// Back to normal: one to one and forwards.
    #[serde(default)]
    pub reset: bool,
}

impl ClipSpeed {
    /// The speed this request asks for, starting from the clip's current one.
    fn wanted(&self, current: Option<&Speed>) -> Result<Speed, ApiError> {
        let given = [
            self.rate.is_some(),
            self.curve.is_some(),
            self.preset.is_some(),
        ];
        if given.iter().filter(|g| **g).count() > 1 {
            return Err(invalid("clip.speed: give one of rate, curve and preset"));
        }
        if self.reset {
            if given.iter().any(|g| *g) || self.reverse.is_some() {
                return Err(invalid(
                    "clip.speed: reset returns to normal speed; give it alone",
                ));
            }
            return Ok(Speed::default());
        }
        if !given.iter().any(|g| *g)
            && self.reverse.is_none()
            && self.pitch.is_none()
            && self.interpolate.is_none()
        {
            return Err(invalid(
                "clip.speed: give a rate, curve, preset, reverse, pitch, interpolate or reset",
            ));
        }
        let mut speed = current.cloned().unwrap_or_default();
        if speed.is_frozen() {
            speed.rate = 1.0;
        }
        if let Some(rate) = self.rate {
            if !(MIN_RATE..=MAX_RATE).contains(&rate) {
                return Err(invalid(format!(
                    "clip.speed: rate must be from {MIN_RATE} to {MAX_RATE}"
                ))
                .with_hint("For a still frame, use clip.freeze."));
            }
            speed.rate = rate;
            speed.curve.clear();
        }
        let curve = match (&self.curve, &self.preset) {
            (Some(curve), _) => Some(curve.clone()),
            (_, Some(name)) => Some(speed_preset(name).ok_or_else(|| {
                let names: Vec<&str> = SPEED_PRESETS.iter().map(|(n, ..)| *n).collect();
                invalid(format!("clip.speed: there is no preset {name}"))
                    .with_hint(format!("Presets: {}.", names.join(", ")))
            })?),
            _ => None,
        };
        if let Some(curve) = curve {
            let ordered = curve.windows(2).all(|p| p[0].at < p[1].at);
            let inside = curve
                .iter()
                .all(|p| (0.0..=1.0).contains(&p.at) && (MIN_RATE..=MAX_RATE).contains(&p.rate));
            if curve.len() < 2 || !ordered || !inside {
                return Err(invalid(format!(
                    "clip.speed: a curve has two or more points with at rising from 0 to 1 and rates from {MIN_RATE} to {MAX_RATE}"
                )));
            }
            speed.rate = 1.0;
            speed.curve = curve;
        }
        if let Some(reverse) = self.reverse {
            speed.reverse = reverse;
        }
        if let Some(pitch) = self.pitch {
            speed.pitch = pitch;
        }
        if let Some(interpolate) = self.interpolate {
            speed.interpolate = interpolate;
        }
        Ok(speed)
    }
}

impl Command for ClipSpeed {
    const NAME: &'static str = "clip.speed";
    const SUMMARY: &'static str = "Change a video, audio or compound clip's speed: a constant rate (0.05 to 100), a speed curve or preset (montage, hero, bullet, jump_cut, flash_in, flash_out), reverse, pitch (preserve or shift) and smooth slow motion (interpolate blend). The clip keeps the media it plays, so faster clips get shorter; ripple moves the clips after it.";
    type Output = SpeedChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<SpeedChanged, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let original = ctx.project.timelines[&location.timeline].tracks[location.track].clips
            [location.clip]
            .clone();
        let total = match &original.content {
            ClipContent::Video { asset, .. } | ClipContent::Audio { asset, .. } => {
                ctx.project.media.get(asset).and_then(|a| a.info.duration)
            }
            // A compound clip plays its timeline at the speed (ADR-0025).
            ClipContent::Timeline { timeline, .. } => {
                ctx.project.timelines.get(timeline).map(|t| t.duration())
            }
            _ => {
                return Err(invalid(format!(
                    "clip.speed: {} is not a video, audio or compound clip",
                    original.id
                )));
            }
        };
        let speed = self.wanted(original.speed.as_ref())?;
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let rate = timeline.format.rate;
        let track = &mut timeline.tracks[location.track];
        ensure_unlocked(track)?;
        // The media the clip plays stays the same; its length follows the mean rate.
        let used = original.source_length();
        let frame = rate.frame_duration();
        let mut duration = if used.is_positive() {
            let frames = (used.get() as f64 / speed.mean_rate() / frame.get() as f64).round();
            Flicks::new(frame.get() * (frames as i64).max(1))
        } else {
            original.duration
        };
        let source_in = original.content.source_in().unwrap_or(Flicks::ZERO);
        // Rounding to whole frames can reach a little past the media's end: one frame less.
        while let Some(total) = total
            && duration > frame
            && source_in + speed.source_length(duration) > total
        {
            duration = duration - frame;
        }
        if let Some(total) = total
            && source_in + speed.source_length(duration) > total
        {
            return Err(ApiError::new(
                codes::CLIP_SOURCE_RANGE,
                format!(
                    "clip.speed: {} would play past the end of its media",
                    original.id
                ),
            )
            .with_hint("Trim the clip first, or choose a slower speed."));
        }
        let delta = duration - original.duration;
        let end = original.start + duration;
        let later = location.clip + 1..track.clips.len();
        let mut moved = Vec::new();
        if self.ripple {
            for clip in &mut track.clips[later] {
                clip.start = clip.start + delta;
                moved.push(clip.id.clone());
            }
        } else if let Some(next) = track.clips.get(location.clip + 1)
            && next.start < end
        {
            return Err(ApiError::new(
                codes::CLIP_OVERLAP,
                format!(
                    "clip.speed: {} would grow into {} at {}",
                    original.id,
                    next.id,
                    TimeReport::new(next.start, rate).timecode
                ),
            )
            .with_details(json!({
                "conflicting_clip": next.id,
                "new_end": TimeReport::new(end, rate),
            }))
            .with_hint("Pass ripple: true to move the later clips, or make room first."));
        }
        let clip = &mut track.clips[location.clip];
        clip.duration = duration;
        clip.speed = Some(speed).filter(|s| !s.is_normal());
        if let Some(transition) = &mut clip.transition
            && transition.duration > duration
        {
            transition.duration = duration;
        }
        Ok(SpeedChanged {
            placement: ClipPlacement::of(clip, &track.id, rate),
            speed: clip.speed.clone(),
            source_used: TimeReport::new(clip.source_length(), rate),
            moved,
        })
    }
}

/// Holds one frame of a video clip for a while, pushing what follows later.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipFreeze {
    /// The video clip.
    pub clip: Id,
    /// The timeline time of the frame to hold, inside the clip or at its end (the last
    /// frame); snapped to frames.
    pub at: TimeInput,
    /// How long to hold it; two seconds when omitted.
    #[serde(default)]
    pub duration: Option<TimeInput>,
}

/// Result of `clip.freeze`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Frozen {
    /// The new frozen clip.
    pub freeze: ClipPlacement,
    /// The part of the clip after the freeze, if it was split.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<ClipPlacement>,
    /// Clips moved later on the track to make room.
    pub moved: Vec<Id>,
}

impl Command for ClipFreeze {
    const NAME: &'static str = "clip.freeze";
    const SUMMARY: &'static str = "Insert a freeze frame: hold the frame of a video clip at a time for a duration (two seconds by default), splitting the clip there and moving what follows on its track later. The held frame is silent.";
    type Output = Frozen;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Frozen, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let rate = ctx.project.timelines[&location.timeline].format.rate;
        let original = ctx.project.timelines[&location.timeline].tracks[location.track].clips
            [location.clip]
            .clone();
        if !matches!(
            original.content,
            ClipContent::Video { .. } | ClipContent::Timeline { .. }
        ) {
            return Err(invalid(format!(
                "clip.freeze: {} is not a video or compound clip",
                original.id
            )));
        }
        let at = resolve(&self.at, rate)?;
        let hold = match &self.duration {
            Some(d) => resolve(d, rate)?,
            None => rate.snap(Flicks::from_millis(2000)),
        };
        if !hold.is_positive() {
            return Err(invalid("clip.freeze: the freeze lasts more than zero time"));
        }
        if !(original.start <= at && at <= original.end()) {
            return Err(ApiError::new(
                codes::CLIP_SPLIT_OUTSIDE,
                format!(
                    "clip.freeze: {} is not inside {}",
                    TimeReport::new(at, rate).timecode,
                    original.id
                ),
            ));
        }
        ensure_unlocked(&ctx.project.timelines[&location.timeline].tracks[location.track])?;
        // The frame shown at `at`; at the clip's end, its last frame.
        let shown = (at - original.start)
            .min(original.duration - rate.frame_duration())
            .max(Flicks::ZERO);
        let source = original.source_at(shown).unwrap_or(Flicks::ZERO);
        let after = if original.start < at && at < original.end() {
            let split = ClipSplit {
                clip: original.id.clone(),
                at: TimeInput::Flicks(at.get()),
                linked: false,
            }
            .apply(ctx)?;
            Some(split.right.clip)
        } else {
            None
        };
        let mut freeze = original.clone();
        freeze.id = ctx.project.allocate_id(IdKind::Clip);
        renew_effect_ids(ctx.project, &mut freeze);
        freeze.start = at;
        freeze.duration = hold;
        freeze.content = match &original.content {
            ClipContent::Video { asset, .. } => ClipContent::Video {
                asset: asset.clone(),
                source_in: source,
            },
            ClipContent::Timeline { timeline, .. } => ClipContent::Timeline {
                timeline: timeline.clone(),
                source_in: source,
            },
            other => other.clone(),
        };
        freeze.speed = Some(Speed {
            rate: 0.0,
            ..Speed::default()
        });
        freeze.transition = None;
        freeze.animations = Default::default();
        freeze.audio.fade_in = Flicks::ZERO;
        freeze.audio.fade_out = Flicks::ZERO;
        freeze.shift_keyframes(Flicks::ZERO - (at - original.start));
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let track = &mut timeline.tracks[location.track];
        let mut moved = Vec::new();
        for clip in track.clips.iter_mut().filter(|c| c.start >= at) {
            clip.start = clip.start + hold;
            moved.push(clip.id.clone());
        }
        let index = track.clips.partition_point(|c| c.start < at);
        let placement = ClipPlacement::of(&freeze, &track.id, rate);
        track.clips.insert(index, freeze);
        let after = after.and_then(|id| {
            track
                .clips
                .iter()
                .find(|c| c.id == id)
                .map(|c| ClipPlacement::of(c, &track.id, rate))
        });
        Ok(Frozen {
            freeze: placement,
            after,
            moved,
        })
    }
}

/// The speed presets `capabilities` lists.
pub(crate) fn speed_capabilities(brief: bool) -> Value {
    let presets: Vec<Value> = SPEED_PRESETS
        .iter()
        .map(|(name, meaning, points)| {
            if brief {
                json!({ "preset": name, "meaning": meaning })
            } else {
                let curve: Vec<Value> = points
                    .iter()
                    .map(|(at, rate)| json!({ "at": at, "rate": rate }))
                    .collect();
                json!({ "preset": name, "meaning": meaning, "curve": curve })
            }
        })
        .collect();
    json!(presets)
}
