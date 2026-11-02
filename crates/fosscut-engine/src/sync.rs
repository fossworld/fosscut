//! Sound from a separate recorder, synced to the camera (ADR-0034, measured edits; ADR-0029
//! for the measurement). [`sync_audio`] measures how far the recording lags the camera's
//! own sound ([`crate::motion::sync_offsets`]), then, for every clip of the camera asset on
//! a timeline, places the part of the recording that was heard at the same moment on an
//! audio track, silences the camera clip and groups the two (ADR-0024), so they move, trim
//! and split together.
//!
//! Like the other planners it returns ordinary commands; ids of the track, clips and groups
//! the commands make are predicted from the project's next id, so front ends apply them at
//! the revision they were planned at. A doubtful match changes nothing.

use std::path::Path;

use fosscut_model::{Clip, ClipContent, Id, IdKind, Project, TrackKind};
use fosscut_time::{Flicks, Rate};
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;
use crate::motion::{SYNC_CONFIDENCE, sync_offsets};

/// What to sync.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncRequest {
    /// The camera's video asset, whose own sound is the reference.
    pub camera: Id,
    /// The separate recording (an audio or video asset with sound).
    pub recording: Id,
    /// The timeline whose camera clips get the recording; the primary one when `None`.
    pub timeline: Option<Id>,
    /// The audio track for the recording; a new one when `None`.
    pub track: Option<Id>,
    /// Look at most this many seconds either way; any offset when 0.
    pub max_offset_seconds: f64,
}

/// A camera clip and the recording placed with it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SyncPair {
    /// The camera clip, now silent.
    pub camera_clip: Id,
    /// The recording's clip.
    pub recording_clip: Id,
    /// The group holding them.
    pub group: Id,
}

/// A camera clip left alone, and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Skipped {
    /// The clip.
    pub clip: Id,
    /// Why.
    pub reason: String,
}

/// What [`sync_audio`] measured, with the commands that place the recording.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SyncPlan {
    /// The camera asset.
    pub camera: Id,
    /// The recording asset.
    pub recording: Id,
    /// How far the recording lags the camera: its sound at time `t` is the camera's at
    /// `t + offset` (negative when the recording started first).
    pub offset: Flicks,
    /// The same in seconds.
    pub offset_seconds: f64,
    /// How sure the match is, from 0 to 1.
    pub confidence: f64,
    /// How far the placed recording is from the exact offset, in seconds (clips read their
    /// media from frames, so it is at most half a frame).
    pub residual_seconds: f64,
    /// The timeline.
    pub timeline: Id,
    /// The audio track the recording goes on.
    pub track: Id,
    /// Whether that track is added.
    pub new_track: bool,
    /// The project revision the commands are for.
    pub revision: u64,
    /// Every camera clip with its recording.
    pub pairs: Vec<SyncPair>,
    /// Camera clips left alone.
    pub skipped: Vec<Skipped>,
    /// `track.add`, then per camera clip `clip.add`, the camera's volume to 0 and
    /// `clip.group`.
    pub commands: Vec<Value>,
}

fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::InvalidArguments {
        message: message.into(),
    }
}

fn call(command: &str, args: Value) -> Value {
    json!({ "command": command, "args": args })
}

/// The first frame edge at or after `time`.
fn frame_ceil(rate: Rate, time: Flicks) -> Flicks {
    let frame = rate.frame_at(time);
    let at = rate.frames_to_flicks(frame);
    if at < time {
        rate.frames_to_flicks(frame + 1)
    } else {
        at
    }
}

/// The last frame edge at or before `time`.
fn frame_floor(rate: Rate, time: Flicks) -> Flicks {
    rate.frames_to_flicks(rate.frame_at(time))
}

/// Measures the recording against the camera's sound and plans its placement under every
/// clip of the camera asset on the timeline. Fails with `fosscut.audio.sync_uncertain`,
/// planning nothing, when the match is less sure than [`SYNC_CONFIDENCE`].
pub fn sync_audio(
    project: &Project,
    bundle: &Path,
    request: &SyncRequest,
) -> Result<SyncPlan, EngineError> {
    if request.camera == request.recording {
        return Err(invalid(
            "audio sync: the camera and the recording are one asset",
        ));
    }
    let camera = project
        .media
        .get(&request.camera)
        .ok_or_else(|| EngineError::AssetNotFound {
            asset: request.camera.to_string(),
        })?;
    if camera.info.video.is_none() {
        return Err(invalid(format!(
            "audio sync: {} is not a video; give the camera's video as camera",
            request.camera
        )));
    }
    let recording_duration = project
        .media
        .get(&request.recording)
        .ok_or_else(|| EngineError::AssetNotFound {
            asset: request.recording.to_string(),
        })?
        .info
        .duration
        .unwrap_or_default();
    let timeline_id = request
        .timeline
        .clone()
        .unwrap_or_else(|| project.manifest.primary_timeline.clone());
    let timeline =
        project
            .timelines
            .get(&timeline_id)
            .ok_or_else(|| EngineError::TimelineNotFound {
                timeline: timeline_id.to_string(),
            })?;
    let shows_camera = |clip: &&Clip| matches!(&clip.content, ClipContent::Video { asset, .. } if *asset == request.camera);
    let camera_clips: Vec<&Clip> = timeline
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter(shows_camera)
        .collect();
    if camera_clips.is_empty() {
        return Err(invalid(format!(
            "audio sync: no clip on {timeline_id} shows {}; place the camera's video first",
            request.camera
        )));
    }
    if let Some(track) = &request.track {
        let found = timeline
            .track(track)
            .ok_or_else(|| invalid(format!("audio sync: track {track} is not on {timeline_id}")))?;
        if found.kind != TrackKind::Audio {
            return Err(invalid(format!(
                "audio sync: {track} is a {:?} track; the recording needs an audio track",
                found.kind
            )));
        }
    }

    let measured = sync_offsets(
        project,
        bundle,
        &[request.camera.clone(), request.recording.clone()],
        request.max_offset_seconds,
    )?;
    let offset = measured.offsets.get(1).copied().unwrap_or_default();
    let confidence = measured.confidence.get(1).copied().unwrap_or_default();
    if confidence < SYNC_CONFIDENCE {
        return Err(EngineError::SyncUncertain {
            offset_seconds: offset.as_seconds_f64(),
            confidence,
        });
    }

    // Ids the commands will make, in the order they make them.
    let mut next = project.manifest.next_id;
    let mut allocate = |kind: IdKind| {
        let id = Id::new(kind, next);
        next += 1;
        id
    };
    let mut commands = Vec::new();
    let (track, new_track) = match &request.track {
        Some(track) => (track.clone(), false),
        None => (allocate(IdKind::Track), true),
    };
    let rate = timeline.format.rate;
    let mut pairs = Vec::new();
    let mut skipped = Vec::new();
    for clip in camera_clips {
        let skip = |reason: &str| Skipped {
            clip: clip.id.clone(),
            reason: reason.to_string(),
        };
        if clip.speed.as_ref().is_some_and(|s| !s.is_normal()) {
            skipped.push(skip(
                "it plays at another speed; give the recording's clip the same speed by hand",
            ));
            continue;
        }
        let source_in = clip.content.source_in().unwrap_or_default();
        // The recording heard at the camera's moment s is at s - offset; clips start and
        // read their media on frames, so it lines up to the nearest frame.
        let mut start = clip.start;
        let mut recording_in = rate.snap(source_in - offset);
        if recording_in.is_negative() {
            let shifted = frame_ceil(rate, clip.start - recording_in);
            recording_in = recording_in + (shifted - clip.start);
            start = shifted;
        }
        let covered = frame_floor(rate, start + (recording_duration - recording_in));
        let end = clip.end().min(covered);
        if end <= start {
            skipped.push(skip("the recording does not cover it"));
            continue;
        }
        let recording_clip = allocate(IdKind::Clip);
        commands.push(call(
            "clip.add",
            json!({
                "asset": request.recording,
                "timeline": timeline_id,
                "track": track,
                "start": { "flicks": start.get() },
                "source_in": { "flicks": recording_in.get() },
                "duration": { "flicks": (end - start).get() },
            }),
        ));
        if clip.keyframes.contains_key("volume") {
            commands.push(call(
                "keyframe.set_all",
                json!({ "clip": clip.id, "property": "volume", "keys": [] }),
            ));
        }
        commands.push(call(
            "clip.set",
            json!({ "clip": clip.id, "set": { "volume": 0.0 } }),
        ));
        // A camera clip already in a group keeps its partners: the recording joins them.
        let mut members: Vec<Id> = match &clip.group {
            Some(group) => timeline
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .filter(|c| c.group.as_ref() == Some(group))
                .map(|c| c.id.clone())
                .collect(),
            None => vec![clip.id.clone()],
        };
        members.push(recording_clip.clone());
        let group = allocate(IdKind::Group);
        commands.push(call("clip.group", json!({ "clips": members })));
        pairs.push(SyncPair {
            camera_clip: clip.id.clone(),
            recording_clip,
            group,
        });
    }
    if new_track && !pairs.is_empty() {
        commands.insert(
            0,
            call(
                "track.add",
                json!({ "kind": "audio", "timeline": timeline_id, "name": "Recording" }),
            ),
        );
    }
    Ok(SyncPlan {
        camera: request.camera.clone(),
        recording: request.recording.clone(),
        offset,
        offset_seconds: (offset.as_seconds_f64() * 1e6).round() / 1e6,
        confidence,
        residual_seconds: {
            let exact = Flicks::ZERO - offset;
            let placed = rate.snap(exact);
            ((placed - exact).as_seconds_f64() * 1e6).round() / 1e6
        },
        timeline: timeline_id,
        track,
        new_track: new_track && !pairs.is_empty(),
        revision: project.manifest.revision,
        pairs,
        skipped,
        commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_edges_round_the_right_way() {
        let rate = Rate::FPS_30;
        let frame = rate.frame_duration();
        assert_eq!(frame_ceil(rate, Flicks::new(1)), frame);
        assert_eq!(frame_ceil(rate, frame), frame);
        assert_eq!(frame_floor(rate, frame + Flicks::new(1)), frame);
        assert_eq!(frame_floor(rate, frame), frame);
    }
}
