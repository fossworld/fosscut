//! Effects on clips (ADR-0011, items 1 and 2): add, change, remove, reorder and copy
//! effect instances, add adjustment layers, and apply built-in looks. Effect parameters
//! are checked against the type's schema here, with messages that name the parameter;
//! keyframes on them go through `keyframe.set` as `effects.<id>.<param>`.

use std::collections::BTreeMap;

use fosscut_model::{
    AssetKind, AssetRef, Clip, ClipContent, Effect, EffectSpec, Id, IdKind, Media, ParamDefault,
    ParamKind, ParamSpec, ParamValue, Project, Track, TrackKind, effect_type, effect_types,
};
use fosscut_time::TimeInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ClipPlacement;
use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, insert_sorted, locate_clip,
    resolve, timeline_id, timeline_mut, track_index,
};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// The spec of `name`, or an error listing the types.
pub(crate) fn spec_of(command: &str, name: &str) -> Result<EffectSpec, ApiError> {
    effect_type(name).ok_or_else(|| {
        let names: Vec<&str> = effect_types().iter().map(|t| t.name).collect();
        ApiError::new(
            codes::EFFECT_UNKNOWN_TYPE,
            format!("{command}: there is no effect type {name}"),
        )
        .with_hint(format!(
            "Types: {}. capabilities lists them with their parameters under effects.",
            names.join(", ")
        ))
    })
}

/// A parameter value from JSON, checked against its spec.
pub(crate) fn param_value(
    command: &str,
    spec: &EffectSpec,
    param: &ParamSpec,
    value: &Value,
    media: &Media,
) -> Result<ParamValue, ApiError> {
    let name = format!("{}.{}", spec.name, param.name);
    let range = |v: f64| -> Result<f64, ApiError> {
        let ok =
            v.is_finite() && param.min.is_none_or(|m| v >= m) && param.max.is_none_or(|m| v <= m);
        if ok {
            Ok(v)
        } else {
            Err(invalid(format!(
                "{command}: {name} must be from {} to {}, not {v}",
                param.min.unwrap_or(f64::NEG_INFINITY),
                param.max.unwrap_or(f64::INFINITY)
            )))
        }
    };
    let number = |v: &Value| -> Result<f64, ApiError> {
        v.as_f64()
            .ok_or_else(|| invalid(format!("{command}: {name} must be a number, not {v}")))
            .and_then(range)
    };
    match param.kind {
        ParamKind::Number => Ok(ParamValue::Number(number(value)?)),
        ParamKind::Pair => {
            let items = value
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or_else(|| invalid(format!("{command}: {name} must be [x, y], not {value}")))?;
            Ok(ParamValue::Pair([number(&items[0])?, number(&items[1])?]))
        }
        ParamKind::Bool => value
            .as_bool()
            .map(ParamValue::Flag)
            .ok_or_else(|| invalid(format!("{command}: {name} must be true or false"))),
        ParamKind::Choice => value
            .as_str()
            .filter(|n| param.choices.contains(n))
            .map(|n| ParamValue::Name(n.to_string()))
            .ok_or_else(|| {
                invalid(format!(
                    "{command}: {name} must be one of {}",
                    param.choices.join(", ")
                ))
            }),
        ParamKind::Curve => {
            let points = value
                .as_array()
                .filter(|a| a.len() >= 2)
                .ok_or_else(|| {
                    invalid(format!(
                        "{command}: {name} must be at least two points [[x, y], ...]"
                    ))
                })?
                .iter()
                .map(|p| {
                    let pair = p.as_array().filter(|a| a.len() == 2).ok_or_else(|| {
                        invalid(format!(
                            "{command}: each point of {name} is [x, y], not {p}"
                        ))
                    })?;
                    Ok([number(&pair[0])?, number(&pair[1])?])
                })
                .collect::<Result<Vec<[f64; 2]>, ApiError>>()?;
            if points.windows(2).any(|w| w[1][0] <= w[0][0]) {
                return Err(invalid(format!(
                    "{command}: the points of {name} must be in order of x, each x once"
                )));
            }
            Ok(ParamValue::Points(points))
        }
        ParamKind::Asset => {
            let asset: Id = match value {
                Value::String(text) => text.parse().map_err(|e: String| invalid(e))?,
                other => {
                    serde_json::from_value::<AssetRef>(other.clone())
                        .map_err(|_| {
                            invalid(format!(
                                "{command}: {name} is an asset: {{\"asset\": \"asset_…\"}}"
                            ))
                        })?
                        .asset
                }
            };
            let found = media.get(&asset).ok_or_else(|| {
                ApiError::new(
                    codes::ASSET_NOT_FOUND,
                    format!("{command}: asset {asset} does not exist"),
                )
            })?;
            if param.choices.contains(&"lut") && found.kind != AssetKind::Lut {
                return Err(invalid(format!(
                    "{command}: {name} needs a LUT asset; {asset} is {:?}",
                    found.kind
                ))
                .with_hint("Import a .cube file with media_import first."));
            }
            Ok(ParamValue::Asset(AssetRef { asset }))
        }
    }
}

/// Sets `params` on `effect`, keeping the parameter map sparse.
fn set_params(
    command: &str,
    spec: &EffectSpec,
    effect: &mut Effect,
    params: &BTreeMap<String, Value>,
    media: &Media,
) -> Result<(), ApiError> {
    for (name, value) in params {
        let param = spec.param(name).ok_or_else(|| {
            let names: Vec<&str> = spec.params.iter().map(|p| p.name).collect();
            invalid(format!("{command}: {} has no parameter {name}", spec.name))
                .with_hint(format!("Parameters: {}.", names.join(", ")))
        })?;
        if effect.keyframes.contains_key(name) {
            return Err(ApiError::new(
                codes::CLIP_PROPERTY_ANIMATED,
                format!("{command}: {}.{name} has keyframes", spec.name),
            )
            .with_hint(format!(
                "Set its value at a time with keyframe.set (property effects.{}.{name}), or remove its keyframes first.",
                effect.id
            )));
        }
        let value = param_value(command, spec, param, value, media)?;
        if !matches!(param.default, ParamDefault::Required(_)) && value == param.default.value() {
            effect.params.remove(name);
        } else {
            effect.params.insert(name.clone(), value);
        }
    }
    Ok(())
}

/// An effect as commands report it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EffectReport {
    /// Effect id.
    pub effect: Id,
    /// Effect type.
    #[serde(rename = "type")]
    pub effect_type: String,
    /// Whether it applies.
    pub enabled: bool,
    /// Parameters that differ from their defaults.
    pub params: BTreeMap<String, ParamValue>,
}

/// A clip's effects after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EffectsChanged {
    /// The clip.
    pub clip: Id,
    /// The effect the command added or changed, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<Id>,
    /// Every effect of the clip in the order they apply.
    pub effects: Vec<EffectReport>,
}

impl EffectsChanged {
    fn of(clip: &Clip, effect: Option<Id>) -> EffectsChanged {
        EffectsChanged {
            clip: clip.id.clone(),
            effect,
            effects: clip
                .effects
                .iter()
                .map(|e| EffectReport {
                    effect: e.id.clone(),
                    effect_type: e.effect_type.clone(),
                    enabled: e.enabled,
                    params: e.params.clone(),
                })
                .collect(),
        }
    }
}

/// Runs `edit` on a clip with a picture, after checking its track is unlocked.
fn with_picture<T>(
    ctx: &mut Context<'_>,
    command: &str,
    id: &Id,
    edit: impl FnOnce(&mut Clip, &Media) -> Result<T, ApiError>,
) -> Result<(T, EffectsChanged), ApiError> {
    let location = locate_clip(ctx.project, id)?;
    let snapshot = ctx.project.media.clone();
    let timeline = timeline_mut(ctx.project, &location.timeline)?;
    let track = &mut timeline.tracks[location.track];
    ensure_unlocked(track)?;
    let clip = &mut track.clips[location.clip];
    if matches!(clip.content, ClipContent::Audio { .. }) {
        return Err(invalid(format!(
            "{command}: {id} is an audio clip; effects change pictures"
        )));
    }
    let result = edit(clip, &snapshot)?;
    let changed = EffectsChanged::of(clip, None);
    Ok((result, changed))
}

fn effect_index(command: &str, clip: &Clip, effect: &Id) -> Result<usize, ApiError> {
    clip.effects
        .iter()
        .position(|e| &e.id == effect)
        .ok_or_else(|| {
            ApiError::new(
                codes::EFFECT_NOT_FOUND,
                format!("{command}: {} has no effect {effect}", clip.id),
            )
            .with_hint("Read the clip's effects with timeline.read.")
        })
}

/// Adds an effect to a clip.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectAdd {
    /// The clip: a video, image, text, caption or adjustment clip.
    pub clip: Id,
    /// The effect type, such as `color.adjust` (capabilities lists them under effects).
    #[serde(rename = "type")]
    pub effect_type: String,
    /// Parameter values; the rest keep their defaults.
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    /// Position in the clip's effects (0 applies first); last when omitted.
    #[serde(default)]
    pub index: Option<usize>,
}

impl Command for EffectAdd {
    const NAME: &'static str = "effect.add";
    const SUMMARY: &'static str = "Add an effect to a clip's picture: color.adjust (exposure, white balance, contrast, tone, saturation), color.curves, color.hsl, color.wheels, color.lut, color.vignette, blur.gaussian, detail.sharpen, stylize.grain. Effects apply in order; on an adjustment clip they apply to everything below it.";
    type Output = EffectsChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<EffectsChanged, ApiError> {
        let spec = spec_of(Self::NAME, &self.effect_type)?;
        let id = ctx.project.allocate_id(IdKind::Effect);
        let added = id.clone();
        let (_, mut changed) = with_picture(ctx, Self::NAME, &self.clip, |clip, media| {
            let mut effect = Effect {
                id,
                effect_type: spec.name.to_string(),
                version: spec.version,
                enabled: true,
                params: BTreeMap::new(),
                keyframes: BTreeMap::new(),
            };
            set_params(Self::NAME, &spec, &mut effect, &self.params, media)?;
            for param in &spec.params {
                if matches!(param.default, ParamDefault::Required(_))
                    && !effect.params.contains_key(param.name)
                {
                    return Err(
                        invalid(format!("effect.add: {} needs {}", spec.name, param.name))
                            .with_hint(param.meaning),
                    );
                }
            }
            let index = self.index.unwrap_or(clip.effects.len());
            if index > clip.effects.len() {
                return Err(invalid(format!(
                    "effect.add: index {index} is beyond the clip's {} effects",
                    clip.effects.len()
                )));
            }
            clip.effects.insert(index, effect);
            Ok(())
        })?;
        changed.effect = Some(added);
        Ok(changed)
    }
}

/// Changes an effect's parameters or turns it on or off.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectSet {
    /// The clip.
    pub clip: Id,
    /// The effect (`effect_…`).
    pub effect: Id,
    /// New parameter values.
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    /// Parameters to return to their defaults.
    #[serde(default)]
    pub reset: Vec<String>,
    /// false bypasses the effect, true applies it again.
    #[serde(default)]
    pub enabled: Option<bool>,
}

impl Command for EffectSet {
    const NAME: &'static str = "effect.set";
    const SUMMARY: &'static str = "Change an effect's parameters, reset some to their defaults, or bypass it (enabled: false). Parameters with keyframes are set with keyframe.set as effects.<id>.<param>.";
    type Output = EffectsChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<EffectsChanged, ApiError> {
        if self.params.is_empty() && self.reset.is_empty() && self.enabled.is_none() {
            return Err(invalid(
                "effect.set: give params to change, reset, or enabled",
            ));
        }
        let effect_id = self.effect.clone();
        let (_, mut changed) = with_picture(ctx, Self::NAME, &self.clip, |clip, media| {
            let index = effect_index(Self::NAME, clip, &self.effect)?;
            let effect = &mut clip.effects[index];
            let spec = spec_of(Self::NAME, &effect.effect_type)?;
            set_params(Self::NAME, &spec, effect, &self.params, media)?;
            for name in &self.reset {
                let param = spec.param(name).ok_or_else(|| {
                    invalid(format!("effect.set: {} has no parameter {name}", spec.name))
                })?;
                if matches!(param.default, ParamDefault::Required(_)) {
                    return Err(invalid(format!(
                        "effect.set: {}.{name} has no default to reset to",
                        spec.name
                    )));
                }
                effect.params.remove(name);
                effect.keyframes.remove(name);
            }
            if let Some(enabled) = self.enabled {
                effect.enabled = enabled;
            }
            Ok(())
        })?;
        changed.effect = Some(effect_id);
        Ok(changed)
    }
}

/// Removes an effect from a clip.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectRemove {
    /// The clip.
    pub clip: Id,
    /// The effect.
    pub effect: Id,
}

impl Command for EffectRemove {
    const NAME: &'static str = "effect.remove";
    const SUMMARY: &'static str = "Remove an effect and its keyframes from a clip.";
    type Output = EffectsChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<EffectsChanged, ApiError> {
        let (_, changed) = with_picture(ctx, Self::NAME, &self.clip, |clip, _| {
            let index = effect_index(Self::NAME, clip, &self.effect)?;
            clip.effects.remove(index);
            Ok(())
        })?;
        Ok(changed)
    }
}

/// Moves an effect to another place in the clip's order.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectMove {
    /// The clip.
    pub clip: Id,
    /// The effect.
    pub effect: Id,
    /// New position: 0 applies first.
    pub index: usize,
}

impl Command for EffectMove {
    const NAME: &'static str = "effect.move";
    const SUMMARY: &'static str =
        "Move an effect to another position in its clip's order; effects apply first to last.";
    type Output = EffectsChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<EffectsChanged, ApiError> {
        let effect_id = self.effect.clone();
        let (_, mut changed) = with_picture(ctx, Self::NAME, &self.clip, |clip, _| {
            let from = effect_index(Self::NAME, clip, &self.effect)?;
            if self.index >= clip.effects.len() {
                return Err(invalid(format!(
                    "effect.move: index {} is beyond the clip's {} effects",
                    self.index,
                    clip.effects.len()
                )));
            }
            let effect = clip.effects.remove(from);
            clip.effects.insert(self.index, effect);
            Ok(())
        })?;
        changed.effect = Some(effect_id);
        Ok(changed)
    }
}

/// Copies one clip's effects to other clips.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectCopy {
    /// The clip whose effects are copied.
    pub from: Id,
    /// The clips that receive them.
    pub to: Vec<Id>,
    /// Add after the receivers' own effects instead of replacing them.
    #[serde(default)]
    pub append: bool,
}

/// Result of `effect.copy`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EffectsCopied {
    /// Every receiving clip with its effects now.
    pub clips: Vec<EffectsChanged>,
}

impl Command for EffectCopy {
    const NAME: &'static str = "effect.copy";
    const SUMMARY: &'static str = "Apply one clip's effects (with their keyframes) to other clips, replacing theirs or, with append, after them: the way to grade every clip alike.";
    type Output = EffectsCopied;

    fn apply(self, ctx: &mut Context<'_>) -> Result<EffectsCopied, ApiError> {
        if self.to.is_empty() {
            return Err(invalid("effect.copy: give at least one clip in to"));
        }
        let source = locate_clip(ctx.project, &self.from)?;
        let effects = ctx.project.timelines[&source.timeline].tracks[source.track].clips
            [source.clip]
            .effects
            .clone();
        let mut clips = Vec::with_capacity(self.to.len());
        for target in &self.to {
            if target == &self.from {
                return Err(invalid(
                    "effect.copy: a clip cannot receive its own effects",
                ));
            }
            let mut copies = effects.clone();
            for effect in &mut copies {
                effect.id = ctx.project.allocate_id(IdKind::Effect);
            }
            let append = self.append;
            let (_, changed) = with_picture(ctx, Self::NAME, target, |clip, _| {
                if !append {
                    clip.effects.clear();
                }
                clip.effects.extend(copies);
                Ok(())
            })?;
            clips.push(changed);
        }
        Ok(EffectsCopied { clips })
    }
}

/// An effect to create: its type and parameters.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectInput {
    /// The effect type.
    #[serde(rename = "type")]
    pub effect_type: String,
    /// Parameter values.
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

fn build_effects(
    command: &str,
    project: &mut Project,
    inputs: &[EffectInput],
) -> Result<Vec<Effect>, ApiError> {
    let mut effects = Vec::with_capacity(inputs.len());
    for input in inputs {
        let spec = spec_of(command, &input.effect_type)?;
        let mut effect = Effect {
            id: project.allocate_id(IdKind::Effect),
            effect_type: spec.name.to_string(),
            version: spec.version,
            enabled: true,
            params: BTreeMap::new(),
            keyframes: BTreeMap::new(),
        };
        set_params(command, &spec, &mut effect, &input.params, &project.media)?;
        effects.push(effect);
    }
    Ok(effects)
}

/// The track an adjustment clip goes on by default: the top video track when it holds
/// only adjustment clips and the range is free, otherwise a new video track above every
/// video track, so the adjustment covers all pictures but not titles and captions.
fn adjustment_track(
    project: &mut Project,
    timeline: &Id,
    range: (fosscut_time::Flicks, fosscut_time::Flicks),
) -> Result<Id, ApiError> {
    let doc = timeline_mut(project, timeline)?;
    let top = doc.tracks.iter().rposition(|t| t.kind == TrackKind::Video);
    if let Some(index) = top {
        let track = &doc.tracks[index];
        let only_adjustments = !track.clips.is_empty()
            && track
                .clips
                .iter()
                .all(|c| matches!(c.content, ClipContent::Adjustment));
        let free = track
            .clips
            .iter()
            .all(|c| c.end() <= range.0 || c.start >= range.1);
        if only_adjustments && free && !track.locked {
            return Ok(track.id.clone());
        }
    }
    let id = project.allocate_id(IdKind::Track);
    let doc = timeline_mut(project, timeline)?;
    let index = top.map_or(0, |i| i + 1);
    doc.tracks.insert(
        index,
        Track {
            id: id.clone(),
            kind: TrackKind::Video,
            name: Some("Adjustments".into()),
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: Vec::new(),
        },
    );
    Ok(id)
}

/// Adds an adjustment layer: a clip whose effects apply to every picture below it.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdjustmentAdd {
    /// Where it starts.
    pub start: TimeInput,
    /// How long it lasts.
    pub duration: TimeInput,
    /// Effects to start with, applied in order.
    #[serde(default)]
    pub effects: Vec<EffectInput>,
    /// A video track; by default an adjustment track above every video track (created
    /// when needed), so titles and captions stay as they are.
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
}

/// Result of `adjustment.add`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AdjustmentAdded {
    /// Where the adjustment clip is.
    #[serde(flatten)]
    pub placement: ClipPlacement,
    /// Its effects.
    pub effects: Vec<Id>,
}

impl Command for AdjustmentAdd {
    const NAME: &'static str = "adjustment.add";
    const SUMMARY: &'static str = "Add an adjustment layer: a clip on a video track whose effects (and opacity) apply to every picture below it for its time range, such as one grade over a whole sequence. Add effects with effects here or effect.add later.";
    type Output = AdjustmentAdded;

    fn apply(self, ctx: &mut Context<'_>) -> Result<AdjustmentAdded, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let start = resolve(&self.start, rate)?;
        let duration = resolve(&self.duration, rate)?;
        ensure_valid_range(start, duration)?;
        let effects = build_effects(Self::NAME, ctx.project, &self.effects)?;
        let track = match &self.track {
            Some(track) => track.clone(),
            None => adjustment_track(ctx.project, &timeline, (start, start + duration))?,
        };
        let id = ctx.project.allocate_id(IdKind::Clip);
        let doc = timeline_mut(ctx.project, &timeline)?;
        let index = track_index(doc, &track)?;
        let track_doc = &mut doc.tracks[index];
        ensure_unlocked(track_doc)?;
        ensure_kind(track_doc, &ClipContent::Adjustment)?;
        ensure_free(track_doc, start, start + duration, None, rate)?;
        let ids = effects.iter().map(|e| e.id.clone()).collect();
        let clip = Clip {
            effects,
            ..Clip::new(id, start, duration, ClipContent::Adjustment)
        };
        let placement = ClipPlacement::of(&clip, &track, rate);
        insert_sorted(track_doc, clip);
        Ok(AdjustmentAdded {
            placement,
            effects: ids,
        })
    }
}

/// Applies a built-in look: a set of effects with parameters chosen for a style.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LookApply {
    /// The clip (an adjustment clip grades everything below it).
    pub clip: Id,
    /// The look's name (capabilities lists them under looks).
    pub look: String,
    /// Replace the clip's effects instead of adding after them.
    #[serde(default)]
    pub replace: bool,
}

impl Command for LookApply {
    const NAME: &'static str = "look.apply";
    const SUMMARY: &'static str = "Apply a built-in look (a filter such as warm, cool, teal_orange, vintage, noir, faded, vivid) as ordinary effects the clip owns, so it can be adjusted afterwards.";
    type Output = EffectsChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<EffectsChanged, ApiError> {
        let look = crate::looks::look(&self.look)?;
        let inputs: Vec<EffectInput> = look
            .effects
            .iter()
            .map(|e| EffectInput {
                effect_type: e.effect_type.clone(),
                params: e.params.clone(),
            })
            .collect();
        let effects = build_effects(Self::NAME, ctx.project, &inputs)?;
        let replace = self.replace;
        let (_, changed) = with_picture(ctx, Self::NAME, &self.clip, |clip, _| {
            if replace {
                clip.effects.clear();
            }
            clip.effects.extend(effects);
            Ok(())
        })?;
        Ok(changed)
    }
}
