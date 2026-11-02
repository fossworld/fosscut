//! Punch-ins (ADR-0034, measured edits): a talking head zooms in for a moment and back out,
//! centred on the face. [`punch_in`] returns `keyframe.set_all` calls for the clip's
//! `transform.scale` (ease in, hold, ease out for every moment, the clip's own scale kept
//! elsewhere) and, while the clip is otherwise at its plain size, `transform.anchor` held on
//! the face during each moment: from the newest cached `pose` analysis when there is one, a
//! point the caller gives, or the centre of the picture's upper third. Nothing else changes.

use std::path::PathBuf;

use fosscut_analysis::framing;
use fosscut_model::{ClipContent, Id, Interpolation, KeyValue, Keyframe, Project};
use fosscut_render::geometry::clockwise_rotation;
use fosscut_time::Flicks;
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;

/// How far a punch-in zooms by default.
pub const DEFAULT_SCALE: f64 = 1.15;
/// How long a punch-in takes to zoom in, and to zoom back out, by default: 0.25 s.
pub const DEFAULT_EASE: Flicks = Flicks::from_millis(250);

/// One punch-in, in timeline time.
#[derive(Clone, Debug, PartialEq)]
pub struct Moment {
    /// Where it starts zooming in.
    pub start: Flicks,
    /// Where it is back at the clip's own size.
    pub end: Flicks,
    /// How far it zooms, relative to the clip's own scale: 0.5 to 4, not 1.
    pub scale: f64,
    /// How long zooming in and back out each take (at most half the moment, at least one
    /// frame).
    pub ease: Flicks,
}

/// What to punch in on.
#[derive(Clone, Debug, PartialEq)]
pub struct PunchRequest {
    /// The project bundle.
    pub bundle: PathBuf,
    /// The analysis cache, normally [`crate::cache_dir`].
    pub cache: PathBuf,
    /// The video clip.
    pub clip: Id,
    /// The moments, which must not overlap.
    pub moments: Vec<Moment>,
    /// The point to zoom on, as fractions of the picture from its left and top edges,
    /// instead of the face.
    pub focus: Option<[f64; 2]>,
}

/// One planned punch-in.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlannedMoment {
    /// Where it starts, in timeline time.
    pub start: Flicks,
    /// Where it ends.
    pub end: Flicks,
    /// Its zoom.
    pub scale: f64,
    /// The point it zooms on, as fractions of the picture from its left and top edges.
    pub focus: [f64; 2],
}

/// What [`punch_in`] planned.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PunchPlan {
    /// The clip.
    pub clip: Id,
    /// Where the zoom centre came from: `pose` (the face), `focus` (given), `default` (the
    /// upper third's centre) or `kept` (the clip is already scaled, so its anchor stays,
    /// since moving it would move the picture).
    pub anchor: &'static str,
    /// The punch-ins, in order.
    pub moments: Vec<PlannedMoment>,
    /// `keyframe.set_all` calls for `transform.scale` and, unless kept, `transform.anchor`.
    pub commands: Vec<Value>,
}

fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::InvalidArguments {
        message: message.into(),
    }
}

/// The picture's upper third's centre, where a face usually is.
const UPPER_THIRD: [f64; 2] = [0.5, 1.0 / 3.0];

fn round(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn easing(interpolation: Interpolation) -> Value {
    serde_json::to_value(interpolation).unwrap_or(Value::Null)
}

/// Plans the punch-ins of `request` on its clip.
pub fn punch_in(project: &Project, request: &PunchRequest) -> Result<PunchPlan, EngineError> {
    let (timeline, clip) = project
        .timelines
        .values()
        .find_map(|t| {
            t.tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|c| c.id == request.clip)
                .map(|c| (t, c))
        })
        .ok_or_else(|| invalid(format!("clip {} does not exist", request.clip)))?;
    let ClipContent::Video { asset, .. } = &clip.content else {
        return Err(invalid(format!(
            "punch in: {} is a {} clip; punch-ins zoom video clips",
            clip.id,
            clip.content.kind_name()
        )));
    };
    if request.moments.is_empty() {
        return Err(invalid("punch in: give at least one moment"));
    }
    // Keyframes fall on frames: the moments do too.
    let rate = timeline.format.rate;
    let frame = rate.frame_duration();
    let mut moments: Vec<Moment> = request
        .moments
        .iter()
        .map(|m| Moment {
            start: rate.snap(m.start),
            end: rate.snap(m.end),
            ..m.clone()
        })
        .collect();
    moments.sort_by_key(|m| m.start);
    for (index, m) in moments.iter().enumerate() {
        if m.start < clip.start || m.end > clip.end() || m.end < m.start + frame + frame {
            return Err(invalid(format!(
                "punch in: moment {index} ({} s to {} s) must lie inside {} ({} s to {} s) and last at least two frames",
                m.start.as_seconds_f64(),
                m.end.as_seconds_f64(),
                clip.id,
                clip.start.as_seconds_f64(),
                clip.end().as_seconds_f64()
            )));
        }
        if !(m.scale.is_finite() && (0.5..=4.0).contains(&m.scale)) || m.scale == 1.0 {
            return Err(invalid(format!(
                "punch in: scale {} is outside 0.5 to 4 (or 1, which does nothing)",
                m.scale
            )));
        }
        if m.ease.is_negative() {
            return Err(invalid("punch in: ease must be zero or more"));
        }
        if index > 0 && m.start < moments[index - 1].end {
            return Err(invalid(format!(
                "punch in: moments {} and {index} overlap",
                index - 1
            )));
        }
    }
    if let Some([x, y]) = request.focus
        && !(x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y))
    {
        return Err(invalid("punch in: focus is [x, y], each from 0 to 1"));
    }

    // Where each moment zooms: the given point, the face, or the upper third.
    let pose = if request.focus.is_none() {
        crate::measured::pose(project, &request.bundle, &request.cache, asset)?
    } else {
        None
    };
    let rotation = project
        .media
        .get(asset)
        .and_then(|a| a.info.video.as_ref())
        .map_or(0, |v| clockwise_rotation(v.rotation));
    let face_during = |m: &Moment| -> Option<[f64; 2]> {
        let track = pose.as_ref()?;
        let step = (m.end - m.start).get() / 8;
        let mut points: Vec<(f64, f64)> = (0..=8)
            .filter_map(|i| {
                let offset = m.start + Flicks::new(step * i) - clip.start;
                let source = clip.source_at(offset)?;
                let at = track.at(source)?;
                let subject = framing::from_pose(at.presence, &at.landmarks)?.turned(rotation);
                Some((subject.x, subject.y))
            })
            .collect();
        if points.is_empty() {
            return None;
        }
        let middle = points.len() / 2;
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let x = points[middle].0;
        points.sort_by(|a, b| a.1.total_cmp(&b.1));
        let y = points[middle].1;
        Some([x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)])
    };
    let mut source = match (&request.focus, &pose) {
        (Some(_), _) => "focus",
        (None, Some(_)) => "pose",
        (None, None) => "default",
    };
    let faces: Vec<Option<[f64; 2]>> = moments.iter().map(&face_during).collect();
    let fallback = faces.iter().flatten().next().copied();
    if source == "pose" && fallback.is_none() {
        source = "default";
    }
    let focus: Vec<[f64; 2]> = faces
        .iter()
        .map(|face| request.focus.or(*face).or(fallback).unwrap_or(UPPER_THIRD))
        .collect();

    // Scale keys: the clip's own scale (keyframed or not) outside the moments.
    let base = |offset: Flicks| clip.transform_at(offset).scale;
    let existing: Vec<Keyframe> = clip
        .keyframes
        .get("transform.scale")
        .cloned()
        .unwrap_or_default();
    let mut keys: Vec<Keyframe> = existing
        .into_iter()
        .filter(|k| {
            !moments
                .iter()
                .any(|m| k.time >= m.start - clip.start && k.time <= m.end - clip.start)
        })
        .collect();
    let ease_in_out = Interpolation::EASE_IN_OUT;
    for m in &moments {
        let (from, to) = (m.start - clip.start, m.end - clip.start);
        let half = rate.frames_to_flicks(rate.frame_at(to - from) / 2);
        let ease = rate.snap(m.ease).max(frame).min(half);
        let (before, after) = (base(from), base(to));
        let mut key = |time: Flicks, value: f64, interpolation: Interpolation| {
            keys.push(Keyframe {
                time,
                value: KeyValue::Number(round(value)),
                interpolation,
            });
        };
        key(from, before, ease_in_out);
        if from + ease < to - ease {
            key(
                from + ease,
                base(from + ease) * m.scale,
                Interpolation::Linear,
            );
            key(to - ease, base(to - ease) * m.scale, ease_in_out);
        } else {
            key(from + ease, base(from + ease) * m.scale, ease_in_out);
        }
        key(to, after, Interpolation::Linear);
    }
    keys.sort_by_key(|k| k.time);
    keys.dedup_by(|later, earlier| later.time == earlier.time);
    let as_calls = |keys: &[Keyframe]| -> Vec<Value> {
        keys.iter()
            .map(|k| {
                let value = match k.value {
                    KeyValue::Number(v) => json!(v),
                    KeyValue::Pair(p) => json!(p),
                };
                json!({ "offset": { "flicks": k.time.get() }, "value": value, "easing": easing(k.interpolation) })
            })
            .collect()
    };
    let mut commands = vec![json!({
        "command": "keyframe.set_all",
        "args": { "clip": clip.id, "property": "transform.scale", "keys": as_calls(&keys) },
    })];

    // The anchor turns the zoom around the face. It moves the picture wherever the clip
    // is scaled or turned, so it is only set on clips at their plain size and angle.
    let plain = !clip.keyframes.contains_key("transform.scale")
        && clip.transform.scale == 1.0
        && clip.transform.stretch == [1.0, 1.0]
        && clip.transform.rotation == 0.0
        && !clip.keyframes.contains_key("transform.rotation")
        && !clip.keyframes.contains_key("transform.stretch");
    if plain {
        let anchors: Vec<Keyframe> = moments
            .iter()
            .zip(&focus)
            .map(|(m, [x, y])| Keyframe {
                time: m.start - clip.start,
                value: KeyValue::Pair([round(x - 0.5), round(y - 0.5)]),
                interpolation: Interpolation::Hold,
            })
            .collect();
        commands.push(json!({
            "command": "keyframe.set_all",
            "args": { "clip": clip.id, "property": "transform.anchor", "keys": as_calls(&anchors) },
        }));
    } else {
        source = "kept";
    }
    Ok(PunchPlan {
        clip: clip.id.clone(),
        anchor: source,
        moments: moments
            .iter()
            .zip(&focus)
            .map(|(m, focus)| PlannedMoment {
                start: m.start,
                end: m.end,
                scale: m.scale,
                focus: focus.map(round),
            })
            .collect(),
        commands,
    })
}

/// The base scale of `clip` at `offset`, for tests.
#[cfg(test)]
fn scale_at(clip: &fosscut_model::Clip, offset: Flicks) -> f64 {
    clip.transform_at(offset).scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_upper_third_is_the_default_focus() {
        assert_eq!(UPPER_THIRD, [0.5, 1.0 / 3.0]);
        let clip = fosscut_model::Clip::new(
            Id::new(fosscut_model::IdKind::Clip, 1),
            Flicks::ZERO,
            Flicks::from_millis(1000),
            ClipContent::Video {
                asset: Id::new(fosscut_model::IdKind::Asset, 1),
                source_in: Flicks::ZERO,
            },
        );
        assert_eq!(scale_at(&clip, Flicks::ZERO), 1.0);
        assert_eq!(easing(Interpolation::Hold), json!("hold"));
    }
}
