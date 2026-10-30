//! Loudness normalization (ADR-0026, item 7). Each clip's sound is measured as it plays
//! (speed, channels and sound effects applied, at volume 1, alone) with the ITU-R BS.1770
//! meter of `fosscut-analysis`. When its peaks leave room, a volume brings it to the target
//! loudness; when they do not, an `audio.limiter` raises it while holding peaks at the
//! ceiling, measured again with the limiter until it reaches the target. Front ends apply
//! the result as one request of `clip.set` and effect commands, so the change goes through
//! the command framework and is undoable.

use std::path::Path;

use fosscut_analysis::Loudness;
use fosscut_analysis::loudness::LoudnessMeter;
use fosscut_model::{ClipContent, Effect, Id, IdKind, ParamValue, Project};
use fosscut_time::Flicks;
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;
use crate::preview::Sound;

/// The loudness short-form platforms play at, in LUFS.
pub const DEFAULT_TARGET_LUFS: f64 = -14.0;
/// The highest sample peak a normalized clip may reach, in dBFS.
pub const DEFAULT_CEILING_DBFS: f64 = -1.0;
/// Gains beyond these, in dB, are not applied: louder would mostly raise noise.
const GAIN_RANGE: (f64, f64) = (-40.0, 20.0);
/// The limiter's gain range, as `audio.limiter` takes it.
const LIMITER_GAIN: (f64, f64) = (0.0, 24.0);
/// Close enough to the target, in LU.
const TOLERANCE: f64 = 0.1;
/// Sound is measured in blocks of this many seconds.
const BLOCK_SECONDS: i64 = 10;
const RATE: u32 = 48_000;

/// The limiter a normalization uses.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LimiterSettings {
    /// Gain before limiting, in dB.
    pub gain_db: f64,
    /// The highest sample peak, in dBFS.
    pub ceiling_dbfs: f64,
}

/// One clip's measurement and what levels it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Leveled {
    /// The clip.
    pub clip: Id,
    /// Its integrated loudness at volume 1 without a limiter, in LUFS; absent when it is
    /// silent or shorter than 400 ms, and then it is left alone.
    pub measured_lufs: Option<f64>,
    /// Its sample peak at volume 1, in dBFS.
    pub peak_dbfs: Option<f64>,
    /// The gain applied, by the volume or the limiter, in dB.
    pub gain_db: f64,
    /// Its new `volume`.
    pub volume: f64,
    /// The limiter that holds its peaks, when the volume alone could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limiter: Option<LimiterSettings>,
    /// The loudness it plays at afterwards, in LUFS.
    pub reached_lufs: Option<f64>,
    /// True when it falls short of the target: the ceiling held it back without a
    /// limiter, or the gain range ran out.
    pub limited: bool,
}

/// What `normalize` found, with the commands that apply it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Normalization {
    /// The target loudness, in LUFS.
    pub target_lufs: f64,
    /// The ceiling for peaks, in dBFS.
    pub ceiling_dbfs: f64,
    /// Every clip asked for.
    pub clips: Vec<Leveled>,
    /// `clip.set` and effect commands, as an edit request's `commands`.
    pub commands: Vec<Value>,
}

/// Measures `clips` of `project` and works out what brings each to `target_lufs` with
/// peaks at most `ceiling_dbfs`: a volume, or with `limit`, a limiter where the peaks
/// leave too little room.
pub fn normalize(
    project: &Project,
    bundle: &Path,
    clips: &[Id],
    target_lufs: f64,
    ceiling_dbfs: f64,
    limit: bool,
) -> Result<Normalization, EngineError> {
    if !(target_lufs.is_finite() && (-40.0..=0.0).contains(&target_lufs)) {
        return Err(EngineError::InvalidArguments {
            message: format!("the target must be from -40 to 0 LUFS, not {target_lufs}"),
        });
    }
    if !(ceiling_dbfs.is_finite() && (-12.0..=0.0).contains(&ceiling_dbfs)) {
        return Err(EngineError::InvalidArguments {
            message: format!("the ceiling must be from -12 to 0 dBFS, not {ceiling_dbfs}"),
        });
    }
    if clips.is_empty() {
        return Err(EngineError::InvalidArguments {
            message: "name at least one clip to normalize".to_string(),
        });
    }
    let mut sound = Sound::new();
    let mut leveled = Vec::with_capacity(clips.len());
    let mut commands = Vec::new();
    for id in clips {
        let mut alone = alone(project, id)?;
        let measured = alone.measure(&mut sound, bundle)?;
        let (Some(lufs), Some(peak)) = (measured.integrated_lufs, measured.sample_peak_dbfs) else {
            leveled.push(Leveled {
                clip: id.clone(),
                measured_lufs: measured.integrated_lufs,
                peak_dbfs: measured.sample_peak_dbfs,
                gain_db: 0.0,
                volume: clip_volume(project, id),
                limiter: None,
                reached_lufs: None,
                limited: false,
            });
            continue;
        };
        let wanted = target_lufs - lufs;
        let headroom = ceiling_dbfs - peak;
        if wanted <= headroom || !limit {
            let gain = wanted.min(headroom).clamp(GAIN_RANGE.0, GAIN_RANGE.1);
            let volume = round(10f64.powf(gain / 20.0) * 1000.0) / 1000.0;
            commands.push(json!({
                "command": "clip.set",
                "args": { "clip": id, "set": { "volume": volume } },
            }));
            // A limiter from an earlier normalization would raise it further.
            if let Some(effect) = &alone.limiter {
                commands.push(json!({
                    "command": "effect.remove",
                    "args": { "clip": id, "effect": effect },
                }));
            }
            leveled.push(Leveled {
                clip: id.clone(),
                measured_lufs: Some(lufs),
                peak_dbfs: Some(peak),
                gain_db: round(gain),
                volume,
                limiter: None,
                reached_lufs: Some(round(lufs + gain)),
                limited: (gain - wanted).abs() > 0.005,
            });
            continue;
        }
        // Limiting takes some loudness off the peaks: measure again and make up for it.
        let mut gain = wanted.clamp(LIMITER_GAIN.0, LIMITER_GAIN.1);
        let mut reached = None;
        for _ in 0..4 {
            alone.limit(gain, ceiling_dbfs);
            reached = alone.measure(&mut sound, bundle)?.integrated_lufs;
            let Some(now) = reached else { break };
            let missing = target_lufs - now;
            let next = (gain + missing).clamp(LIMITER_GAIN.0, LIMITER_GAIN.1);
            if missing.abs() < TOLERANCE || (next - gain).abs() < 0.01 {
                break;
            }
            gain = next;
        }
        let gain = round(gain);
        let params = json!({ "gain": gain, "ceiling": ceiling_dbfs });
        commands.push(json!({
            "command": "clip.set",
            "args": { "clip": id, "set": { "volume": 1.0 } },
        }));
        commands.push(match &alone.limiter {
            Some(effect) => json!({
                "command": "effect.set",
                "args": { "clip": id, "effect": effect, "params": params },
            }),
            None => json!({
                "command": "effect.add",
                "args": { "clip": id, "type": "audio.limiter", "params": params },
            }),
        });
        leveled.push(Leveled {
            clip: id.clone(),
            measured_lufs: Some(lufs),
            peak_dbfs: Some(peak),
            gain_db: gain,
            volume: 1.0,
            limiter: Some(LimiterSettings {
                gain_db: gain,
                ceiling_dbfs,
            }),
            reached_lufs: reached.map(round),
            limited: reached.is_none_or(|r| target_lufs - r > 0.5),
        });
    }
    Ok(Normalization {
        target_lufs,
        ceiling_dbfs,
        clips: leveled,
        commands,
    })
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn clip_volume(project: &Project, id: &Id) -> f64 {
    project
        .timelines
        .values()
        .flat_map(|t| &t.tracks)
        .flat_map(|t| &t.clips)
        .find(|c| &c.id == id)
        .map_or(1.0, |c| c.volume)
}

/// A clip alone on its timeline, to measure.
struct Alone {
    project: Project,
    timeline: Id,
    track: usize,
    start: Flicks,
    duration: Flicks,
    /// The clip's own limiter, left out of the measurement.
    limiter: Option<Id>,
}

impl Alone {
    /// Its loudness as it plays.
    fn measure(&self, sound: &mut Sound, bundle: &Path) -> Result<Loudness, EngineError> {
        let mut meter = LoudnessMeter::new(2);
        let block = Flicks::from_seconds(BLOCK_SECONDS).unwrap_or(self.duration);
        let mut at = self.start;
        while at < self.start + self.duration {
            let length = block.min(self.start + self.duration - at);
            let mixed = sound.mix(&self.project, bundle, &self.timeline, at, length, RATE, 2)?;
            meter.push(&mixed.samples);
            at = at + length;
        }
        Ok(meter.finish())
    }

    /// Puts a limiter with `gain_db` and `ceiling_dbfs` last in the clip's effects.
    fn limit(&mut self, gain_db: f64, ceiling_dbfs: f64) {
        let Some(clip) = self
            .project
            .timelines
            .get_mut(&self.timeline)
            .and_then(|t| t.tracks.get_mut(self.track))
            .and_then(|t| t.clips.first_mut())
        else {
            return;
        };
        clip.effects.retain(|e| e.effect_type != "audio.limiter");
        clip.effects.push(Effect {
            id: Id::new(IdKind::Effect, u64::MAX >> 16),
            effect_type: "audio.limiter".into(),
            version: 1,
            enabled: true,
            params: [
                ("gain".to_string(), ParamValue::Number(gain_db)),
                ("ceiling".to_string(), ParamValue::Number(ceiling_dbfs)),
            ]
            .into_iter()
            .collect(),
            keyframes: Default::default(),
        });
    }
}

/// A copy of `project` where the clip's timeline holds only the clip, at volume 1 without
/// fades, transitions or its own limiter.
fn alone(project: &Project, id: &Id) -> Result<Alone, EngineError> {
    let found = project.timelines.iter().find_map(|(timeline, doc)| {
        doc.tracks.iter().enumerate().find_map(|(t, track)| {
            track
                .clips
                .iter()
                .position(|c| &c.id == id)
                .map(|c| (timeline.clone(), t, c))
        })
    });
    let Some((timeline_id, track_index, clip_index)) = found else {
        return Err(EngineError::InvalidArguments {
            message: format!("clip {id} does not exist"),
        });
    };
    let mut copy = project.clone();
    let Some(timeline) = copy.timelines.get_mut(&timeline_id) else {
        return Err(EngineError::TimelineNotFound {
            timeline: timeline_id.to_string(),
        });
    };
    let mut clip = timeline.tracks[track_index].clips[clip_index].clone();
    let sound = match &clip.content {
        ClipContent::Audio { .. } | ClipContent::Timeline { .. } => true,
        ClipContent::Video { asset, .. } => project.media.get(asset).is_some_and(|a| a.has_audio()),
        _ => false,
    };
    if !sound {
        return Err(EngineError::InvalidArguments {
            message: format!("{id} is a {} clip without sound", clip.content.kind_name()),
        });
    }
    if clip.keyframes.contains_key("volume") {
        return Err(EngineError::InvalidArguments {
            message: format!(
                "{id} has volume keyframes; remove them (keyframe.remove with all) before normalizing"
            ),
        });
    }
    let limiter = clip
        .effects
        .iter()
        .find(|e| e.effect_type == "audio.limiter")
        .map(|e| e.id.clone());
    clip.effects.retain(|e| e.effect_type != "audio.limiter");
    clip.volume = 1.0;
    clip.audio.fade_in = Flicks::ZERO;
    clip.audio.fade_out = Flicks::ZERO;
    clip.audio.detached = false;
    clip.transition = None;
    clip.enabled = true;
    let (start, duration) = (clip.start, clip.duration);
    let mut track = timeline.tracks[track_index].clone();
    track.muted = false;
    track.clips = vec![clip];
    timeline.tracks = vec![track];
    Ok(Alone {
        project: copy,
        timeline: timeline_id,
        track: 0,
        start,
        duration,
        limiter,
    })
}
