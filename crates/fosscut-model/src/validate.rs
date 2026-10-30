use std::collections::BTreeSet;
use std::fmt;

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Animator, AssetKind, CaptionContent, CaptionStyle, Clip, ClipContent, Effect, Id, IdKind,
    KeyValue, ParamKind, ParamValue, Project, PropertyKind, Track, TrackKind, clip_property,
    effect_type,
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

    for asset in &project.media.assets {
        let path = asset.id.to_string();
        expect_kind(&mut issues, &asset.id, IdKind::Asset, &path);
        unique(&mut issues, &asset.id, &path);
        let timed = matches!(asset.kind, AssetKind::Video | AssetKind::Audio);
        if asset.kind == AssetKind::Lut && asset.info.lut.as_ref().is_none_or(|l| l.size < 2) {
            issues.push(
                &path,
                "fosscut.model.asset_lut",
                "LUT assets need a table of at least 2 entries per axis",
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
                if matches!(clip.content, ClipContent::Audio { .. }) && !clip.effects.is_empty() {
                    issues.push(
                        &clip_path,
                        "fosscut.model.effect_target",
                        "audio clips have no picture for effects",
                    );
                }
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
                }
                check_content(
                    &mut issues,
                    project,
                    track.kind,
                    &clip.content,
                    clip.duration,
                    &clip_path,
                );
            }
        }
    }
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

fn check_content(
    issues: &mut Issues,
    project: &Project,
    track_kind: TrackKind,
    content: &ClipContent,
    duration: Flicks,
    path: &str,
) {
    let allowed = match content {
        ClipContent::Video { .. } | ClipContent::Image { .. } | ClipContent::Adjustment => {
            track_kind == TrackKind::Video
        }
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
    if let ClipContent::Text(text) = content
        && text.text.trim().is_empty()
    {
        issues.push(path, "fosscut.model.empty_text", "text clips need text");
    }
    if let ClipContent::Caption(caption) = content {
        check_caption(issues, caption, duration, path);
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
        ClipContent::Video { .. } => asset.kind == AssetKind::Video,
        ClipContent::Audio { .. } => asset.has_audio(),
        ClipContent::Image { .. } => asset.kind == AssetKind::Image,
        ClipContent::Text(_) | ClipContent::Caption(_) | ClipContent::Adjustment => true,
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
    if let (Some(source_in), Some(asset_duration)) = (content.source_in(), asset.info.duration)
        && (source_in.is_negative() || source_in + duration > asset_duration)
    {
        issues.push(
            path,
            "fosscut.model.source_range",
            "clip reads beyond the start or end of its asset",
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
        ..
    } = style;
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
