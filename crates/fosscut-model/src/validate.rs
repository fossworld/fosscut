use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Animator, AssetKind, CaptionContent, CaptionStyle, Clip, ClipContent, Effect, Id, IdKind,
    KeyValue, ParamKind, ParamValue, Project, PropertyKind, ShapeContent, TextContent, TextStyle,
    Track, TrackKind, clip_property, effect_type,
};

/// One rule violation found by [`validate`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Issue {
    /// Where the problem is, for example `timeline_0000/track_0001/clip_0002`.
    pub path: String,
    /// Stable code such as `fosscut.model.overlap`.
    pub code: String,
    /// Human-readable description.
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} ({})", self.path, self.message, self.code)
    }
}

struct Issues(Vec<Issue>);

impl Issues {
    fn push(&mut self, path: impl Into<String>, code: &str, message: impl Into<String>) {
        self.0.push(Issue {
            path: path.into(),
            code: code.to_string(),
            message: message.into(),
        });
    }
}

fn expect_kind(issues: &mut Issues, id: &Id, kind: IdKind, path: &str) {
    if !id.is(kind) {
        issues.push(
            path,
            "fosscut.model.id_kind",
            format!("{id} should be a {} id", kind.prefix()),
        );
    }
}

/// Checks every format rule of ADR-0004 that types alone cannot express.
pub fn validate(project: &Project) -> Vec<Issue> {
    let mut issues = Issues(Vec::new());
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut unique = |issues: &mut Issues, id: &Id, path: &str| {
        if !seen.insert(id.to_string()) {
            issues.push(
                path,
                "fosscut.model.duplicate_id",
                format!("id {id} is used more than once"),
            );
        }
    };

    let manifest = &project.manifest;
    expect_kind(&mut issues, &manifest.id, IdKind::Project, "project");
    if !project.timelines.contains_key(&manifest.primary_timeline) {
        issues.push(
            "project",
            "fosscut.model.missing_timeline",
            format!(
                "primary timeline {} does not exist",
                manifest.primary_timeline
            ),
        );
    }
    let listed: BTreeSet<&Id> = manifest.timelines.iter().map(|t| &t.id).collect();
    let stored: BTreeSet<&Id> = project.timelines.keys().collect();
    if listed != stored {
        issues.push(
            "project",
            "fosscut.model.timeline_list",
            "manifest timeline list does not match timeline documents",
        );
    }

    // Saved text styles have unique names in order, and valid styles (ADR-0027).
    let families = project.media.font_families();
    let presets = &manifest.text_presets;
    if !presets.windows(2).all(|w| w[0].name < w[1].name)
        || presets.iter().any(|p| p.name.trim().is_empty())
    {
        issues.push(
            "project",
            "fosscut.model.text_preset",
            "text presets need names, unique and sorted",
        );
    }
    for preset in presets {
        let path = format!("project.text_presets.{}", preset.name);
        check_text_style(&mut issues, &preset.style, &families, &path);
    }

    // Folders are valid paths, sorted, each with its parents; assets sit in listed ones.
    let folders = &project.media.folders;
    let listed = folders.windows(2).all(|w| w[0] < w[1])
        && folders.iter().all(|f| {
            crate::media::valid_folder(f)
                && crate::media::folder_and_parents(f)
                    .iter()
                    .all(|p| folders.contains(p))
        });
    if !listed {
        issues.push(
            "media",
            "fosscut.model.folder",
            "folders must be sorted paths of names, each listed with its parents",
        );
    }
    for asset in &project.media.assets {
        let path = asset.id.to_string();
        expect_kind(&mut issues, &asset.id, IdKind::Asset, &path);
        unique(&mut issues, &asset.id, &path);
        if let Some(folder) = &asset.folder
            && !folders.contains(folder)
        {
            issues.push(
                &path,
                "fosscut.model.folder",
                format!("asset is in folder {folder}, which the media bin does not list"),
            );
        }
        let timed = matches!(
            asset.kind,
            AssetKind::Video | AssetKind::Audio | AssetKind::Web
        );
        if asset.kind == AssetKind::Web && asset.info.video.is_none() {
            issues.push(
                &path,
                "fosscut.model.asset_web",
                "web assets need their size and frame rate in info.video (ADR-0039)",
            );
        }
        if asset.kind == AssetKind::Lut && asset.info.lut.as_ref().is_none_or(|l| l.size < 2) {
            issues.push(
                &path,
                "fosscut.model.asset_lut",
                "LUT assets need a table of at least 2 entries per axis",
            );
        }
        if asset.kind == AssetKind::Vector
            && (asset.info.vector.is_none() || asset.info.video.is_none())
        {
            issues.push(
                &path,
                "fosscut.model.asset_vector",
                "vector assets need their size (video) and drawing (vector) information",
            );
        }
        if asset.kind == AssetKind::Model && asset.info.model.is_none() {
            issues.push(
                &path,
                "fosscut.model.asset_model",
                "model assets need what their file holds in info.model (ADR-0040)",
            );
        }
        if timed && asset.info.duration.is_none_or(|d| !d.is_positive()) {
            issues.push(
                &path,
                "fosscut.model.asset_duration",
                "video and audio assets need a positive duration",
            );
        }
    }

    for (id, timeline) in &project.timelines {
        let timeline_path = id.to_string();
        expect_kind(&mut issues, id, IdKind::Timeline, &timeline_path);
        unique(&mut issues, id, &timeline_path);
        if &timeline.id != id {
            issues.push(
                &timeline_path,
                "fosscut.model.id_mismatch",
                "timeline id does not match its document",
            );
        }
        if timeline.format.width == 0 || timeline.format.height == 0 {
            issues.push(
                &timeline_path,
                "fosscut.model.canvas",
                "canvas width and height must be positive",
            );
        }
        for marker in &timeline.markers {
            let marker_path = format!("{timeline_path}/{}", marker.id);
            expect_kind(&mut issues, &marker.id, IdKind::Marker, &marker_path);
            unique(&mut issues, &marker.id, &marker_path);
            if marker.at.is_negative() {
                issues.push(
                    &marker_path,
                    "fosscut.model.marker",
                    "a marker cannot be before zero",
                );
            }
        }
        for track in &timeline.tracks {
            let track_path = format!("{timeline_path}/{}", track.id);
            expect_kind(&mut issues, &track.id, IdKind::Track, &track_path);
            unique(&mut issues, &track.id, &track_path);
            check_caption_style(&mut issues, track, &track_path);
            let mut previous_end: Option<Flicks> = None;
            for clip in &track.clips {
                let clip_path = format!("{track_path}/{}", clip.id);
                expect_kind(&mut issues, &clip.id, IdKind::Clip, &clip_path);
                unique(&mut issues, &clip.id, &clip_path);
                if clip.start.is_negative() {
                    issues.push(
                        &clip_path,
                        "fosscut.model.negative_start",
                        "clip starts before zero",
                    );
                }
                if !clip.duration.is_positive() {
                    issues.push(
                        &clip_path,
                        "fosscut.model.empty_clip",
                        "clip duration must be positive",
                    );
                }
                if let Some(end) = previous_end
                    && clip.start < end
                {
                    issues.push(
                        &clip_path,
                        "fosscut.model.overlap",
                        "clip overlaps the previous clip or is out of order",
                    );
                }
                previous_end = Some(clip.end());
                if !(0.0..=1.0).contains(&clip.opacity) {
                    issues.push(
                        &clip_path,
                        "fosscut.model.opacity",
                        "opacity must be between 0 and 1",
                    );
                }
                if !(clip.volume >= 0.0 && clip.volume.is_finite()) {
                    issues.push(
                        &clip_path,
                        "fosscut.model.volume",
                        "volume must be a finite number of at least 0",
                    );
                }
                check_properties(&mut issues, clip, &clip_path);
                check_motion(&mut issues, clip, &clip_path);
                if matches!(clip.content, ClipContent::Audio { .. }) && !clip.masks.is_empty() {
                    issues.push(
                        &clip_path,
                        "fosscut.model.mask_target",
                        "audio clips have no picture for masks",
                    );
                }
                for mask in &clip.masks {
                    let mask_path = format!("{clip_path}/{}", mask.id);
                    expect_kind(&mut issues, &mask.id, IdKind::Mask, &mask_path);
                    unique(&mut issues, &mask.id, &mask_path);
                    check_mask(&mut issues, mask, &mask_path);
                }
                for effect in &clip.effects {
                    let effect_path = format!("{clip_path}/{}", effect.id);
                    expect_kind(&mut issues, &effect.id, IdKind::Effect, &effect_path);
                    unique(&mut issues, &effect.id, &effect_path);
                    check_effect(&mut issues, project, effect, &effect_path);
                    check_effect_target(&mut issues, project, clip, effect, &effect_path);
                }
                check_content(&mut issues, project, track.kind, clip, &clip_path);
                for (code, message) in crate::vector::clip_problems(timeline, clip) {
                    issues.push(&clip_path, code, message);
                }
            }
        }
    }
    check_groups(&mut issues, project);
    check_nesting(&mut issues, project);
    issues.0
}

/// Placement, crop, sound and keyframes must hold values their properties allow
/// (ADR-0011, item 3).
fn check_properties(issues: &mut Issues, clip: &Clip, path: &str) {
    let mut bad = |code: &str, message: String| issues.push(path, code, message);
    let t = &clip.transform;
    let finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
    if !finite(&[t.position[0], t.position[1], t.rotation]) {
        bad(
            "fosscut.model.transform",
            "position and rotation must be finite".into(),
        );
    }
    if !finite(&[
        t.position_z,
        t.rotation_x,
        t.rotation_y,
        t.anchor[0],
        t.anchor[1],
    ]) {
        bad(
            "fosscut.model.transform",
            "position_z, rotation_x, rotation_y and anchor must be finite".into(),
        );
    }
    if !(finite(&[t.scale, t.stretch[0], t.stretch[1]])
        && t.scale > 0.0
        && t.stretch.iter().all(|s| *s > 0.0))
    {
        bad(
            "fosscut.model.transform",
            "scale and stretch must be greater than 0".into(),
        );
    }
    let c = &clip.crop;
    let edges = [c.left, c.right, c.top, c.bottom];
    if !edges
        .iter()
        .all(|e| e.is_finite() && (0.0..1.0).contains(e))
        || c.left + c.right >= 1.0
        || c.top + c.bottom >= 1.0
    {
        bad(
            "fosscut.model.crop",
            "crop edges must be 0 to 1 and leave part of the picture".into(),
        );
    }
    let a = &clip.audio;
    if a.detached && !matches!(clip.content, ClipContent::Video { .. }) {
        bad(
            "fosscut.model.detached",
            "only a video clip's sound can be detached".into(),
        );
    }
    if a.fade_in.is_negative() || a.fade_out.is_negative() {
        bad("fosscut.model.fade", "fades cannot be negative".into());
    }
    if !(a.pan.is_finite() && (-1.0..=1.0).contains(&a.pan)) {
        bad("fosscut.model.pan", "pan must be between -1 and 1".into());
    }
    for (property, keys) in &clip.keyframes {
        let Some(spec) = clip_property(property).filter(|p| p.animatable) else {
            bad(
                "fosscut.model.keyframe_path",
                format!("{property} does not take keyframes"),
            );
            continue;
        };
        if keys.is_empty() {
            bad(
                "fosscut.model.keyframes",
                format!("{property} has an empty keyframe list"),
            );
        }
        for pair in keys.windows(2) {
            if pair[1].time <= pair[0].time {
                bad(
                    "fosscut.model.keyframes",
                    format!("{property} keyframes must be in time order, one per time"),
                );
                break;
            }
        }
        for key in keys {
            let values: Vec<f64> = match (spec.kind, key.value) {
                (PropertyKind::Number, KeyValue::Number(v)) => vec![v],
                (PropertyKind::Pair, KeyValue::Pair(v)) => v.to_vec(),
                _ => {
                    bad(
                        "fosscut.model.keyframe_value",
                        format!("{property} keyframes need {:?} values", spec.kind),
                    );
                    continue;
                }
            };
            let in_range = values.iter().all(|v| {
                v.is_finite()
                    && spec.min.is_none_or(|m| *v >= m)
                    && spec.max.is_none_or(|m| *v <= m)
            });
            if !in_range {
                bad(
                    "fosscut.model.keyframe_value",
                    format!("{property} keyframe value {:?} is out of range", key.value),
                );
            }
            if let crate::Interpolation::Bezier([x1, y1, x2, y2]) = key.interpolation
                && !(finite(&[y1, y2]) && (0.0..=1.0).contains(&x1) && (0.0..=1.0).contains(&x2))
            {
                bad(
                    "fosscut.model.keyframe_easing",
                    format!("{property} Bezier x control points must be 0 to 1"),
                );
            }
        }
    }
}

/// Sound effects go on clips with sound, picture effects on clips with a picture
/// (ADR-0026, item 3).
fn check_effect_target(
    issues: &mut Issues,
    project: &Project,
    clip: &Clip,
    effect: &Effect,
    path: &str,
) {
    let Some(spec) = effect_type(&effect.effect_type) else {
        return;
    };
    let sound = match &clip.content {
        ClipContent::Audio { .. } | ClipContent::Timeline { .. } => true,
        ClipContent::Video { asset, .. } => project.media.get(asset).is_some_and(|a| a.has_audio()),
        _ => false,
    };
    let picture = !matches!(
        clip.content,
        ClipContent::Audio { .. } | ClipContent::Camera(_)
    );
    if spec.is_audio() && !sound {
        issues.push(
            path,
            "fosscut.model.effect_target",
            format!("{} changes sound; this clip has none", spec.name),
        );
    } else if !spec.is_audio() && !picture {
        issues.push(
            path,
            "fosscut.model.effect_target",
            format!("{} changes pictures; audio clips have none", spec.name),
        );
    }
}

/// An effect must be a known type and version, with parameters of the right kind and
/// range, existing LUT assets, and keyframes only on animatable parameters (ADR-0011,
/// item 2).
fn check_effect(issues: &mut Issues, project: &Project, effect: &Effect, path: &str) {
    let mut bad = |code: &str, message: String| issues.push(path, code, message);
    let Some(spec) = effect_type(&effect.effect_type) else {
        bad(
            "fosscut.model.effect_type",
            format!(
                "{} is not an effect type this build knows",
                effect.effect_type
            ),
        );
        return;
    };
    if effect.version == 0 || effect.version > spec.version {
        bad(
            "fosscut.model.effect_version",
            format!(
                "{} version {} is not supported; this build reads versions 1 to {}",
                spec.name, effect.version, spec.version
            ),
        );
    }
    let finite = |v: f64| v.is_finite();
    let within = |p: &crate::ParamSpec, v: f64| {
        finite(v) && p.min.is_none_or(|m| v >= m) && p.max.is_none_or(|m| v <= m)
    };
    for param in &spec.params {
        let value = effect.params.get(param.name);
        if value.is_none()
            && matches!(param.default, crate::ParamDefault::Required(_))
            && !effect.keyframes.contains_key(param.name)
        {
            bad(
                "fosscut.model.effect_param",
                format!("{} needs {}", spec.name, param.name),
            );
        }
    }
    for (name, value) in &effect.params {
        let Some(param) = spec.param(name) else {
            bad(
                "fosscut.model.effect_param",
                format!("{} has no parameter {name}", spec.name),
            );
            continue;
        };
        let ok = match (param.kind, value) {
            (ParamKind::Number, ParamValue::Number(v)) => within(param, *v),
            (ParamKind::Pair, ParamValue::Pair(v)) => v.iter().all(|x| within(param, *x)),
            (ParamKind::Bool, ParamValue::Flag(_)) => true,
            (ParamKind::Choice, ParamValue::Name(n)) => param.choices.contains(&n.as_str()),
            (ParamKind::Color, ParamValue::Name(n)) => crate::Color::parse(n).is_ok(),
            (ParamKind::Curve, ParamValue::Points(points)) => {
                points.len() >= 2
                    && points.iter().all(|p| p.iter().all(|x| within(param, *x)))
                    && points.windows(2).all(|w| w[1][0] > w[0][0])
            }
            (ParamKind::Asset, ParamValue::Asset(reference)) => {
                let wanted = param.choices.first().copied();
                match project.media.get(&reference.asset) {
                    Some(asset) => wanted != Some("lut") || asset.kind == AssetKind::Lut,
                    None => false,
                }
            }
            _ => false,
        };
        if !ok {
            bad(
                "fosscut.model.effect_param",
                format!("{}.{name} has an invalid value {value:?}", spec.name),
            );
        }
    }
    for (name, keys) in &effect.keyframes {
        let Some(param) = spec.param(name).filter(|p| p.animatable) else {
            bad(
                "fosscut.model.keyframe_path",
                format!("{}.{name} does not take keyframes", spec.name),
            );
            continue;
        };
        if keys.is_empty() || keys.windows(2).any(|w| w[1].time <= w[0].time) {
            bad(
                "fosscut.model.keyframes",
                format!(
                    "{}.{name} keyframes must be in time order, one per time",
                    spec.name
                ),
            );
        }
        for key in keys {
            let ok = match (param.kind, key.value) {
                (ParamKind::Number, KeyValue::Number(v)) => within(param, v),
                (ParamKind::Pair, KeyValue::Pair(v)) => v.iter().all(|x| within(param, *x)),
                _ => false,
            };
            if !ok {
                bad(
                    "fosscut.model.keyframe_value",
                    format!(
                        "{}.{name} keyframe value {:?} is invalid",
                        spec.name, key.value
                    ),
                );
            }
        }
    }
}

/// A transition needs a known type, valid parameters and a positive length no longer than
/// its clip; animations need known presets in their places and positive lengths.
fn check_motion(issues: &mut Issues, clip: &Clip, path: &str) {
    if let Some(speed) = &clip.speed {
        check_speed(issues, clip, speed, path);
    }
    let mut bad = |code: &str, message: String| issues.push(path, code, message);
    if let Some(t) = &clip.transition {
        match crate::transition_type(&t.transition_type) {
            None => bad(
                "fosscut.model.transition",
                format!("{} is not a transition this build knows", t.transition_type),
            ),
            Some(spec) => {
                for (name, value) in &t.params {
                    let ok = spec.param(name).is_some_and(|p| match (p.kind, value) {
                        (ParamKind::Choice, ParamValue::Name(n)) => p.choices.contains(&n.as_str()),
                        (ParamKind::Color, ParamValue::Name(n)) => crate::Color::parse(n).is_ok(),
                        (ParamKind::Number, ParamValue::Number(v)) => {
                            v.is_finite()
                                && p.min.is_none_or(|m| *v >= m)
                                && p.max.is_none_or(|m| *v <= m)
                        }
                        _ => false,
                    });
                    if !ok {
                        bad(
                            "fosscut.model.transition",
                            format!("{} has no valid parameter {name} = {value:?}", spec.name),
                        );
                    }
                }
            }
        }
        if !t.duration.is_positive() || t.duration > clip.duration {
            bad(
                "fosscut.model.transition",
                "a transition lasts more than zero and no longer than its clip".into(),
            );
        }
    }
    let a = &clip.animations;
    for (place, animation) in [("in", &a.entrance), ("out", &a.exit), ("combo", &a.combo)] {
        let Some(animation) = animation else {
            continue;
        };
        let fits = crate::ANIMATION_PRESETS
            .iter()
            .any(|(name, places, _)| *name == animation.preset && places.contains(place));
        if !fits || !animation.duration.is_positive() {
            bad(
                "fosscut.model.animation",
                format!(
                    "{} is not a positive-length {place} animation",
                    animation.preset
                ),
            );
        }
        if animation.delay.is_negative() {
            bad(
                "fosscut.model.animation",
                format!("the {place} animation's delay cannot be negative"),
            );
        }
    }
}

/// Compound clips show compound timelines of the project with the same frame rate, within
/// their length, without cycles and at most eight levels deep (ADR-0025).
fn check_nesting(issues: &mut Issues, project: &Project) {
    use crate::TimelineRole;
    let compound = |id: &Id| {
        project
            .manifest
            .timelines
            .iter()
            .any(|t| &t.id == id && t.role == TimelineRole::Compound)
    };
    let shows = |timeline: &crate::Timeline| -> Vec<Id> {
        timeline
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter_map(|c| c.content.nested().cloned())
            .collect()
    };
    for (id, timeline) in &project.timelines {
        for track in &timeline.tracks {
            for clip in &track.clips {
                let Some(nested) = clip.content.nested() else {
                    continue;
                };
                let path = format!("{id}/{}/{}", track.id, clip.id);
                let mut bad =
                    |message: String| issues.push(&path, "fosscut.model.nesting", message);
                let Some(inner) = project.timelines.get(nested) else {
                    bad(format!("timeline {nested} does not exist"));
                    continue;
                };
                if !compound(nested) {
                    bad(format!("{nested} is not a compound timeline"));
                }
                if inner.format.rate != timeline.format.rate {
                    bad(format!("{nested} has another frame rate"));
                }
                let source_in = clip.content.source_in().unwrap_or_default();
                if source_in.is_negative() || source_in + clip.source_length() > inner.duration() {
                    issues.push(
                        &path,
                        "fosscut.model.source_range",
                        format!("the clip reads past the end of {nested}"),
                    );
                }
            }
        }
        // Depth first through what this timeline shows: a cycle or more than 8 levels.
        let mut stack = vec![(id.clone(), 0usize, vec![id.clone()])];
        while let Some((at, depth, path)) = stack.pop() {
            let Some(doc) = project.timelines.get(&at) else {
                continue;
            };
            for next in shows(doc) {
                if path.contains(&next) {
                    issues.push(
                        id.as_str(),
                        "fosscut.model.nesting",
                        format!("{id} shows itself through {next}"),
                    );
                    return;
                }
                if depth + 1 > 8 {
                    issues.push(
                        id.as_str(),
                        "fosscut.model.nesting",
                        "compound clips nest more than eight levels deep",
                    );
                    return;
                }
                let mut deeper = path.clone();
                deeper.push(next.clone());
                stack.push((next, depth + 1, deeper));
            }
        }
    }
}

/// A group has two or more clips, all on one timeline (ADR-0024).
fn check_groups(issues: &mut Issues, project: &Project) {
    let mut groups: BTreeMap<&Id, (Vec<&Id>, BTreeSet<&Id>, String)> = BTreeMap::new();
    for (timeline_id, timeline) in &project.timelines {
        for track in &timeline.tracks {
            for clip in &track.clips {
                if let Some(group) = &clip.group {
                    let entry = groups.entry(group).or_insert_with(|| {
                        (
                            Vec::new(),
                            BTreeSet::new(),
                            format!("{timeline_id}/{}/{}", track.id, clip.id),
                        )
                    });
                    entry.0.push(&clip.id);
                    entry.1.insert(timeline_id);
                }
            }
        }
    }
    for (group, (clips, timelines, path)) in groups {
        if group.prefix() != IdKind::Group.prefix() {
            issues.push(
                &path,
                "fosscut.model.id_kind",
                format!("{group} is not a group id"),
            );
        }
        if clips.len() < 2 || timelines.len() > 1 {
            issues.push(
                &path,
                "fosscut.model.group",
                format!("group {group} needs two or more clips on one timeline"),
            );
        }
    }
}

/// Speed applies to video, audio and compound clips, with a rate from 0.05 to 100 (or 0, a freeze)
/// or a curve of at least two points in order (ADR-0023).
fn check_speed(issues: &mut Issues, clip: &Clip, speed: &crate::Speed, path: &str) {
    let mut bad = |message: &str| issues.push(path, "fosscut.model.speed", message.to_string());
    let in_range = |rate: f64| (crate::MIN_RATE..=crate::MAX_RATE).contains(&rate);
    if !matches!(
        clip.content,
        ClipContent::Video { .. } | ClipContent::Audio { .. } | ClipContent::Timeline { .. }
    ) {
        bad("only video, audio and compound clips have a speed");
    }
    if speed.curve.is_empty() {
        if !(speed.rate == 0.0 || in_range(speed.rate)) {
            bad("a speed rate is 0 (a freeze) or from 0.05 to 100");
        }
        return;
    }
    if speed.rate != 1.0 {
        bad("a speed has a rate or a curve, not both");
    }
    let ordered = speed.curve.windows(2).all(|p| p[0].at < p[1].at);
    let inside = speed
        .curve
        .iter()
        .all(|p| (0.0..=1.0).contains(&p.at) && in_range(p.rate));
    if speed.curve.len() < 2 || !ordered || !inside {
        bad(
            "a speed curve has two or more points in order, at 0 to 1, with rates from 0.05 to 100",
        );
    }
}

/// A mask's values must be in range, and its shape must have what it needs: a path at
/// least three points, a text mask some text, a brush at least one stroke.
fn check_mask(issues: &mut Issues, mask: &crate::Mask, path: &str) {
    use crate::MaskShape;
    let mut bad = |code: &str, message: String| issues.push(path, code, message);
    let finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
    let ok = finite(&[
        mask.center[0],
        mask.center[1],
        mask.size[0],
        mask.size[1],
        mask.rotation,
    ]) && mask.size.iter().all(|s| *s >= 0.0)
        && (0.0..=1.0).contains(&mask.feather)
        && (-1.0..=1.0).contains(&mask.expansion)
        && (0.0..=1.0).contains(&mask.roundness);
    if !ok {
        bad(
            "fosscut.model.mask",
            "mask size must be at least 0, feather and roundness 0 to 1, expansion -1 to 1".into(),
        );
    }
    let shaped = match mask.shape {
        MaskShape::Path => {
            mask.points.len() >= 3
                && mask.points.iter().all(|p| {
                    finite(&p.at)
                        && p.in_handle.is_none_or(|h| finite(&h))
                        && p.out_handle.is_none_or(|h| finite(&h))
                })
        }
        MaskShape::Text => mask.text.as_deref().is_some_and(|t| !t.trim().is_empty()),
        MaskShape::Brush => {
            !mask.strokes.is_empty()
                && mask.strokes.iter().all(|s| {
                    !s.points.is_empty()
                        && s.radius.is_finite()
                        && s.radius > 0.0
                        && s.points.iter().all(|p| finite(p))
                })
        }
        _ => true,
    };
    if !shaped {
        bad(
            "fosscut.model.mask_shape",
            format!(
                "a {:?} mask needs its points (three or more), text or strokes",
                mask.shape
            ),
        );
    }
    for (name, keys) in &mask.keyframes {
        let Some((_, pair, low, high)) = crate::MASK_PROPERTIES.iter().find(|(n, ..)| n == name)
        else {
            bad(
                "fosscut.model.keyframe_path",
                format!("mask {name} does not take keyframes"),
            );
            continue;
        };
        if keys.is_empty() || keys.windows(2).any(|w| w[1].time <= w[0].time) {
            bad(
                "fosscut.model.keyframes",
                format!("mask {name} keyframes must be in time order, one per time"),
            );
        }
        for key in keys {
            let values: Option<Vec<f64>> = match (pair, key.value) {
                (false, KeyValue::Number(v)) => Some(vec![v]),
                (true, KeyValue::Pair(v)) => Some(v.to_vec()),
                _ => None,
            };
            let fine =
                values.is_some_and(|v| v.iter().all(|x| !x.is_nan() && x >= low && x <= high));
            if !fine {
                bad(
                    "fosscut.model.keyframe_value",
                    format!("mask {name} keyframe value {:?} is invalid", key.value),
                );
            }
        }
    }
}

/// A camera's values must be in range, and a camera clip has no picture or sound of its
/// own for effects, masks, transitions, animations or speed (ADR-0037).
fn check_camera(issues: &mut Issues, clip: &Clip, camera: &crate::CameraContent, path: &str) {
    for message in crate::camera_issues(camera) {
        issues.push(path, "fosscut.model.camera", message);
    }
    let a = &clip.animations;
    let extras = !clip.effects.is_empty()
        || !clip.masks.is_empty()
        || clip.transition.is_some()
        || a.entrance.is_some()
        || a.exit.is_some()
        || a.combo.is_some()
        || clip.speed.is_some();
    if extras {
        issues.push(
            path,
            "fosscut.model.camera",
            "camera clips have no effects, masks, transitions, animations or speed",
        );
    }
}

fn check_content(
    issues: &mut Issues,
    project: &Project,
    track_kind: TrackKind,
    clip: &Clip,
    path: &str,
) {
    let (content, duration) = (&clip.content, clip.duration);
    let allowed = match content {
        ClipContent::Video { .. }
        | ClipContent::Image { .. }
        | ClipContent::Adjustment
        | ClipContent::Shape(_)
        | ClipContent::Timeline { .. }
        | ClipContent::Camera(_)
        | ClipContent::Particles { .. }
        | ClipContent::Model { .. } => track_kind == TrackKind::Video,
        ClipContent::Audio { .. } => track_kind == TrackKind::Audio,
        ClipContent::Text(_) => track_kind == TrackKind::Text,
        ClipContent::Caption(_) => track_kind == TrackKind::Caption,
    };
    if !allowed {
        issues.push(
            path,
            "fosscut.model.track_kind",
            format!(
                "{} clips cannot be placed on {:?} tracks",
                content.kind_name(),
                track_kind
            ),
        );
    }
    if let ClipContent::Text(text) = content {
        if text.text.trim().is_empty() {
            issues.push(path, "fosscut.model.empty_text", "text clips need text");
        }
        check_text(issues, project, text, path);
    }
    if let ClipContent::Caption(caption) = content {
        check_caption(issues, caption, duration, path);
    }
    if let ClipContent::Shape(shape) = content {
        check_shape(issues, project, shape, path);
    }
    if let ClipContent::Camera(camera) = content {
        check_camera(issues, clip, camera, path);
    }
    if let ClipContent::Particles { particles } = content {
        for issue in crate::particles::particle_issues(project, particles) {
            issues.push(path, &issue.code, issue.message);
        }
    }
    if let ClipContent::Model { model } = content {
        for issue in crate::model3d::model_issues(project, model) {
            issues.push(path, &issue.code, issue.message);
        }
    }
    if let Some(angle) = &clip.angle {
        let shown = match content {
            ClipContent::Timeline { timeline, .. } => {
                project.timelines.get(timeline).is_some_and(|t| {
                    t.tracks
                        .iter()
                        .any(|track| &track.id == angle && track.kind == TrackKind::Video)
                })
            }
            _ => false,
        };
        if !shown {
            issues.push(
                path,
                "fosscut.model.angle",
                "an angle must be a video track of the compound clip's timeline",
            );
        }
    }
    if let Some(fill) = &clip.canvas {
        if !matches!(
            content,
            ClipContent::Video { .. } | ClipContent::Image { .. }
        ) {
            issues.push(
                path,
                "fosscut.model.canvas",
                "only video and image clips have a canvas fill",
            );
        }
        if let crate::CanvasFill::Blur { amount } = fill
            && !(amount.is_finite() && (0.0..=0.2).contains(amount))
        {
            issues.push(
                path,
                "fosscut.model.canvas",
                "a canvas blur's amount is from 0 to 0.2 of the canvas height",
            );
        }
    }
    let Some(asset_id) = content.asset() else {
        return;
    };
    let Some(asset) = project.media.get(asset_id) else {
        issues.push(
            path,
            "fosscut.model.missing_asset",
            format!("asset {asset_id} does not exist"),
        );
        return;
    };
    let kind_ok = match content {
        ClipContent::Video { .. } => asset.kind.is_video(),
        ClipContent::Audio { .. } => asset.has_audio(),
        ClipContent::Image { .. } => matches!(asset.kind, AssetKind::Image | AssetKind::Vector),
        ClipContent::Text(_)
        | ClipContent::Caption(_)
        | ClipContent::Adjustment
        | ClipContent::Shape(_)
        | ClipContent::Timeline { .. }
        | ClipContent::Camera(_)
        | ClipContent::Particles { .. } => true,
        ClipContent::Model { .. } => asset.kind == AssetKind::Model,
    };
    if !kind_ok {
        issues.push(
            path,
            "fosscut.model.asset_kind",
            format!(
                "{} clip cannot use {:?} asset {asset_id}",
                content.kind_name(),
                asset.kind
            ),
        );
    }
    // With speed, the clip uses more or less source than its duration (ADR-0023).
    if let (Some(source_in), Some(asset_duration)) = (content.source_in(), asset.info.duration)
        && (source_in.is_negative() || source_in + clip.source_length() > asset_duration)
    {
        issues.push(
            path,
            "fosscut.model.source_range",
            "clip reads beyond the start or end of its asset",
        );
    }
}

/// Problems with a shape in `project` (ADR-0028), so commands can report them before they
/// change anything.
pub fn shape_issues(project: &Project, shape: &ShapeContent) -> Vec<Issue> {
    let mut issues = Issues(Vec::new());
    check_shape(&mut issues, project, shape, "shape");
    issues.0
}

/// A shape must be drawable, sized, and its outline and label must make sense.
fn check_shape(issues: &mut Issues, project: &Project, shape: &ShapeContent, path: &str) {
    let within = |v: f64, low: f64, high: f64| v.is_finite() && v >= low && v <= high;
    let mut bad = |message: &str| issues.push(path, "fosscut.model.shape", message);
    if !ShapeContent::drawable(shape.shape) {
        bad("shapes are rectangle, ellipse, star, heart or path");
    }
    if !shape.size.iter().all(|&v| within(v, 1e-4, 4.0)) {
        bad("size must be from 0.0001 to 4 times the canvas");
    }
    if !within(shape.roundness, 0.0, 1.0) {
        bad("roundness must be from 0 to 1");
    }
    if !within(shape.outline.width, 0.0, 0.5) {
        bad("the outline width must be from 0 to 0.5 of the shorter side");
    }
    if !shape.label_at.iter().all(|&v| within(v, -1.0, 1.0)) {
        bad("label_at must be from -1 to 1 on each axis");
    }
    if shape.shape == crate::MaskShape::Path && shape.closed && shape.points.len() < 3 {
        bad("a path shape needs at least three points");
    }
    // Strokes, open paths, contours and trims (ADR-0036).
    for problem in crate::vector::shape_problems(shape) {
        bad(&problem);
    }
    if let Some(label) = &shape.label {
        if label.text.trim().is_empty() {
            issues.push(path, "fosscut.model.shape", "a label needs text");
        }
        check_text(issues, project, label, path);
    }
}

/// Problems with a text's style, spans and reveal in `project` (ADR-0027), so commands can
/// report them before they change anything.
pub fn text_issues(project: &Project, text: &TextContent) -> Vec<Issue> {
    let mut issues = Issues(Vec::new());
    check_text(&mut issues, project, text, "text");
    issues.0
}

/// Problems with a text style in `project`, as [`text_issues`].
pub fn text_style_issues(project: &Project, style: &TextStyle) -> Vec<Issue> {
    let mut issues = Issues(Vec::new());
    check_text_style(&mut issues, style, &project.media.font_families(), "style");
    issues.0
}

/// A text's style, spans and reveal must make sense, and name only known fonts (ADR-0027).
fn check_text(issues: &mut Issues, project: &Project, text: &TextContent, path: &str) {
    let families = project.media.font_families();
    check_text_style(issues, &text.style, &families, path);
    let length = text.text.chars().count();
    for (index, span) in text.spans.iter().enumerate() {
        if span.start >= span.end || span.end > length {
            issues.push(
                path,
                "fosscut.model.text_span",
                format!(
                    "span {index} ({} to {}) must cover characters of the {length}-character text",
                    span.start, span.end
                ),
            );
        }
        if span
            .size
            .is_some_and(|s| !(s.is_finite() && s > 0.0 && s <= 1.0))
        {
            issues.push(
                path,
                "fosscut.model.text_style",
                format!("span {index}: size must be a fraction of the canvas height up to 1"),
            );
        }
        check_font(issues, span.font.as_deref(), &families, path);
    }
    if let Some(reveal) = &text.reveal
        && !reveal.duration.is_positive()
    {
        issues.push(
            path,
            "fosscut.model.text_reveal",
            "a reveal needs a positive duration",
        );
    }
    if let Some(reveal) = &text.reveal {
        for problem in crate::text::reveal_problems(reveal) {
            issues.push(path, "fosscut.model.text_reveal", problem);
        }
    }
}

/// The ranges ADR-0027 gives text style values.
fn check_text_style(issues: &mut Issues, style: &TextStyle, families: &[String], path: &str) {
    let within = |v: f64, low: f64, high: f64| v.is_finite() && v >= low && v <= high;
    let checks = [
        (
            within(style.size, f64::MIN_POSITIVE, 1.0),
            "size must be a fraction of the canvas height up to 1",
        ),
        (
            within(style.letter_spacing, -0.5, 2.0),
            "letter_spacing must be from -0.5 to 2 em",
        ),
        (
            within(style.line_height, 0.5, 4.0),
            "line_height must be from 0.5 to 4",
        ),
        (
            within(style.stroke.width, 0.0, 1.0),
            "stroke width must be from 0 to 1 em",
        ),
        (
            within(style.background.padding, 0.0, 2.0) && within(style.background.radius, 0.0, 1.0),
            "box padding must be from 0 to 2 em and radius from 0 to 1",
        ),
        (
            style.shadow.offset.iter().all(|&v| within(v, -1.0, 1.0))
                && within(style.shadow.blur, 0.0, 1.0),
            "shadow offset must be from -1 to 1 em and blur from 0 to 1 em",
        ),
        (
            within(style.glow.radius, 0.0, 2.0),
            "glow radius must be from 0 to 2 em",
        ),
    ];
    for (ok, message) in checks {
        if !ok {
            issues.push(path, "fosscut.model.text_style", message);
        }
    }
    check_font(issues, style.font.as_deref(), families, path);
}

fn check_font(issues: &mut Issues, font: Option<&str>, families: &[String], path: &str) {
    if let Some(font) = font
        && !families.iter().any(|f| f == font)
    {
        issues.push(
            path,
            "fosscut.model.unknown_font",
            format!(
                "font {font:?} is neither bundled nor in a font asset; known: {}",
                families.join(", ")
            ),
        );
    }
}

/// Caption pages need words in order, each inside the clip (ADR-0021 item 1).
fn check_caption(issues: &mut Issues, caption: &CaptionContent, duration: Flicks, path: &str) {
    let mut bad = |message: String| issues.push(path, "fosscut.model.caption_words", message);
    if caption.lines.is_empty() || caption.lines.iter().any(|line| line.words.is_empty()) {
        bad("caption pages need at least one line, and every line at least one word".into());
        return;
    }
    let mut previous_end = Flicks::ZERO;
    for (index, word) in caption.words().enumerate() {
        if word.text.trim().is_empty() {
            bad(format!("word {index} has no text"));
        }
        if word.start.is_negative() || word.end < word.start || word.end > duration {
            bad(format!(
                "word {index} ({}) is not inside the clip",
                word.text
            ));
        }
        if word.start < previous_end {
            bad(format!(
                "word {index} ({}) starts before the previous word ends",
                word.text
            ));
        }
        previous_end = word.end;
    }
}

/// Only caption tracks carry a caption style, and its values must make sense.
fn check_caption_style(issues: &mut Issues, track: &Track, path: &str) {
    let Some(style) = &track.caption_style else {
        return;
    };
    let mut bad = |message: &str| issues.push(path, "fosscut.model.caption_style", message);
    if track.kind != TrackKind::Caption {
        bad("only caption tracks have a caption style");
    }
    let CaptionStyle {
        size,
        stroke,
        background,
        animators,
        translation,
        ..
    } = style;
    if !(translation.size.is_finite() && (0.3..=1.0).contains(&translation.size)) {
        bad("translation size must be from 0.3 to 1 of the caption size");
    }
    if !(size.is_finite() && *size > 0.0) {
        bad("caption size must be a positive fraction of the canvas height");
    }
    if !(stroke.width.is_finite() && stroke.width >= 0.0) {
        bad("stroke width must be zero or more");
    }
    if !(background.padding.is_finite()
        && background.padding >= 0.0
        && background.radius.is_finite()
        && background.radius >= 0.0)
    {
        bad("box padding and radius must be zero or more");
    }
    for animator in animators {
        let supported = Animator::TYPES
            .iter()
            .find(|(name, _)| *name == animator.type_name())
            .map_or(0, |(_, version)| *version);
        if animator.version() == 0 || animator.version() > supported {
            issues.push(
                path,
                "fosscut.model.animator_version",
                format!(
                    "{} version {} is not supported; this build reads versions 1 to {supported}",
                    animator.type_name(),
                    animator.version()
                ),
            );
        }
        let scales = match animator {
            Animator::PagePop(a) => vec![a.params.from],
            Animator::WordHighlight(a) => vec![a.params.scale],
            Animator::WordPop(a) => vec![a.params.scale],
            Animator::WordEmphasis(a) => vec![a.params.scale],
            _ => Vec::new(),
        };
        if scales.iter().any(|s| !(s.is_finite() && *s > 0.0)) {
            issues.push(
                path,
                "fosscut.model.caption_style",
                format!("{} scale must be positive", animator.type_name()),
            );
        }
    }
}
