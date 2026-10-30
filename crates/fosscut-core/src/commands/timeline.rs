//! Commands that reshape a whole edit: `timeline.duplicate` copies a timeline onto a canvas
//! of another shape, and `timeline.remove_ranges` cuts time ranges out of every track and
//! closes the gaps (the cut and ripple of ADR-0007's patches).

use fosscut_model::{
    ClipContent, Id, IdKind, Project, Timeline, TimelineEntry, TimelineFormat, TrackKind,
};
use fosscut_time::{Flicks, Rate, TimeInput, TimeReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::caption::window_caption;
use super::util::{ensure_unlocked, resolve, timeline_id, timeline_mut, track_index};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// How pictures meet a canvas of another shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Keep each clip's placement: pictures stay fitted inside the canvas, with bars.
    #[default]
    Keep,
    /// Scale video and image clips to cover the canvas, cropping what falls outside.
    Fill,
}

/// Copies a timeline, optionally onto a canvas of another size.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimelineDuplicate {
    /// The timeline to copy; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Name of the copy; the original's name with the canvas size when omitted.
    #[serde(default)]
    pub name: Option<String>,
    /// Canvas width of the copy in pixels; the original's when omitted.
    #[serde(default)]
    pub width: Option<u32>,
    /// Canvas height of the copy in pixels; the original's when omitted.
    #[serde(default)]
    pub height: Option<u32>,
    /// `keep` (default) or `fill`: scale video and image clips to cover the new canvas.
    #[serde(default)]
    pub fit: Fit,
    /// Make the copy the primary timeline, which commands, frames and exports use when
    /// they are given no timeline.
    #[serde(default)]
    pub primary: bool,
}

/// Result of `timeline.duplicate`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TimelineDuplicated {
    /// The copy.
    pub timeline: Id,
    /// Its name.
    pub name: String,
    /// The timeline it was copied from.
    pub source: Id,
    /// Its canvas, rate and sample rate.
    pub format: TimelineFormat,
    /// Clips copied.
    pub clips: usize,
    /// Whether it is now the primary timeline.
    pub primary: bool,
}

/// The scale that makes a picture of `size` (fitted inside the canvas at scale 1) cover
/// `canvas` instead.
fn fill_scale(size: (u32, u32), canvas: (u32, u32)) -> f64 {
    let (w, h) = (f64::from(size.0.max(1)), f64::from(size.1.max(1)));
    let (cw, ch) = (f64::from(canvas.0), f64::from(canvas.1));
    let fit = (cw / w).min(ch / h);
    let cover = (cw / w).max(ch / h);
    cover / fit
}

impl Command for TimelineDuplicate {
    const NAME: &'static str = "timeline.duplicate";
    const SUMMARY: &'static str = "Copy a timeline, for example onto a 1080x1920 canvas for a vertical version; fit fill scales the pictures to cover the new canvas. The original stays as it is.";
    type Output = TimelineDuplicated;

    fn apply(self, ctx: &mut Context<'_>) -> Result<TimelineDuplicated, ApiError> {
        let source = timeline_id(ctx.project, self.timeline.as_ref())?;
        let original = ctx.project.timelines[&source].clone();
        let format = TimelineFormat {
            width: self.width.unwrap_or(original.format.width),
            height: self.height.unwrap_or(original.format.height),
            ..original.format.clone()
        };
        if format.width == 0 || format.height == 0 {
            return Err(invalid(
                "timeline.duplicate: width and height must be at least 1",
            ));
        }
        let name = self
            .name
            .clone()
            .unwrap_or_else(|| format!("{} {}x{}", original.name, format.width, format.height));
        let id = ctx.project.allocate_id(IdKind::Timeline);
        let mut copy = Timeline {
            id: id.clone(),
            name: name.clone(),
            format: format.clone(),
            tracks: Vec::with_capacity(original.tracks.len()),
        };
        let mut clips = 0;
        for track in &original.tracks {
            let mut track = track.clone();
            track.id = ctx.project.allocate_id(IdKind::Track);
            for clip in &mut track.clips {
                clip.id = ctx.project.allocate_id(IdKind::Clip);
                clips += 1;
                let asset = match &clip.content {
                    ClipContent::Video { asset, .. } | ClipContent::Image { asset } => asset,
                    _ => continue,
                };
                if self.fit == Fit::Fill
                    && let Some(video) = ctx
                        .project
                        .media
                        .get(asset)
                        .and_then(|a| a.info.video.as_ref())
                {
                    let turned = video.rotation.rem_euclid(180) == 90;
                    let size = if turned {
                        (video.height, video.width)
                    } else {
                        (video.width, video.height)
                    };
                    let factor = fill_scale(size, (format.width, format.height));
                    clip.transform.scale *= factor;
                    for key in clip
                        .keyframes
                        .get_mut("transform.scale")
                        .into_iter()
                        .flatten()
                    {
                        if let fosscut_model::KeyValue::Number(v) = &mut key.value {
                            *v *= factor;
                        }
                    }
                }
            }
            copy.tracks.push(track);
        }
        ctx.project.manifest.timelines.push(TimelineEntry {
            id: id.clone(),
            name: name.clone(),
        });
        ctx.project.timelines.insert(id.clone(), copy);
        if self.primary {
            ctx.project.manifest.primary_timeline = id.clone();
        }
        Ok(TimelineDuplicated {
            timeline: id,
            name,
            source,
            format,
            clips,
            primary: self.primary,
        })
    }
}

/// A time range on the timeline.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeInput {
    /// Where it starts; snapped to the nearest frame.
    pub start: TimeInput,
    /// Where it ends (exclusive); snapped to the nearest frame.
    pub end: TimeInput,
}

/// Removes time ranges from a timeline.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimelineRemoveRanges {
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Time ranges to remove, for example
    /// `[{"start": {"seconds": "3.4"}, "end": {"seconds": "4.1"}}]`. Overlapping and
    /// touching ranges are merged.
    pub ranges: Vec<RangeInput>,
    /// Tracks to cut; every track when omitted. Tracks left out keep their timing and can
    /// fall out of sync with the others.
    #[serde(default)]
    pub tracks: Option<Vec<Id>>,
    /// Move everything after each range left to close the gap (default true).
    #[serde(default = "yes")]
    pub ripple: bool,
}

fn yes() -> bool {
    true
}

/// A range as reported.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RangeReport {
    /// Start.
    pub start: TimeReport,
    /// End (exclusive).
    pub end: TimeReport,
}

/// Result of `timeline.remove_ranges`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RangesRemoved {
    /// The timeline.
    pub timeline: Id,
    /// The ranges removed, merged and on frames.
    pub ranges: Vec<RangeReport>,
    /// Their total length.
    pub removed: TimeReport,
    /// The timeline's length before.
    pub duration_before: TimeReport,
    /// Its length after.
    pub duration_after: TimeReport,
    /// Clips removed entirely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips_removed: Vec<Id>,
    /// Clips shortened or moved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips_changed: Vec<Id>,
    /// New clips: the later parts of clips a range cut through.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips_added: Vec<Id>,
}

/// Sorted, merged, frame-aligned ranges.
fn merged(ranges: &[RangeInput], rate: Rate) -> Result<Vec<(Flicks, Flicks)>, ApiError> {
    if ranges.is_empty() {
        return Err(invalid("timeline.remove_ranges: give at least one range"));
    }
    let mut spans = Vec::with_capacity(ranges.len());
    for (index, range) in ranges.iter().enumerate() {
        let (start, end) = (
            resolve(&range.start, rate)?.max(Flicks::ZERO),
            resolve(&range.end, rate)?,
        );
        if end <= start {
            return Err(invalid(format!(
                "timeline.remove_ranges: range {index} is shorter than a frame or ends before it starts"
            ))
            .with_details(json!({ "range": index })));
        }
        spans.push((start, end));
    }
    spans.sort();
    let mut out: Vec<(Flicks, Flicks)> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    Ok(out)
}

/// The parts of `[start, end)` outside every range.
fn kept(start: Flicks, end: Flicks, ranges: &[(Flicks, Flicks)]) -> Vec<(Flicks, Flicks)> {
    let mut parts = Vec::new();
    let mut from = start;
    for &(a, b) in ranges {
        if b <= from || a >= end {
            continue;
        }
        if a > from {
            parts.push((from, a));
        }
        from = from.max(b);
    }
    if from < end {
        parts.push((from, end));
    }
    parts
}

/// How much of the ranges lies before `time`.
fn removed_before(time: Flicks, ranges: &[(Flicks, Flicks)]) -> Flicks {
    ranges
        .iter()
        .filter(|(a, _)| *a < time)
        .map(|&(a, b)| b.min(time) - a)
        .fold(Flicks::ZERO, |sum, part| sum + part)
}

impl Command for TimelineRemoveRanges {
    const NAME: &'static str = "timeline.remove_ranges";
    const SUMMARY: &'static str = "Cut time ranges (pauses, fillers, retakes) out of every track at once and close the gaps, keeping picture, sound and captions in sync. Cutting the primary video track needs allow_destructive.";
    type Output = RangesRemoved;

    fn apply(self, ctx: &mut Context<'_>) -> Result<RangesRemoved, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let ranges = merged(&self.ranges, rate)?;
        let doc = &ctx.project.timelines[&timeline];
        let chosen: Vec<usize> = match &self.tracks {
            Some(ids) => ids
                .iter()
                .map(|id| track_index(doc, id))
                .collect::<Result<_, _>>()?,
            None => (0..doc.tracks.len()).collect(),
        };
        let first_range = ranges.first().map_or(Flicks::ZERO, |r| r.0);
        let primary_video = doc.tracks.iter().position(|t| t.kind == TrackKind::Video);
        for &index in &chosen {
            let track = &doc.tracks[index];
            // A track whose clips would be cut or moved must be unlocked.
            if track.clips.iter().any(|c| c.end() > first_range) {
                ensure_unlocked(track)?;
            }
            let cuts_footage = track
                .clips
                .iter()
                .any(|c| ranges.iter().any(|&(a, b)| a < c.end() && c.start < b));
            if Some(index) == primary_video && cuts_footage {
                ctx.require_destructive(
                    "Removing time from the primary video track",
                    json!({
                        "track": track.id,
                        "ranges": ranges.iter().map(|&(a, b)| json!({
                            "start": TimeReport::new(a, rate),
                            "end": TimeReport::new(b, rate),
                        })).collect::<Vec<_>>(),
                    }),
                )?;
            }
        }
        let duration_before = doc.duration();
        let mut report = RangesRemoved {
            timeline: timeline.clone(),
            ranges: ranges
                .iter()
                .map(|&(a, b)| RangeReport {
                    start: TimeReport::new(a, rate),
                    end: TimeReport::new(b, rate),
                })
                .collect(),
            removed: TimeReport::new(
                ranges
                    .iter()
                    .fold(Flicks::ZERO, |sum, &(a, b)| sum + (b - a)),
                rate,
            ),
            duration_before: TimeReport::new(duration_before, rate),
            duration_after: TimeReport::new(duration_before, rate),
            clips_removed: Vec::new(),
            clips_changed: Vec::new(),
            clips_added: Vec::new(),
        };
        for index in chosen {
            cut_track(
                ctx.project,
                &timeline,
                index,
                &ranges,
                self.ripple,
                &mut report,
            )?;
        }
        let after = ctx.project.timelines[&timeline].duration();
        report.duration_after = TimeReport::new(after, rate);
        Ok(report)
    }
}

/// Cuts `ranges` out of one track, splitting clips a range falls inside, and moves what
/// follows left when rippling.
fn cut_track(
    project: &mut Project,
    timeline: &Id,
    index: usize,
    ranges: &[(Flicks, Flicks)],
    ripple: bool,
    report: &mut RangesRemoved,
) -> Result<(), ApiError> {
    let clips = std::mem::take(&mut timeline_mut(project, timeline)?.tracks[index].clips);
    let mut out = Vec::with_capacity(clips.len());
    for clip in clips {
        let parts = kept(clip.start, clip.end(), ranges);
        let untouched = parts.len() == 1 && parts[0] == (clip.start, clip.end());
        let mut first = true;
        for (from, to) in parts {
            let offset = from - clip.start;
            let content = match &clip.content {
                ClipContent::Video { asset, source_in } => ClipContent::Video {
                    asset: asset.clone(),
                    source_in: *source_in + offset,
                },
                ClipContent::Audio { asset, source_in } => ClipContent::Audio {
                    asset: asset.clone(),
                    source_in: *source_in + offset,
                },
                ClipContent::Caption(caption) => {
                    match window_caption(caption, offset, to - clip.start) {
                        Some(caption) => ClipContent::Caption(caption),
                        // A piece of a caption page with no words left is dropped.
                        None => continue,
                    }
                }
                other => other.clone(),
            };
            let id = if first {
                clip.id.clone()
            } else {
                let id = project.allocate_id(IdKind::Clip);
                report.clips_added.push(id.clone());
                id
            };
            first = false;
            let shift = if ripple {
                removed_before(from, ranges)
            } else {
                Flicks::ZERO
            };
            if !(untouched && shift == Flicks::ZERO) && id == clip.id {
                report.clips_changed.push(id.clone());
            }
            let mut part = fosscut_model::Clip {
                id,
                start: from - shift,
                duration: to - from,
                content,
                ..clip.clone()
            };
            // Keyframes stay with the content; fades stay at the clip's outer edges.
            part.shift_keyframes(Flicks::ZERO - offset);
            if from != clip.start {
                part.audio.fade_in = Flicks::ZERO;
            }
            if to != clip.end() {
                part.audio.fade_out = Flicks::ZERO;
            }
            out.push(part);
        }
        if first {
            report.clips_removed.push(clip.id.clone());
        }
    }
    out.sort_by_key(|c| c.start);
    timeline_mut(project, timeline)?.tracks[index].clips = out;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: i64) -> Flicks {
        Flicks::from_millis(n)
    }

    #[test]
    fn kept_parts_and_shifts_follow_the_ranges() {
        let ranges = [(ms(1000), ms(2000)), (ms(3000), ms(3500))];
        assert_eq!(
            kept(ms(0), ms(4000), &ranges),
            [
                (ms(0), ms(1000)),
                (ms(2000), ms(3000)),
                (ms(3500), ms(4000))
            ]
        );
        assert_eq!(kept(ms(1200), ms(1800), &ranges), []);
        assert_eq!(kept(ms(4000), ms(5000), &ranges), [(ms(4000), ms(5000))]);
        assert_eq!(removed_before(ms(500), &ranges), ms(0));
        assert_eq!(removed_before(ms(1500), &ranges), ms(500));
        assert_eq!(removed_before(ms(3500), &ranges), ms(1500));
        assert_eq!(removed_before(ms(9000), &ranges), ms(1500));
    }

    #[test]
    fn fill_covers_the_canvas() {
        // A 16:9 picture on a 9:16 canvas: fitted to the width, it must grow by (16/9)^2.
        let scale = fill_scale((1920, 1080), (1080, 1920));
        assert!((scale - (16.0f64 / 9.0).powi(2)).abs() < 1e-9);
        assert!((fill_scale((1920, 1080), (1920, 1080)) - 1.0).abs() < 1e-12);
        assert!((fill_scale((1080, 1080), (1080, 1920)) - 16.0 / 9.0).abs() < 1e-9);
    }
}
