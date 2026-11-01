//! Transitions between clips and clip animations (ADR-0011, items 1 and 4).

use std::collections::BTreeMap;

use fosscut_model::{
    ANIMATION_PRESETS, Animation, Clip, ClipAnimations, Id, ParamValue, Transition,
    transition_type, transition_types,
};
use fosscut_time::{Flicks, Rate, TimeInput, TimeReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::util::{ensure_unlocked, locate_clip, resolve, timeline_mut};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// A clip's transition and animations after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MotionChanged {
    /// The clip.
    pub clip: Id,
    /// Its transition into the next clip, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    /// Its animations.
    pub animations: ClipAnimations,
}

impl MotionChanged {
    fn of(clip: &Clip) -> MotionChanged {
        MotionChanged {
            clip: clip.id.clone(),
            transition: clip.transition.clone(),
            animations: clip.animations.clone(),
        }
    }
}

/// Plays a transition over the cut from a clip to the next one on its track.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransitionSet {
    /// The clip the transition leaves; the next clip on its track must start where it ends.
    pub clip: Id,
    /// `dissolve`, `dip`, `wipe`, `slide`, `push`, `zoom`, `blur`, `iris`, `clock` or `spin`
    /// (capabilities lists them with their parameters under transitions).
    #[serde(rename = "type")]
    pub transition_type: String,
    /// How long it lasts, centred on the cut; half a second when omitted. No longer than
    /// either clip.
    #[serde(default)]
    pub duration: Option<TimeInput>,
    /// Parameters, such as `{"direction": "up"}` or `{"color": "#FFFFFF"}`.
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

impl Command for TransitionSet {
    const NAME: &'static str = "transition.set";
    const SUMMARY: &'static str = "Add or replace the transition from a clip into the next clip on its track (they must touch): dissolve, dip (to a color), wipe, slide, push (with a direction), zoom, blur, iris, clock or spin, centred on the cut. The sound crossfades with it.";
    type Output = MotionChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MotionChanged, ApiError> {
        let spec = transition_type(&self.transition_type).ok_or_else(|| {
            let names: Vec<&str> = transition_types().iter().map(|t| t.name).collect();
            ApiError::new(
                codes::TRANSITION_UNKNOWN_TYPE,
                format!(
                    "transition.set: there is no transition {}",
                    self.transition_type
                ),
            )
            .with_hint(format!("Transitions: {}.", names.join(", ")))
        })?;
        let location = locate_clip(ctx.project, &self.clip)?;
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let rate = timeline.format.rate;
        let duration = match &self.duration {
            Some(d) => resolve(d, rate)?,
            None => rate.snap(Flicks::from_millis(500)),
        };
        let track = &mut timeline.tracks[location.track];
        ensure_unlocked(track)?;
        let clip = &track.clips[location.clip];
        let next = track
            .clips
            .get(location.clip + 1)
            .filter(|n| n.start == clip.end());
        let Some(next) = next else {
            return Err(ApiError::new(
                codes::TRANSITION_NO_NEIGHBOR,
                format!(
                    "transition.set: no clip starts where {} ends, at {}",
                    clip.id,
                    TimeReport::new(clip.end(), rate).timecode
                ),
            )
            .with_hint(
                "Move the next clip so it starts where this one ends, or close the gap first.",
            ));
        };
        let longest = clip.duration.min(next.duration);
        if !duration.is_positive() || duration > longest {
            return Err(invalid(format!(
                "transition.set: the transition must last more than zero and at most {} (the shorter clip)",
                TimeReport::new(longest, rate).timecode
            )));
        }
        let mut params = BTreeMap::new();
        for (name, value) in &self.params {
            let param = spec.param(name).ok_or_else(|| {
                invalid(format!(
                    "transition.set: {} has no parameter {name}",
                    spec.name
                ))
            })?;
            let parsed = match (param.kind, value) {
                (fosscut_model::ParamKind::Choice, Value::String(n))
                    if param.choices.contains(&n.as_str()) =>
                {
                    ParamValue::Name(n.clone())
                }
                (fosscut_model::ParamKind::Color, Value::String(n)) => {
                    let color = fosscut_model::Color::parse(n).map_err(invalid)?;
                    ParamValue::Name(color.as_str().to_string())
                }
                (fosscut_model::ParamKind::Number, Value::Number(n)) => {
                    let v = n.as_f64().unwrap_or(f64::NAN);
                    if !(v.is_finite()
                        && param.min.is_none_or(|m| v >= m)
                        && param.max.is_none_or(|m| v <= m))
                    {
                        return Err(invalid(format!("transition.set: {name} is out of range")));
                    }
                    ParamValue::Number(v)
                }
                _ => {
                    return Err(invalid(format!(
                        "transition.set: {name} must be {}",
                        if param.choices.is_empty() {
                            "a number or color".to_string()
                        } else {
                            format!("one of {}", param.choices.join(", "))
                        }
                    )));
                }
            };
            params.insert(name.clone(), parsed);
        }
        let clip = &mut track.clips[location.clip];
        clip.transition = Some(Transition {
            transition_type: spec.name.to_string(),
            duration,
            params,
        });
        Ok(MotionChanged::of(clip))
    }
}

/// Removes a clip's transition into the next clip.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransitionRemove {
    /// The clip the transition leaves.
    pub clip: Id,
}

impl Command for TransitionRemove {
    const NAME: &'static str = "transition.remove";
    const SUMMARY: &'static str =
        "Remove the transition from a clip into the next one: a plain cut again.";
    type Output = MotionChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MotionChanged, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let track = &mut timeline.tracks[location.track];
        ensure_unlocked(track)?;
        let clip = &mut track.clips[location.clip];
        if clip.transition.take().is_none() {
            return Err(invalid(format!(
                "transition.remove: {} has no transition",
                clip.id
            )));
        }
        Ok(MotionChanged::of(clip))
    }
}

/// An animation to give a clip.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnimationInput {
    /// The preset (capabilities lists them under animations, with where each applies).
    pub preset: String,
    /// How long it takes; half a second for in and out, one second per cycle for combo.
    #[serde(default)]
    pub duration: Option<TimeInput>,
    /// How long it waits: an entrance starts this long after the clip's start (hidden or in
    /// its first state until then), an exit ends this long before the clip's end, a combo
    /// starts its cycles this late; none when omitted. clips.stagger sets growing delays.
    #[serde(default)]
    pub delay: Option<TimeInput>,
}

/// The animation `input` describes for `place` (`in`, `out` or `combo`) on a timeline at
/// `rate`, lasting `default_ms` when it gives no duration; `command` names the caller in
/// errors.
pub(super) fn animation(
    command: &str,
    input: &AnimationInput,
    place: &str,
    rate: Rate,
    default_ms: i64,
) -> Result<Animation, ApiError> {
    let fits = ANIMATION_PRESETS
        .iter()
        .any(|(name, places, _)| *name == input.preset && places.contains(place));
    if !fits {
        let names: Vec<&str> = ANIMATION_PRESETS
            .iter()
            .filter(|(_, places, _)| places.contains(place))
            .map(|(n, ..)| *n)
            .collect();
        return Err(invalid(format!(
            "{command}: {} is not a {place} animation",
            input.preset
        ))
        .with_hint(format!("{place} animations: {}.", names.join(", "))));
    }
    let duration = match &input.duration {
        Some(d) => resolve(d, rate)?,
        None => rate.snap(Flicks::from_millis(default_ms)),
    };
    if !duration.is_positive() {
        return Err(invalid(format!(
            "{command}: an animation takes more than zero time"
        )));
    }
    let delay = match &input.delay {
        Some(d) => resolve(d, rate)?,
        None => Flicks::ZERO,
    };
    if delay.is_negative() {
        return Err(invalid(format!("{command}: a delay cannot be negative")));
    }
    Ok(Animation {
        preset: input.preset.clone(),
        duration,
        delay,
    })
}

/// Sets how a clip moves in, out and while it shows.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnimationSet {
    /// The clip.
    pub clip: Id,
    /// The entrance: `fade`, `slide_left`, `slide_right`, `slide_up`, `slide_down`,
    /// `zoom_in`, `zoom_out`, `spin`, `pop` or `drop`, or one made for entrances such as
    /// `blur_in`, `pop_in`, `rise_in`, `swoop_in` or `glow_in`, or `draw_on` for a shape
    /// clip's outline (capabilities lists them all under animations).
    #[serde(default, rename = "in")]
    pub entrance: Option<AnimationInput>,
    /// The exit: the same presets, played backwards up to the clip's end, or one made for
    /// exits such as `blur_out`, `pop_out`, `rise_out`, `swoop_out` or `glow_out`, or
    /// `draw_off` for a shape clip's outline.
    #[serde(default, rename = "out")]
    pub exit: Option<AnimationInput>,
    /// A repeating animation: `pulse`, `swing`, `float` or `shake`.
    #[serde(default)]
    pub combo: Option<AnimationInput>,
    /// Animations to remove: any of `in`, `out` and `combo`.
    #[serde(default)]
    pub clear: Vec<String>,
}

impl Command for AnimationSet {
    const NAME: &'static str = "animation.set";
    const SUMMARY: &'static str = "Animate a clip in (fade, slide_left/right/up/down, zoom_in, zoom_out, spin, pop, drop, blur_in, pop_in, rise_in, swoop_in, glow_in), out (the same reversed, or blur_out, pop_out, rise_out, swoop_out, glow_out), or with a repeating combo (pulse, swing, float, shake), each after an optional delay; clear removes them. Animations move placement and opacity (and blur or glow) on top of keyframes; clips.stagger animates several clips one after another. draw_on (in) and draw_off (out) draw a shape clip's outline on and off through its trims.";
    type Output = MotionChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MotionChanged, ApiError> {
        if self.entrance.is_none()
            && self.exit.is_none()
            && self.combo.is_none()
            && self.clear.is_empty()
        {
            return Err(invalid("animation.set: give in, out, combo or clear"));
        }
        let location = locate_clip(ctx.project, &self.clip)?;
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let rate = timeline.format.rate;
        let track = &mut timeline.tracks[location.track];
        ensure_unlocked(track)?;
        let clip = &mut track.clips[location.clip];
        let build = |input: &AnimationInput, place: &str, default_ms: i64| {
            animation(Self::NAME, input, place, rate, default_ms)
        };
        for place in &self.clear {
            match place.as_str() {
                "in" => clip.animations.entrance = None,
                "out" => clip.animations.exit = None,
                "combo" => clip.animations.combo = None,
                other => {
                    return Err(invalid(format!(
                        "animation.set: clear takes in, out or combo, not {other}"
                    )));
                }
            }
        }
        if let Some(input) = &self.entrance {
            clip.animations.entrance = Some(build(input, "in", 500)?);
        }
        if let Some(input) = &self.exit {
            clip.animations.exit = Some(build(input, "out", 500)?);
        }
        if let Some(input) = &self.combo {
            clip.animations.combo = Some(build(input, "combo", 1000)?);
        }
        Ok(MotionChanged::of(clip))
    }
}

/// The transitions and animations `capabilities` lists.
pub(crate) fn motion_capabilities(brief: bool) -> Value {
    let transitions: Vec<Value> = transition_types()
        .iter()
        .map(|t| {
            if brief {
                json!({ "type": t.name, "title": t.title, "summary": t.summary })
            } else {
                json!(t)
            }
        })
        .collect();
    let animations: Vec<Value> = ANIMATION_PRESETS
        .iter()
        .map(|(name, places, meaning)| json!({ "preset": name, "applies": places, "meaning": meaning }))
        .collect();
    json!({ "transitions": transitions, "animations": animations })
}
