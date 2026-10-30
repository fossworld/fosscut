//! Clip properties and their keyframes (ADR-0011, item 3): `clip.set` sets properties of
//! the closed vocabulary by path, `keyframe.set` adds or replaces a keyframe, and
//! `keyframe.remove` removes one or all keyframes of a property.

use std::collections::BTreeMap;

use fosscut_model::{
    Applies, BlendMode, Clip, ClipAudio, ClipContent, Crop, Id, Interpolation, KeyValue, Keyframe,
    Property, PropertyKind, Transform, clip_properties, clip_property,
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

/// A hint listing every property path with what it takes.
pub(crate) fn paths_hint() -> String {
    let paths: Vec<String> = clip_properties()
        .iter()
        .map(|p| format!("{} ({})", p.path, p.meaning))
        .collect();
    format!("Properties: {}.", paths.join("; "))
}

/// The property at `path` if `clip` has it.
fn property_of(command: &str, clip: &Clip, path: &str) -> Result<Property, ApiError> {
    let property = clip_property(path).ok_or_else(|| {
        invalid(format!("{command}: there is no property {path}"))
            .with_hint(paths_hint())
            .with_details(json!({ "unknown": path }))
    })?;
    let picture = !matches!(clip.content, ClipContent::Audio { .. });
    let sound = matches!(
        clip.content,
        ClipContent::Audio { .. } | ClipContent::Video { .. }
    );
    let has = match property.applies {
        Applies::Picture => picture,
        Applies::Sound => sound,
        Applies::Any => true,
    };
    if !has {
        let why = if picture {
            "it has no sound"
        } else {
            "it has no picture"
        };
        return Err(invalid(format!(
            "{command}: {} is a {} clip; {why}, so it has no {path}",
            clip.id,
            clip.content.kind_name()
        )));
    }
    Ok(property)
}

fn number(command: &str, property: &Property, value: &Value) -> Result<f64, ApiError> {
    let v = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| {
        invalid(format!(
            "{command}: {} must be a number, not {value}",
            property.path
        ))
    })?;
    in_range(command, property, v)
}

fn in_range(command: &str, property: &Property, v: f64) -> Result<f64, ApiError> {
    let low = property.min.is_none_or(|m| v >= m);
    let high = property.max.is_none_or(|m| v <= m);
    // Scales and stretches must stay above zero as plain values.
    let positive = !(matches!(property.path, "transform.scale" | "transform.stretch") && v <= 0.0);
    if low && high && positive {
        return Ok(v);
    }
    let range = match (property.min, property.max) {
        (Some(a), Some(b)) => format!("from {a} to {b}"),
        (Some(a), None) if positive => format!("at least {a}"),
        (Some(a), None) => format!("greater than {a}"),
        (None, Some(b)) => format!("at most {b}"),
        (None, None) => "finite".to_string(),
    };
    Err(invalid(format!(
        "{command}: {} must be {range}, not {v}",
        property.path
    )))
}

fn pair(command: &str, property: &Property, value: &Value) -> Result<[f64; 2], ApiError> {
    let items = value.as_array().filter(|a| a.len() == 2).ok_or_else(|| {
        invalid(format!(
            "{command}: {} must be [x, y], not {value}",
            property.path
        ))
    })?;
    Ok([
        number(command, property, &items[0])?,
        number(command, property, &items[1])?,
    ])
}

/// A keyframe value for an animatable property.
fn key_value(command: &str, property: &Property, value: &Value) -> Result<KeyValue, ApiError> {
    match property.kind {
        PropertyKind::Number => Ok(KeyValue::Number(number(command, property, value)?)),
        PropertyKind::Pair => Ok(KeyValue::Pair(pair(command, property, value)?)),
        _ => Err(invalid(format!(
            "{command}: {} does not take keyframes",
            property.path
        ))),
    }
}

/// Sets the plain value of `property` on `clip`.
fn assign(
    command: &str,
    clip: &mut Clip,
    property: &Property,
    value: &Value,
    rate: Rate,
) -> Result<(), ApiError> {
    let choice = || {
        value
            .as_str()
            .filter(|name| property.choices.contains(name))
            .ok_or_else(|| {
                invalid(format!(
                    "{command}: {} must be one of {}, not {value}",
                    property.path,
                    property.choices.join(", ")
                ))
            })
    };
    let flag = || {
        value.as_bool().ok_or_else(|| {
            invalid(format!(
                "{command}: {} must be true or false, not {value}",
                property.path
            ))
        })
    };
    let time = || -> Result<Flicks, ApiError> {
        let input: TimeInput = serde_json::from_value(value.clone()).map_err(|e| {
            invalid(format!(
                "{command}: {} must be a time such as {{\"seconds\": 0.5}} or {{\"frames\": 15}}: {e}",
                property.path
            ))
        })?;
        let t = resolve(&input, rate)?;
        if t.is_negative() {
            return Err(invalid(format!(
                "{command}: {} cannot be negative",
                property.path
            )));
        }
        Ok(t)
    };
    let t = &mut clip.transform;
    match property.path {
        "transform.position" => t.position = pair(command, property, value)?,
        "transform.scale" => t.scale = number(command, property, value)?,
        "transform.stretch" => t.stretch = pair(command, property, value)?,
        "transform.rotation" => t.rotation = number(command, property, value)?,
        "transform.flip_horizontal" => t.flip_horizontal = flag()?,
        "transform.flip_vertical" => t.flip_vertical = flag()?,
        "crop.left" => clip.crop.left = number(command, property, value)?,
        "crop.right" => clip.crop.right = number(command, property, value)?,
        "crop.top" => clip.crop.top = number(command, property, value)?,
        "crop.bottom" => clip.crop.bottom = number(command, property, value)?,
        "opacity" => clip.opacity = number(command, property, value)?,
        "blend" => clip.blend = serde_json::from_value(json!(choice()?)).unwrap_or_default(),
        "enabled" => clip.enabled = flag()?,
        "volume" => clip.volume = number(command, property, value)?,
        "audio.pan" => clip.audio.pan = number(command, property, value)?,
        "audio.fade_in" => clip.audio.fade_in = time()?,
        "audio.fade_out" => clip.audio.fade_out = time()?,
        "audio.channels" => {
            clip.audio.channels = serde_json::from_value(json!(choice()?)).unwrap_or_default()
        }
        other => return Err(invalid(format!("{command}: {other} cannot be set"))),
    }
    Ok(())
}

/// A keyframe as agents read it: its time on the timeline and from the clip's start.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct KeyframeReport {
    /// Position on the timeline.
    pub at: TimeReport,
    /// Position from the clip's start.
    pub offset: TimeReport,
    /// The value.
    pub value: KeyValue,
    /// How the value moves on to the next keyframe.
    pub interpolation: Interpolation,
}

/// Keyframes of every property of `clip`, with timeline times.
pub(crate) fn keyframe_reports(clip: &Clip, rate: Rate) -> BTreeMap<String, Vec<KeyframeReport>> {
    clip.keyframes
        .iter()
        .map(|(path, keys)| {
            let reports = keys
                .iter()
                .map(|k| KeyframeReport {
                    at: TimeReport::new(clip.start + k.time, rate),
                    offset: TimeReport::new(k.time, rate),
                    value: k.value,
                    interpolation: k.interpolation,
                })
                .collect();
            (path.clone(), reports)
        })
        .collect()
}

/// A clip's properties after `clip.set` or a keyframe change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClipProperties {
    /// The clip.
    pub clip: Id,
    /// Its track.
    pub track: Id,
    /// Placement on the canvas.
    pub transform: Transform,
    /// Edges removed from the picture.
    pub crop: Crop,
    /// Opacity, 0 to 1.
    pub opacity: f64,
    /// Blend mode.
    pub blend: BlendMode,
    /// Linear audio gain.
    pub volume: f64,
    /// Fades, pan and channels.
    pub audio: ClipAudio,
    /// Whether the clip is drawn and heard.
    pub enabled: bool,
    /// Keyframes by property path.
    pub keyframes: BTreeMap<String, Vec<KeyframeReport>>,
}

impl ClipProperties {
    fn of(clip: &Clip, track: &Id, rate: Rate) -> ClipProperties {
        ClipProperties {
            clip: clip.id.clone(),
            track: track.clone(),
            transform: clip.transform.clone(),
            crop: clip.crop.clone(),
            opacity: clip.opacity,
            blend: clip.blend,
            volume: clip.volume,
            audio: clip.audio.clone(),
            enabled: clip.enabled,
            keyframes: keyframe_reports(clip, rate),
        }
    }
}

/// Runs `edit` on the clip, after checking its track is unlocked.
fn with_clip<T>(
    ctx: &mut Context<'_>,
    id: &Id,
    edit: impl FnOnce(&mut Clip, Rate) -> Result<T, ApiError>,
) -> Result<(T, ClipProperties), ApiError> {
    let location = locate_clip(ctx.project, id)?;
    let timeline = timeline_mut(ctx.project, &location.timeline)?;
    let rate = timeline.format.rate;
    let track = &mut timeline.tracks[location.track];
    ensure_unlocked(track)?;
    let track_id = track.id.clone();
    let clip = &mut track.clips[location.clip];
    let result = edit(clip, rate)?;
    Ok((result, ClipProperties::of(clip, &track_id, rate)))
}

/// Changes a clip's properties: placement, crop, opacity, blend mode, sound, and whether
/// it is active.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipSet {
    /// The clip.
    pub clip: Id,
    /// Property paths with their new values, for example
    /// `{"transform.position": [0.2, 0], "transform.scale": 1.8, "blend": "screen"}`.
    /// Paths: `transform.position`, `transform.scale`, `transform.stretch`,
    /// `transform.rotation`, `transform.flip_horizontal`, `transform.flip_vertical`,
    /// `crop.left`, `crop.right`, `crop.top`, `crop.bottom`, `opacity`, `blend`, `enabled`,
    /// `volume`, `audio.pan`, `audio.fade_in`, `audio.fade_out`, `audio.channels`.
    pub set: BTreeMap<String, Value>,
}

impl Command for ClipSet {
    const NAME: &'static str = "clip.set";
    const SUMMARY: &'static str = "Set a clip's properties by path: transform.position, transform.scale, transform.stretch (non-uniform), transform.rotation, transform.flip_horizontal and flip_vertical (mirror), crop.left/right/top/bottom, opacity, blend (multiply, screen, overlay and others), enabled (false deactivates), volume, audio.pan, audio.fade_in, audio.fade_out, audio.channels (fill channel). Position and scale are relative to the canvas and the fitted size. Properties with keyframes are set with keyframe.set instead.";
    type Output = ClipProperties;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipProperties, ApiError> {
        if self.set.is_empty() {
            return Err(invalid("clip.set: give at least one property").with_hint(paths_hint()));
        }
        let clip_id = self.clip.clone();
        let (_, properties) = with_clip(ctx, &self.clip, |clip, rate| {
            for (path, value) in &self.set {
                let property = property_of(Self::NAME, clip, path)?;
                if clip.keyframes.contains_key(path) {
                    return Err(ApiError::new(
                        codes::CLIP_PROPERTY_ANIMATED,
                        format!("clip.set: {path} of {clip_id} has keyframes"),
                    )
                    .with_hint("Set its value at a time with keyframe.set, or remove its keyframes with keyframe.remove (all: true) first."));
                }
                assign(Self::NAME, clip, &property, value, rate)?;
            }
            Ok(())
        })?;
        Ok(properties)
    }
}

/// Easing from a keyframe to the next.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    /// Constant rate.
    Linear,
    /// Keep the value until the next keyframe.
    Hold,
    /// Start slowly.
    EaseIn,
    /// End slowly.
    EaseOut,
    /// Start and end slowly.
    EaseInOut,
    /// A custom cubic Bezier `[x1, y1, x2, y2]` as in CSS; x1 and x2 from 0 to 1.
    Bezier([f64; 4]),
}

impl Easing {
    fn interpolation(&self) -> Result<Interpolation, ApiError> {
        Ok(match self {
            Easing::Linear => Interpolation::Linear,
            Easing::Hold => Interpolation::Hold,
            Easing::EaseIn => Interpolation::EASE_IN,
            Easing::EaseOut => Interpolation::EASE_OUT,
            Easing::EaseInOut => Interpolation::EASE_IN_OUT,
            Easing::Bezier(points) => {
                let [x1, y1, x2, y2] = *points;
                if !(points.iter().all(|p| p.is_finite())
                    && (0.0..=1.0).contains(&x1)
                    && (0.0..=1.0).contains(&x2))
                {
                    return Err(invalid(
                        "keyframe.set: Bezier x control points must be from 0 to 1",
                    ));
                }
                Interpolation::Bezier([x1, y1, x2, y2])
            }
        })
    }
}

/// The keyframe offset from the clip's start for `at` (timeline) or `offset` (from the
/// clip's start), exactly one of them, inside the clip.
fn key_offset(
    command: &str,
    clip: &Clip,
    at: Option<&TimeInput>,
    offset: Option<&TimeInput>,
    rate: Rate,
) -> Result<Flicks, ApiError> {
    let offset = match (at, offset) {
        (Some(at), None) => resolve(at, rate)? - clip.start,
        (None, Some(offset)) => resolve(offset, rate)?,
        _ => {
            return Err(invalid(format!(
                "{command}: give the keyframe's time as at (on the timeline) or offset (from the clip's start), not both"
            )));
        }
    };
    if offset.is_negative() || offset > clip.duration {
        return Err(ApiError::new(
            codes::KEYFRAME_OUTSIDE,
            format!(
                "{command}: {} is outside {}",
                TimeReport::new(clip.start + offset, rate).timecode,
                clip.id
            ),
        )
        .with_details(json!({
            "clip_start": TimeReport::new(clip.start, rate),
            "clip_end": TimeReport::new(clip.end(), rate),
        })));
    }
    Ok(offset)
}

/// The plain value of an animatable property, as a keyframe value.
fn plain_value(clip: &Clip, path: &str) -> Option<KeyValue> {
    let t = &clip.transform;
    Some(match path {
        "transform.position" => KeyValue::Pair(t.position),
        "transform.scale" => KeyValue::Number(t.scale),
        "transform.stretch" => KeyValue::Pair(t.stretch),
        "transform.rotation" => KeyValue::Number(t.rotation),
        "crop.left" => KeyValue::Number(clip.crop.left),
        "crop.right" => KeyValue::Number(clip.crop.right),
        "crop.top" => KeyValue::Number(clip.crop.top),
        "crop.bottom" => KeyValue::Number(clip.crop.bottom),
        "opacity" => KeyValue::Number(clip.opacity),
        "volume" => KeyValue::Number(clip.volume),
        "audio.pan" => KeyValue::Number(clip.audio.pan),
        _ => return None,
    })
}

/// Adds a keyframe to a clip property, or replaces the one at that time.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyframeSet {
    /// The clip.
    pub clip: Id,
    /// An animatable property: `transform.position`, `transform.scale`,
    /// `transform.stretch`, `transform.rotation`, `crop.left`, `crop.right`, `crop.top`,
    /// `crop.bottom`, `opacity`, `volume` or `audio.pan`.
    pub property: String,
    /// Timeline position of the keyframe, from the clip's start to its end; snapped to the
    /// nearest frame. Give this or `offset`.
    #[serde(default)]
    pub at: Option<TimeInput>,
    /// Position from the clip's start, instead of `at`.
    #[serde(default)]
    pub offset: Option<TimeInput>,
    /// The value (a number, or `[x, y]` for position and stretch). When omitted, the value
    /// the property has at that time, so the animation does not change yet.
    #[serde(default)]
    pub value: Option<Value>,
    /// How the value moves from this keyframe to the next: `linear` (the default for a new
    /// keyframe), `hold`, `ease_in`, `ease_out`, `ease_in_out`, or `{"bezier": [x1, y1, x2,
    /// y2]}`. Unchanged for an existing keyframe when omitted.
    #[serde(default)]
    pub easing: Option<Easing>,
}

/// The keyframes of a property after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct KeyframesChanged {
    /// The property.
    pub property: String,
    /// The clip and all its properties.
    #[serde(flatten)]
    pub clip: ClipProperties,
}

impl Command for KeyframeSet {
    const NAME: &'static str = "keyframe.set";
    const SUMMARY: &'static str = "Animate a clip property: add a keyframe at a time (or replace the one there) with a value and an easing (linear, hold, ease_in, ease_out, ease_in_out or a Bezier). Between keyframes the value moves along the easing; before the first and after the last it holds. Animatable: transform.position, transform.scale, transform.stretch, transform.rotation, crop edges, opacity, volume, audio.pan.";
    type Output = KeyframesChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<KeyframesChanged, ApiError> {
        let easing = self
            .easing
            .as_ref()
            .map(Easing::interpolation)
            .transpose()?;
        let path = self.property.clone();
        let (_, properties) = with_clip(ctx, &self.clip, |clip, rate| {
            let property = property_of(Self::NAME, clip, &path)?;
            if !property.animatable {
                return Err(invalid(format!(
                    "keyframe.set: {path} does not take keyframes; set it with clip.set"
                )));
            }
            let offset = key_offset(
                Self::NAME,
                clip,
                self.at.as_ref(),
                self.offset.as_ref(),
                rate,
            )?;
            let value = match &self.value {
                Some(value) => key_value(Self::NAME, &property, value)?,
                None => clip
                    .keyed(&path, offset)
                    .or_else(|| plain_value(clip, &path))
                    .ok_or_else(|| invalid(format!("keyframe.set: {path} has no value")))?,
            };
            let keys = clip.keyframes.entry(path.clone()).or_default();
            let index = keys.partition_point(|k| k.time < offset);
            match keys.get_mut(index).filter(|k| k.time == offset) {
                Some(existing) => {
                    existing.value = value;
                    if let Some(interpolation) = easing {
                        existing.interpolation = interpolation;
                    }
                }
                None => keys.insert(
                    index,
                    Keyframe {
                        time: offset,
                        value,
                        interpolation: easing.unwrap_or_default(),
                    },
                ),
            }
            Ok(())
        })?;
        Ok(KeyframesChanged {
            property: self.property,
            clip: properties,
        })
    }
}

/// Removes one keyframe of a clip property, or all of them.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyframeRemove {
    /// The clip.
    pub clip: Id,
    /// The property.
    pub property: String,
    /// Timeline position of the keyframe to remove. Give this, `offset`, or `all`.
    #[serde(default)]
    pub at: Option<TimeInput>,
    /// Position from the clip's start, instead of `at`.
    #[serde(default)]
    pub offset: Option<TimeInput>,
    /// Remove every keyframe of the property; it then keeps the value it had at `at` (or
    /// `offset`) when one is given, otherwise its plain value.
    #[serde(default)]
    pub all: bool,
}

impl Command for KeyframeRemove {
    const NAME: &'static str = "keyframe.remove";
    const SUMMARY: &'static str = "Remove a keyframe of a clip property at a time, or all of its keyframes (all: true), which stops the animation; with all and a time, the property keeps the value it had then.";
    type Output = KeyframesChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<KeyframesChanged, ApiError> {
        let path = self.property.clone();
        let (_, properties) = with_clip(ctx, &self.clip, |clip, rate| {
            let property = property_of(Self::NAME, clip, &path)?;
            let timed = self.at.is_some() || self.offset.is_some();
            let offset = if timed {
                Some(key_offset(
                    Self::NAME,
                    clip,
                    self.at.as_ref(),
                    self.offset.as_ref(),
                    rate,
                )?)
            } else if self.all {
                None
            } else {
                return Err(invalid(
                    "keyframe.remove: give the keyframe's time (at or offset), or all: true",
                ));
            };
            let Some(keys) = clip.keyframes.get(&path) else {
                return Err(ApiError::new(
                    codes::KEYFRAME_NOT_FOUND,
                    format!("keyframe.remove: {path} of {} has no keyframes", clip.id),
                ));
            };
            if self.all {
                let keep = offset.and_then(|o| fosscut_model::evaluate(keys, o));
                clip.keyframes.remove(&path);
                if let Some(value) = keep {
                    let value = match value {
                        KeyValue::Number(v) => json!(v),
                        KeyValue::Pair(v) => json!(v),
                    };
                    assign(Self::NAME, clip, &property, &value, rate)?;
                }
                return Ok(());
            }
            let offset = offset.unwrap_or(Flicks::ZERO);
            let keys = clip.keyframes.entry(path.clone()).or_default();
            let Some(index) = keys.iter().position(|k| k.time == offset) else {
                return Err(ApiError::new(
                    codes::KEYFRAME_NOT_FOUND,
                    format!(
                        "keyframe.remove: {path} has no keyframe at {}",
                        TimeReport::new(clip.start + offset, rate).timecode
                    ),
                ));
            };
            let removed = keys.remove(index);
            if keys.is_empty() {
                // The last keyframe goes: the property keeps that value.
                clip.keyframes.remove(&path);
                let value = match removed.value {
                    KeyValue::Number(v) => json!(v),
                    KeyValue::Pair(v) => json!(v),
                };
                assign(Self::NAME, clip, &property, &value, rate)?;
            }
            Ok(())
        })?;
        Ok(KeyframesChanged {
            property: self.property,
            clip: properties,
        })
    }
}
