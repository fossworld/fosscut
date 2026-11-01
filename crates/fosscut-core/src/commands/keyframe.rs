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
    // Camera clips (ADR-0037) have neither a picture nor sound of their own.
    let picture = !matches!(
        clip.content,
        ClipContent::Audio { .. } | ClipContent::Camera(_)
    );
    // A compound clip sounds like its timeline's mix (ADR-0025), so it has a volume,
    // fades and pan like the clips it holds.
    let sound = matches!(
        clip.content,
        ClipContent::Audio { .. } | ClipContent::Video { .. } | ClipContent::Timeline { .. }
    );
    let has = match property.applies {
        Applies::Picture => picture,
        Applies::Sound => sound,
        Applies::Any => true,
        Applies::Camera => matches!(clip.content, ClipContent::Camera(_)),
        Applies::Particles => matches!(clip.content, ClipContent::Particles { .. }),
        // 3D objects with this setting (ADR-0040).
        Applies::Model => super::model::has(clip, path),
        // Shape outlines and the clips following them (ADR-0036).
        Applies::Shape => matches!(clip.content, ClipContent::Shape(_)),
        Applies::Follower => clip.follow.is_some(),
    };
    if !has {
        let why = if property.applies == Applies::Camera {
            "only camera clips have it"
        } else if matches!(clip.content, ClipContent::Camera(_)) {
            "a camera has no picture or sound"
        } else if property.applies == Applies::Particles {
            "only particle clips have it"
        } else if property.applies == Applies::Model {
            if matches!(clip.content, ClipContent::Model { .. }) {
                super::model::missing(path)
            } else {
                "only 3D object clips have it"
            }
        } else if property.applies == Applies::Shape {
            "only shape clips have it"
        } else if property.applies == Applies::Follower {
            "it follows no path (make it follow one with clip.follow)"
        } else if picture {
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
        // Placement in 3D (ADR-0037).
        "transform.three_d" => t.three_d = flag()?,
        "transform.position_z" => t.position_z = number(command, property, value)?,
        "transform.rotation_x" => t.rotation_x = number(command, property, value)?,
        "transform.rotation_y" => t.rotation_y = number(command, property, value)?,
        "transform.anchor" => t.anchor = pair(command, property, value)?,
        path if path.starts_with("camera.") => {
            // Ranges first, as for every property; then the camera's own checks.
            let value = match property.kind {
                PropertyKind::Number => json!(number(command, property, value)?),
                PropertyKind::Pair => json!(pair(command, property, value)?),
                _ => value.clone(),
            };
            super::camera::assign(command, clip, property, &value)?
        }
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
        path if path.starts_with("particles.") => {
            // Particle emitter settings (ADR-0038).
            let value = match property.kind {
                PropertyKind::Pair => KeyValue::Pair(pair(command, property, value)?),
                _ => KeyValue::Number(number(command, property, value)?),
            };
            let set = match &mut clip.content {
                ClipContent::Particles { particles } => particles.set_value(path, value),
                _ => false,
            };
            if !set {
                return Err(invalid(format!("{command}: {path} cannot be set")));
            }
        }
        path if path.starts_with("model.") => {
            // 3D objects (ADR-0040).
            let v = number(command, property, value)?;
            super::model::assign(command, clip, path, v)?;
        }
        path if path.starts_with("shape.") || path.starts_with("follow.") => {
            // Shape outlines and following (ADR-0036).
            let v = number(command, property, value)?;
            super::vector::assign(command, clip, path, v)?;
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

/// Keyframes of every parameter of `effect` on `clip`, with timeline times.
pub(crate) fn effect_keyframe_reports(
    clip: &Clip,
    effect: &fosscut_model::Effect,
    rate: Rate,
) -> BTreeMap<String, Vec<KeyframeReport>> {
    effect
        .keyframes
        .iter()
        .map(|(name, keys)| {
            let reports = keys
                .iter()
                .map(|k| KeyframeReport {
                    at: TimeReport::new(clip.start + k.time, rate),
                    offset: TimeReport::new(k.time, rate),
                    value: k.value,
                    interpolation: k.interpolation,
                })
                .collect();
            (name.clone(), reports)
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
    /// `volume`, `audio.pan`, `audio.fade_in`, `audio.fade_out`, `audio.channels`; placement
    /// in 3D (ADR-0037): `transform.three_d`, `transform.position_z`, `transform.rotation_x`,
    /// `transform.rotation_y`, `transform.anchor`; on camera clips `camera.<field>`.
    pub set: BTreeMap<String, Value>,
}

impl Command for ClipSet {
    const NAME: &'static str = "clip.set";
    const SUMMARY: &'static str = "Set a clip's properties by path: transform.position, transform.scale, transform.stretch (non-uniform), transform.rotation, transform.flip_horizontal and flip_vertical (mirror), crop.left/right/top/bottom, opacity, blend (multiply, screen, overlay and others), enabled (false deactivates), volume, audio.pan, audio.fade_in, audio.fade_out, audio.channels (fill channel); in 3D transform.three_d (seen through the timeline's camera), transform.position_z, transform.rotation_x, transform.rotation_y and transform.anchor (pivot), and camera.* on camera clips. Position and scale are relative to the canvas and the fitted size. Properties with keyframes are set with keyframe.set instead.";
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
        "transform.position_z" => KeyValue::Number(t.position_z),
        "transform.rotation_x" => KeyValue::Number(t.rotation_x),
        "transform.rotation_y" => KeyValue::Number(t.rotation_y),
        "transform.anchor" => KeyValue::Pair(t.anchor),
        path if path.starts_with("camera.") => return super::camera::plain_value(clip, path),
        "crop.left" => KeyValue::Number(clip.crop.left),
        "crop.right" => KeyValue::Number(clip.crop.right),
        "crop.top" => KeyValue::Number(clip.crop.top),
        "crop.bottom" => KeyValue::Number(clip.crop.bottom),
        "opacity" => KeyValue::Number(clip.opacity),
        "volume" => KeyValue::Number(clip.volume),
        "audio.pan" => KeyValue::Number(clip.audio.pan),
        path if path.starts_with("particles.") => match &clip.content {
            ClipContent::Particles { particles } => return particles.value(path),
            _ => return None,
        },
        path if path.starts_with("model.") => match &clip.content {
            ClipContent::Model { model } => return model.value(path),
            _ => return None,
        },
        path if path.starts_with("shape.") || path.starts_with("follow.") => {
            return super::vector::plain_value(clip, path);
        }
        _ => return None,
    })
}

/// What a keyframe command animates: a clip property, or an effect parameter addressed
/// as `effects.<id>.<param>`.
enum Target {
    Property(Property),
    Effect {
        index: usize,
        param: fosscut_model::ParamSpec,
    },
    Mask {
        index: usize,
        name: &'static str,
        pair: bool,
        range: (f64, f64),
    },
}

/// The value of mask property `name`.
fn mask_value(mask: &fosscut_model::Mask, name: &str) -> KeyValue {
    match name {
        "center" => KeyValue::Pair(mask.center),
        "size" => KeyValue::Pair(mask.size),
        "rotation" => KeyValue::Number(mask.rotation),
        "feather" => KeyValue::Number(mask.feather),
        "expansion" => KeyValue::Number(mask.expansion),
        _ => KeyValue::Number(mask.roundness),
    }
}

impl Target {
    fn of(command: &str, clip: &Clip, path: &str) -> Result<Target, ApiError> {
        if let Some((mask, name)) = fosscut_model::mask_path(path) {
            let index = clip
                .masks
                .iter()
                .position(|m| m.id.as_str() == mask)
                .ok_or_else(|| {
                    ApiError::new(
                        codes::MASK_NOT_FOUND,
                        format!("{command}: {} has no mask {mask}", clip.id),
                    )
                })?;
            let (name, pair, low, high) = fosscut_model::MASK_PROPERTIES
                .iter()
                .find(|(n, ..)| *n == name)
                .copied()
                .ok_or_else(|| {
                    invalid(format!(
                        "{command}: masks animate center, size, rotation, feather, expansion and roundness, not {name}"
                    ))
                })?;
            return Ok(Target::Mask {
                index,
                name,
                pair,
                range: (low, high),
            });
        }
        let Some((effect, name)) = fosscut_model::effect_path(path) else {
            return Ok(Target::Property(property_of(command, clip, path)?));
        };
        let index = clip
            .effects
            .iter()
            .position(|e| e.id.as_str() == effect)
            .ok_or_else(|| {
                ApiError::new(
                    codes::EFFECT_NOT_FOUND,
                    format!("{command}: {} has no effect {effect}", clip.id),
                )
            })?;
        let spec = super::effect::spec_of(command, &clip.effects[index].effect_type)?;
        let param = spec.param(name).cloned().ok_or_else(|| {
            let names: Vec<&str> = spec.params.iter().map(|p| p.name).collect();
            invalid(format!("{command}: {} has no parameter {name}", spec.name))
                .with_hint(format!("Parameters: {}.", names.join(", ")))
        })?;
        Ok(Target::Effect { index, param })
    }

    fn animatable(&self) -> bool {
        match self {
            Target::Property(p) => p.animatable,
            Target::Effect { param, .. } => param.animatable,
            Target::Mask { .. } => true,
        }
    }

    /// The name keyframes are stored under, in the map [`Target::keys`] returns.
    fn key_name(&self) -> String {
        match self {
            Target::Property(p) => p.path.to_string(),
            Target::Effect { param, .. } => param.name.to_string(),
            Target::Mask { name, .. } => name.to_string(),
        }
    }

    fn keys<'a>(&self, clip: &'a mut Clip) -> &'a mut BTreeMap<String, Vec<Keyframe>> {
        match self {
            Target::Property(_) => &mut clip.keyframes,
            Target::Effect { index, .. } => &mut clip.effects[*index].keyframes,
            Target::Mask { index, .. } => &mut clip.masks[*index].keyframes,
        }
    }

    fn keys_of<'a>(&self, clip: &'a Clip) -> Option<&'a Vec<Keyframe>> {
        match self {
            Target::Property(p) => clip.keyframes.get(p.path),
            Target::Effect { index, param } => clip.effects[*index].keyframes.get(param.name),
            Target::Mask { index, name, .. } => clip.masks[*index].keyframes.get(*name),
        }
    }

    fn parse(&self, command: &str, value: &Value) -> Result<KeyValue, ApiError> {
        match self {
            Target::Property(p) => key_value(command, p, value),
            Target::Effect { param, .. } => {
                let as_property = Property {
                    path: param.name,
                    kind: match param.kind {
                        fosscut_model::ParamKind::Pair => PropertyKind::Pair,
                        _ => PropertyKind::Number,
                    },
                    applies: Applies::Picture,
                    animatable: true,
                    min: param.min,
                    max: param.max,
                    choices: &[],
                    meaning: param.meaning,
                };
                key_value(command, &as_property, value)
            }
            Target::Mask {
                name, pair, range, ..
            } => {
                let as_property = Property {
                    path: name,
                    kind: if *pair {
                        PropertyKind::Pair
                    } else {
                        PropertyKind::Number
                    },
                    applies: Applies::Picture,
                    animatable: true,
                    min: range.0.is_finite().then_some(range.0),
                    max: range.1.is_finite().then_some(range.1),
                    choices: &[],
                    meaning: "",
                };
                key_value(command, &as_property, value)
            }
        }
    }

    /// The value without keyframes.
    fn plain(&self, clip: &Clip) -> Option<KeyValue> {
        match self {
            Target::Property(p) => plain_value(clip, p.path),
            Target::Effect { index, param } => {
                let effect = &clip.effects[*index];
                match effect
                    .params
                    .get(param.name)
                    .cloned()
                    .unwrap_or_else(|| param.default.value())
                {
                    fosscut_model::ParamValue::Number(v) => Some(KeyValue::Number(v)),
                    fosscut_model::ParamValue::Pair(v) => Some(KeyValue::Pair(v)),
                    _ => None,
                }
            }
            Target::Mask { index, name, .. } => Some(mask_value(&clip.masks[*index], name)),
        }
    }

    /// Makes `value` the plain value, once the keyframes are gone.
    fn assign(
        &self,
        command: &str,
        clip: &mut Clip,
        value: KeyValue,
        rate: Rate,
    ) -> Result<(), ApiError> {
        let json = match value {
            KeyValue::Number(v) => json!(v),
            KeyValue::Pair(v) => json!(v),
        };
        match self {
            Target::Property(p) => assign(command, clip, p, &json, rate),
            Target::Effect { index, param } => {
                let value = fosscut_model::ParamValue::from(value);
                let params = &mut clip.effects[*index].params;
                if value == param.default.value() {
                    params.remove(param.name);
                } else {
                    params.insert(param.name.to_string(), value);
                }
                Ok(())
            }
            Target::Mask { index, name, .. } => {
                let mask = &mut clip.masks[*index];
                match (*name, value) {
                    ("center", KeyValue::Pair(v)) => mask.center = v,
                    ("size", KeyValue::Pair(v)) => mask.size = v,
                    ("rotation", KeyValue::Number(v)) => mask.rotation = v,
                    ("feather", KeyValue::Number(v)) => mask.feather = v,
                    ("expansion", KeyValue::Number(v)) => mask.expansion = v,
                    ("roundness", KeyValue::Number(v)) => mask.roundness = v,
                    _ => {}
                }
                Ok(())
            }
        }
    }
}

/// Adds a keyframe to a clip property, or replaces the one at that time.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyframeSet {
    /// The clip.
    pub clip: Id,
    /// An animatable property: `transform.position`, `transform.scale`,
    /// `transform.stretch`, `transform.rotation`, `transform.position_z`,
    /// `transform.rotation_x`, `transform.rotation_y`, `transform.anchor`, a camera clip's
    /// `camera.<field>` (ADR-0037), `crop.left`, `crop.right`, `crop.top`,
    /// `crop.bottom`, `opacity`, `volume` or `audio.pan`; a shape clip's `shape.trim_start`,
    /// `shape.trim_end`, `shape.trim_offset` or `shape.stroke_width`, or a following clip's
    /// `follow.progress` (ADR-0036); a numeric effect parameter as
    /// `effects.<effect id>.<param>` (for example `effects.effect_0004.exposure`); or a mask's
    /// `center`, `size`, `rotation`, `feather`, `expansion` or `roundness` as
    /// `masks.<mask id>.<property>`.
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
    const SUMMARY: &'static str = "Animate a clip property: add a keyframe at a time (or replace the one there) with a value and an easing (linear, hold, ease_in, ease_out, ease_in_out or a Bezier). Between keyframes the value moves along the easing; before the first and after the last it holds. Animatable: transform.position, transform.scale, transform.stretch, transform.rotation, crop edges, opacity, volume, audio.pan, numeric effect parameters as effects.<id>.<param>, and mask placement as masks.<id>.<property>.";
    type Output = KeyframesChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<KeyframesChanged, ApiError> {
        let easing = self
            .easing
            .as_ref()
            .map(Easing::interpolation)
            .transpose()?;
        let path = self.property.clone();
        let (_, properties) = with_clip(ctx, &self.clip, |clip, rate| {
            let target = Target::of(Self::NAME, clip, &path)?;
            if !target.animatable() {
                return Err(invalid(format!(
                    "keyframe.set: {path} does not take keyframes; set it with clip.set or effect.set"
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
                Some(value) => target.parse(Self::NAME, value)?,
                None => target
                    .keys_of(clip)
                    .and_then(|keys| fosscut_model::evaluate(keys, offset))
                    .or_else(|| target.plain(clip))
                    .ok_or_else(|| invalid(format!("keyframe.set: {path} has no value")))?,
            };
            let keys = target.keys(clip).entry(target.key_name()).or_default();
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

/// One keyframe of `keyframe.set_all`.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyInput {
    /// Position from the clip's start.
    pub offset: TimeInput,
    /// The value: a number, or `[x, y]` for pairs.
    pub value: Value,
    /// How the value moves to the next keyframe; linear when omitted.
    #[serde(default)]
    pub easing: Option<Easing>,
}

/// Replaces every keyframe of a clip property at once: how measured motion (stabilization,
/// tracking) lands on a clip, one undo step however many keyframes it has.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyframeSetAll {
    /// The clip.
    pub clip: Id,
    /// The property, as `keyframe.set` takes it.
    pub property: String,
    /// The keyframes, at different offsets; an empty list removes them all.
    pub keys: Vec<KeyInput>,
}

impl Command for KeyframeSetAll {
    const NAME: &'static str = "keyframe.set_all";
    const SUMMARY: &'static str = "Replace every keyframe of a clip property with a list of offsets from the clip's start and values, in one step: for motion measured by stabilization or tracking, or any animation planned whole.";
    type Output = KeyframesChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<KeyframesChanged, ApiError> {
        let path = self.property.clone();
        let (_, properties) = with_clip(ctx, &self.clip, |clip, rate| {
            let target = Target::of(Self::NAME, clip, &path)?;
            if !target.animatable() {
                return Err(invalid(format!(
                    "keyframe.set_all: {path} does not take keyframes; set it with clip.set or effect.set"
                )));
            }
            let mut keys: Vec<Keyframe> = Vec::with_capacity(self.keys.len());
            for key in &self.keys {
                let offset = key_offset(Self::NAME, clip, None, Some(&key.offset), rate)?;
                keys.push(Keyframe {
                    time: offset,
                    value: target.parse(Self::NAME, &key.value)?,
                    interpolation: key
                        .easing
                        .as_ref()
                        .map(Easing::interpolation)
                        .transpose()?
                        .unwrap_or_default(),
                });
            }
            keys.sort_by_key(|k| k.time);
            if keys.windows(2).any(|w| w[0].time == w[1].time) {
                return Err(invalid(
                    "keyframe.set_all: two keyframes fall on the same frame",
                ));
            }
            let store = target.keys(clip);
            if keys.is_empty() {
                store.remove(&target.key_name());
            } else {
                store.insert(target.key_name(), keys);
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
            let target = Target::of(Self::NAME, clip, &path)?;
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
            let Some(keys) = target.keys_of(clip) else {
                return Err(ApiError::new(
                    codes::KEYFRAME_NOT_FOUND,
                    format!("keyframe.remove: {path} of {} has no keyframes", clip.id),
                ));
            };
            if self.all {
                let keep = offset.and_then(|o| fosscut_model::evaluate(keys, o));
                target.keys(clip).remove(&target.key_name());
                if let Some(value) = keep {
                    target.assign(Self::NAME, clip, value, rate)?;
                }
                return Ok(());
            }
            let offset = offset.unwrap_or(Flicks::ZERO);
            let name = target.key_name();
            let keys = target.keys(clip).entry(name.clone()).or_default();
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
                target.keys(clip).remove(&name);
                target.assign(Self::NAME, clip, removed.value, rate)?;
            }
            Ok(())
        })?;
        Ok(KeyframesChanged {
            property: self.property,
            clip: properties,
        })
    }
}
