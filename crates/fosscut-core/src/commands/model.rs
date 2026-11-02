//! 3D objects (ADR-0040): `model.add` puts a 3D object clip (a glTF model asset, or a box,
//! sphere, cylinder, plane, torus, extruded text, an extruded path or a grid of blocks, from
//! FossCut's own presets or from fields) on a video track above the footage, and
//! `model.set` changes one by a merge patch. Objects are placed by the clip's 3D transform
//! and seen through the timeline's camera; `model.*` settings animate with `keyframe.set`.

use fosscut_model::{
    AssetObject, Clip, ClipContent, Id, IdKind, KeyValue, MODEL_PROPERTIES, ModelContent,
    ModelObject, PathObject, Project, Track, TrackKind, Transform, model_issues,
};
use fosscut_time::{Flicks, TimeInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ClipPlacement;
use super::caption::merge;
use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, insert_sorted, locate_clip,
    resolve, timeline_id, timeline_mut, track_index,
};
use crate::{ApiError, Command, Context, codes};

const PRESETS: &str = include_str!("../../presets/library/models.json");

/// A built-in 3D object, made by FossCut and free to use anywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelPreset {
    /// Stable name, for example `shipping-box`.
    pub name: String,
    /// Display title.
    pub title: String,
    /// What it looks like and what it suits.
    pub description: String,
    /// Search words.
    pub tags: Vec<String>,
    /// The object.
    pub content: ModelContent,
    /// Where it sits when `model.add` is given no transform: grids lie low and tilt towards
    /// the viewer, for example.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
}

/// Every built-in 3D object preset.
pub fn model_presets() -> Vec<ModelPreset> {
    serde_json::from_str(PRESETS).unwrap_or_default()
}

/// The 3D object preset called `name`.
pub fn model_preset(name: &str) -> Result<ModelPreset, ApiError> {
    model_presets()
        .into_iter()
        .find(|p| p.name == name)
        .ok_or_else(|| {
            let names: Vec<String> = model_presets().into_iter().map(|p| p.name).collect();
            ApiError::new(
                codes::LIBRARY_UNKNOWN_ITEM,
                format!("there is no 3D object preset {name:?}"),
            )
            .with_hint(format!("3D object presets: {}.", names.join(", ")))
        })
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::OBJECT_INVALID, message).with_hint(
        "capabilities lists the 3D object presets under library.models and every object field under models.",
    )
}

/// Refuses an object that breaks the rules of ADR-0040, with the first problem as message.
fn checked(project: &Project, command: &str, content: &ModelContent) -> Result<(), ApiError> {
    match model_issues(project, content).first() {
        Some(issue) if issue.code == "fosscut.model.missing_asset" => Err(ApiError::new(
            codes::ASSET_NOT_FOUND,
            format!("{command}: {}", issue.message),
        )),
        Some(issue) => Err(invalid(format!("{command}: {}", issue.message))),
        None => Ok(()),
    }
}

/// Where an object comes from, besides presets and patches: a model asset, a text, or a
/// shape clip's outline.
struct Sources<'a> {
    asset: Option<&'a Id>,
    text: Option<&'a str>,
    shape_clip: Option<&'a Id>,
}

/// SVG path data and width (a fraction of the canvas width) of the outline of shape clip
/// `clip`, as it is drawn on its own canvas.
fn shape_outline(project: &Project, command: &str, clip: &Id) -> Result<(String, f64), ApiError> {
    let location = locate_clip(project, clip)?;
    let doc = &project.timelines[&location.timeline];
    let found = &doc.tracks[location.track].clips[location.clip];
    let ClipContent::Shape(shape) = &found.content else {
        return Err(invalid(format!(
            "{command}: {clip} is a {} clip, not a shape clip",
            found.content.kind_name()
        )));
    };
    if !shape.closed {
        return Err(invalid(format!(
            "{command}: {clip} is an open path; only closed outlines can be extruded"
        )));
    }
    let (w, h) = (
        f64::from(doc.format.width.max(1)),
        f64::from(doc.format.height.max(1)),
    );
    let outline = shape.outline([shape.size[0] * w, shape.size[1] * h]);
    let width = fosscut_model::fit_path(&outline, 1e-9)
        .map(|fitted| fitted.bounds[2])
        .ok_or_else(|| invalid(format!("{command}: {clip} has no outline")))?;
    Ok((outline.to_svg(), width / w))
}

/// Applies what both commands share to `content`: the sources, then the merge patch. A
/// patch whose `object.kind` differs from the current one starts that object afresh.
fn change(
    project: &Project,
    command: &str,
    content: &mut ModelContent,
    sources: &Sources<'_>,
    set: Option<&Value>,
) -> Result<(), ApiError> {
    if let Some(asset) = sources.asset {
        let mut object: AssetObject = serde_json::from_value(json!({ "asset": asset }))
            .map_err(|e| invalid(format!("{command}: {e}")))?;
        if let ModelObject::Asset(current) = &content.object {
            object.size = current.size;
        }
        content.object = ModelObject::Asset(object);
    }
    if let Some(text) = sources.text {
        match &mut content.object {
            ModelObject::Text(object) => object.text = text.to_string(),
            other => {
                *other = serde_json::from_value(json!({ "kind": "text", "text": text }))
                    .map_err(|e| invalid(format!("{command}: {e}")))?;
            }
        }
    }
    if let Some(clip) = sources.shape_clip {
        let (path, size) = shape_outline(project, command, clip)?;
        match &mut content.object {
            ModelObject::Path(object) => {
                object.path = path;
                object.size = size;
            }
            other => {
                let fresh: PathObject =
                    serde_json::from_value(json!({ "path": path, "size": size }))
                        .map_err(|e| invalid(format!("{command}: {e}")))?;
                *other = ModelObject::Path(fresh);
            }
        }
    }
    if let Some(patch) = set {
        if !patch.is_object() {
            return Err(invalid(format!(
                "{command}: set must be an object of object fields, not {patch}"
            )));
        }
        let mut value = serde_json::to_value(&*content).unwrap_or_else(|_| json!({}));
        let kind = patch.pointer("/object/kind").and_then(Value::as_str);
        if kind.is_some_and(|k| k != content.object.kind_name()) {
            value["object"] = json!({});
        }
        merge(&mut value, patch);
        *content = serde_json::from_value(value).map_err(|e| invalid(format!("{command}: {e}")))?;
    }
    Ok(())
}

/// The video track objects go on: the top video track when it holds only 3D objects and is
/// free over `range`, otherwise a new one above every video track.
fn object_track(
    project: &mut Project,
    timeline: &Id,
    range: (Flicks, Flicks),
) -> Result<Id, ApiError> {
    let doc = timeline_mut(project, timeline)?;
    let top = doc.tracks.iter().rposition(|t| t.kind == TrackKind::Video);
    if let Some(index) = top {
        let track = &doc.tracks[index];
        let objects_only = !track.clips.is_empty()
            && track
                .clips
                .iter()
                .all(|c| matches!(c.content, ClipContent::Model { .. }));
        let free = track
            .clips
            .iter()
            .all(|c| c.end() <= range.0 || c.start >= range.1);
        if objects_only && free && !track.locked {
            return Ok(track.id.clone());
        }
    }
    let id = project.allocate_id(IdKind::Track);
    let doc = timeline_mut(project, timeline)?;
    doc.tracks.insert(
        top.map_or(0, |i| i + 1),
        Track {
            id: id.clone(),
            kind: TrackKind::Video,
            name: Some("3D".into()),
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: Vec::new(),
        },
    );
    Ok(id)
}

/// Adds a 3D object.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelAdd {
    /// A built-in preset to start from: `shipping-box`, `city-blocks`, `podium`,
    /// `extruded-title`, `neon-ring` or `chrome-sphere` (`capabilities` lists them under
    /// library.models). A light grey cube when omitted.
    #[serde(default)]
    pub preset: Option<String>,
    /// A model asset (`.glb` or `.gltf`, imported with media.add) to show.
    #[serde(default)]
    pub asset: Option<Id>,
    /// Text to extrude; replaces an extruded title's text.
    #[serde(default)]
    pub text: Option<String>,
    /// A shape clip whose closed outline is extruded.
    #[serde(default)]
    pub shape_clip: Option<Id>,
    /// Changes to the object as a JSON merge patch over its fields, for example
    /// `{"object": {"kind": "box", "size": [0.3, 0.2, 0.3], "bevel": 0.01}, "material":
    /// {"color": "#D9822B"}, "floor": {}}` (`capabilities` lists the fields under models).
    #[serde(default)]
    pub set: Option<Value>,
    /// Target video track; when omitted, a track of 3D objects above every video track
    /// (reused when free, otherwise made).
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Timeline position; 0 when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// Length; 5 seconds when omitted.
    #[serde(default)]
    pub duration: Option<TimeInput>,
    /// Placement in 3D (position, position_z, rotation, rotation_x, rotation_y, scale,
    /// anchor); objects are always in the timeline's 3D scene. The preset's, or the canvas
    /// centre, when omitted.
    #[serde(default)]
    pub transform: Option<Transform>,
}

impl Command for ModelAdd {
    const NAME: &'static str = "model.add";
    const SUMMARY: &'static str = "Add a 3D object: a glTF model asset (asset), extruded text (text), a shape clip's outline extruded (shape_clip), a box with rounded edges and its own material per face, a sphere, cylinder, plane, torus or a city-like grid of blocks, from FossCut's presets (shipping-box, city-blocks, podium, extruded-title, neon-ring, chrome-sphere) or fields. Materials have a colour, an image printed on them, metallic, roughness, emissive light and opacity; a sky light and a key light with a soft shadow light it; a floor can catch its shadow over footage. It is placed by its 3D transform and seen through the timeline's camera; animate model.light.azimuth, model.emissive_strength, model.depth, model.growth and more with keyframe.set.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let (mut content, preset_transform) = match &self.preset {
            Some(name) => {
                let preset = model_preset(name)?;
                (preset.content, preset.transform)
            }
            None => (ModelContent::default(), None),
        };
        let sources = Sources {
            asset: self.asset.as_ref(),
            text: self.text.as_deref(),
            shape_clip: self.shape_clip.as_ref(),
        };
        change(
            ctx.project,
            Self::NAME,
            &mut content,
            &sources,
            self.set.as_ref(),
        )?;
        checked(ctx.project, Self::NAME, &content)?;
        let start = match &self.start {
            Some(start) => resolve(start, rate)?,
            None => Flicks::ZERO,
        };
        let duration = match &self.duration {
            Some(duration) => resolve(duration, rate)?,
            None => rate.snap(Flicks::from_seconds(5).unwrap_or(Flicks::ZERO)),
        };
        ensure_valid_range(start, duration)?;
        let track = match &self.track {
            Some(track) => track.clone(),
            None => object_track(ctx.project, &timeline, (start, start + duration))?,
        };
        let id = ctx.project.allocate_id(IdKind::Clip);
        let doc = timeline_mut(ctx.project, &timeline)?;
        let index = track_index(doc, &track)?;
        let track_doc = &mut doc.tracks[index];
        let content = ClipContent::Model {
            model: Box::new(content),
        };
        ensure_unlocked(track_doc)?;
        ensure_kind(track_doc, &content)?;
        ensure_free(track_doc, start, start + duration, None, rate)?;
        let clip = Clip {
            transform: self.transform.or(preset_transform).unwrap_or_default(),
            ..Clip::new(id, start, duration, content)
        };
        let placement = ClipPlacement::of(&clip, &track, rate);
        insert_sorted(track_doc, clip);
        Ok(placement)
    }
}

/// Changes a 3D object.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelSet {
    /// The 3D object clip.
    pub clip: Id,
    /// A preset whose object replaces the current one before the other changes apply.
    #[serde(default)]
    pub preset: Option<String>,
    /// A model asset to show instead.
    #[serde(default)]
    pub asset: Option<Id>,
    /// New text to extrude.
    #[serde(default)]
    pub text: Option<String>,
    /// A shape clip whose closed outline is extruded instead.
    #[serde(default)]
    pub shape_clip: Option<Id>,
    /// Changes as a JSON merge patch over the object's fields, for example
    /// `{"light": {"azimuth": 40}, "faces": {"front": {"texture": "asset_0003"}}}`; `null`
    /// resets a field. Settings with keyframes change through keyframe.set instead.
    #[serde(default)]
    pub set: Option<Value>,
}

/// A 3D object after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ModelChanged {
    /// The clip.
    pub clip: Id,
    /// Its object now; defaults are omitted.
    pub content: ModelContent,
}

impl Command for ModelSet {
    const NAME: &'static str = "model.set";
    const SUMMARY: &'static str = "Change a 3D object: start again from a preset, show another model asset, text or shape outline, and change fields by a merge patch (object, material, faces, emissive_strength, light, floor).";
    type Output = ModelChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ModelChanged, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let clip =
            &ctx.project.timelines[&location.timeline].tracks[location.track].clips[location.clip];
        let ClipContent::Model { model: current } = &clip.content else {
            return Err(invalid(format!(
                "model.set: {} is a {} clip, not a 3D object",
                self.clip,
                clip.content.kind_name()
            )));
        };
        let before = current.as_ref().clone();
        let keyed: Vec<String> = clip
            .keyframes
            .keys()
            .filter(|p| p.starts_with("model."))
            .cloned()
            .collect();
        let mut content = match &self.preset {
            Some(name) => model_preset(name)?.content,
            None => before.clone(),
        };
        let sources = Sources {
            asset: self.asset.as_ref(),
            text: self.text.as_deref(),
            shape_clip: self.shape_clip.as_ref(),
        };
        change(
            ctx.project,
            Self::NAME,
            &mut content,
            &sources,
            self.set.as_ref(),
        )?;
        // A setting with keyframes takes its value from them: refuse to change it here.
        for path in &keyed {
            if content.value(path) != before.value(path) {
                return Err(ApiError::new(
                    codes::CLIP_PROPERTY_ANIMATED,
                    format!("model.set: {path} of {} has keyframes", self.clip),
                )
                .with_hint("Set its value at a time with keyframe.set, or remove its keyframes with keyframe.remove (all: true) first."));
            }
        }
        checked(ctx.project, Self::NAME, &content)?;
        let doc = timeline_mut(ctx.project, &location.timeline)?;
        let track = &mut doc.tracks[location.track];
        ensure_unlocked(track)?;
        track.clips[location.clip].content = ClipContent::Model {
            model: Box::new(content.clone()),
        };
        Ok(ModelChanged {
            clip: self.clip,
            content,
        })
    }
}

/// Sets the plain value of a `model.*` property of `clip` for `clip.set` and `keyframe`
/// commands, naming what is missing when the object has no such setting.
pub(crate) fn assign(
    command: &str,
    clip: &mut Clip,
    path: &str,
    value: f64,
) -> Result<(), ApiError> {
    let id = clip.id.clone();
    let ClipContent::Model { model } = &mut clip.content else {
        return Err(invalid(format!(
            "{command}: {path} is a property of 3D object clips only"
        )));
    };
    if model.set_value(path, KeyValue::Number(value)) {
        return Ok(());
    }
    Err(invalid(format!(
        "{command}: {id} has no {path}: {}",
        missing(path)
    )))
}

/// Why an object may lack a `model.*` setting.
pub(crate) fn missing(path: &str) -> &'static str {
    match path {
        "model.depth" => "only extruded text and paths have a depth",
        "model.growth" => "only grids of blocks grow",
        "model.shadow" => "it has no floor (give it one with model.set and floor)",
        _ => "it is not a setting of 3D objects",
    }
}

/// True when `clip` is a 3D object with the setting `path`.
pub(crate) fn has(clip: &Clip, path: &str) -> bool {
    matches!(&clip.content, ClipContent::Model { model } if model.value(path).is_some())
}

/// What `capabilities` says about 3D objects: the presets in brief or in full, and the
/// object kinds, faces, animatable paths, units and limits.
pub(crate) fn model_capabilities(brief: bool) -> (Value, Value) {
    let presets: Vec<Value> = model_presets()
        .into_iter()
        .map(|p| {
            if brief {
                json!({ "name": p.name, "title": p.title, "tags": p.tags })
            } else {
                json!(p)
            }
        })
        .collect();
    let properties: Vec<&str> = MODEL_PROPERTIES.iter().map(|p| p.path).collect();
    let mut reference = json!({
        "kinds": ModelObject::KINDS,
        "faces": fosscut_model::Faces::NAMES,
        "animatable": properties,
        "assets": "Import .glb files, or .gltf files with their buffers and images, with media.add (fosscut media import, media_import); they are copied into the bundle's models/ folder. glTF meshes, node transforms and metallic-roughness materials with base colour textures are drawn; skins, morph targets, embedded animations, cameras and lights in the file are not.",
        "units": "Sizes, depths, bevels, gaps and floor sizes are fractions of the canvas width, the unit of depth of 3D placement; object space has x right, y down and z away from the viewer. The key light's azimuth is degrees around the vertical axis (0 from the viewer's side, 90 from the right), its elevation degrees above the horizon.",
        "placement": "Objects are always in the timeline's 3D scene: transform.position, position_z, rotation, rotation_x, rotation_y, scale, stretch and anchor place them, the timeline's camera sees them, they sort by depth among the 3D clips of neighbouring tracks, and the depth of field blurs them.",
        "limits": {
            "size": fosscut_model::MAX_SIZE,
            "text_characters": fosscut_model::MAX_TEXT_CHARS,
            "grid": fosscut_model::MAX_GRID,
        },
    });
    if !brief {
        reference["content_schema"] =
            serde_json::to_value(schemars::schema_for!(ModelContent)).unwrap_or_default();
    }
    (json!(presets), reference)
}
