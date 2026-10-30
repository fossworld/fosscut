use std::collections::BTreeSet;
use std::fmt;

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Animator, AssetKind, CaptionContent, CaptionStyle, ClipContent, Id, IdKind, Project, Track,
    TrackKind,
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
        if asset.kind != AssetKind::Image && asset.info.duration.is_none_or(|d| !d.is_positive()) {
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

fn check_content(
    issues: &mut Issues,
    project: &Project,
    track_kind: TrackKind,
    content: &ClipContent,
    duration: Flicks,
    path: &str,
) {
    let allowed = match content {
        ClipContent::Video { .. } | ClipContent::Image { .. } => track_kind == TrackKind::Video,
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
        ClipContent::Text(_) | ClipContent::Caption(_) => true,
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
