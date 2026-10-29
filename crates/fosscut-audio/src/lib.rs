//! Timeline audio for FossCut (ADR-0008, item 5; first step).
//!
//! [`mix`] renders a span of a timeline to interleaved `f32` at a fixed rate and channel
//! count: audio clips and the embedded audio of video clips on unmuted tracks, each scaled
//! by its clip's `volume`, placed to the sample. Sources come from an [`AudioSource`], so
//! the mixer is independent of decoders and fully testable. Fades, pan, effects, buses and
//! loudness normalization come later.

#![forbid(unsafe_code)]

use fosscut_media::AudioBuffer;
use fosscut_model::{AssetKind, ClipContent, Id, Project, TrackKind};
use fosscut_time::{FLICKS_PER_SECOND, Flicks};
use thiserror::Error;

/// Errors while mixing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AudioError {
    /// The timeline to mix does not exist.
    #[error("timeline {timeline} does not exist")]
    TimelineNotFound {
        /// The requested timeline.
        timeline: String,
    },
    /// A clip's audio could not be read.
    #[error("cannot read audio of asset {asset}: {message}")]
    SourceFailed {
        /// The asset.
        asset: String,
        /// What went wrong.
        message: String,
    },
}

impl AudioError {
    /// The stable error code used in FossCut API responses (ADR-0006).
    pub fn code(&self) -> &'static str {
        match self {
            AudioError::TimelineNotFound { .. } => "fosscut.timeline.not_found",
            AudioError::SourceFailed { .. } => "fosscut.audio.source_failed",
        }
    }
}

/// Supplies decoded audio for assets.
pub trait AudioSource {
    /// `duration` of `asset`'s audio from `start`, at `sample_rate` with `channels`
    /// channels, exactly `duration` long (silence outside the asset).
    fn read(
        &mut self,
        asset: &Id,
        start: Flicks,
        duration: Flicks,
        sample_rate: u32,
        channels: u16,
    ) -> Result<AudioBuffer, AudioError>;
}

/// Number of sample frames in `time` at `sample_rate`, rounded to the nearest frame. Exact
/// for the common rates, which all divide a second of flicks.
pub fn samples_in(time: Flicks, sample_rate: u32) -> i64 {
    let numerator = i128::from(time.get()) * i128::from(sample_rate);
    let denominator = i128::from(FLICKS_PER_SECOND);
    let rounded = (2 * numerator + denominator).div_euclid(2 * denominator);
    i64::try_from(rounded).unwrap_or(if rounded < 0 { i64::MIN } else { i64::MAX })
}

/// Mixes `duration` of `timeline` from `start` at `sample_rate` with `channels` channels.
/// The result is exactly `duration` long; samples beyond full scale are clamped.
pub fn mix(
    project: &Project,
    timeline: &Id,
    start: Flicks,
    duration: Flicks,
    sample_rate: u32,
    channels: u16,
    source: &mut dyn AudioSource,
) -> Result<AudioBuffer, AudioError> {
    let doc = project
        .timelines
        .get(timeline)
        .ok_or_else(|| AudioError::TimelineNotFound {
            timeline: timeline.to_string(),
        })?;
    let channels_n = usize::from(channels);
    let first = samples_in(start, sample_rate);
    let frames = (samples_in(start + duration, sample_rate) - first).max(0) as usize;
    let mut out = AudioBuffer::silence(sample_rate, channels, frames);
    let end = start + duration;
    for track in doc.tracks.iter().filter(|t| !t.muted) {
        for clip in &track.clips {
            let (asset, source_in) = match (&clip.content, track.kind) {
                (ClipContent::Audio { asset, source_in }, TrackKind::Audio) => (asset, *source_in),
                (ClipContent::Video { asset, source_in }, TrackKind::Video)
                    if has_audio(project, asset) =>
                {
                    (asset, *source_in)
                }
                _ => continue,
            };
            let gain = clip.volume as f32;
            let (from, to) = (clip.start.max(start), clip.end().min(end));
            if from >= to || gain == 0.0 {
                continue;
            }
            let audio = source.read(
                asset,
                source_in + (from - clip.start),
                to - from,
                sample_rate,
                channels,
            )?;
            // Positions in samples on the timeline, so clips meet without gaps or overlaps.
            let offset = (samples_in(from, sample_rate) - first).max(0) as usize;
            let count =
                (samples_in(to, sample_rate) - samples_in(from, sample_rate)).max(0) as usize;
            for i in 0..count.min(audio.frames()) {
                let target = offset + i;
                if target >= frames {
                    break;
                }
                for c in 0..channels_n {
                    out.samples[target * channels_n + c] +=
                        audio.samples[i * channels_n + c] * gain;
                }
            }
        }
    }
    for sample in &mut out.samples {
        *sample = sample.clamp(-1.0, 1.0);
    }
    Ok(out)
}

fn has_audio(project: &Project, asset: &Id) -> bool {
    project
        .media
        .get(asset)
        .is_some_and(|a| a.kind == AssetKind::Audio || a.has_audio())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use fosscut_model::{
        Asset, AudioInfo, Clip, IdKind, MediaInfo, PathHints, TimelineFormat, Track, Transform,
    };
    use fosscut_time::Rate;

    const RATE: u32 = 48_000;

    fn frames(n: i64) -> Flicks {
        Rate::FPS_30.frames_to_flicks(n)
    }

    /// Each asset is a constant level per channel, so sums and gains are exact.
    struct Levels {
        levels: HashMap<Id, f32>,
        reads: Vec<(Id, Flicks, Flicks)>,
    }

    impl AudioSource for Levels {
        fn read(
            &mut self,
            asset: &Id,
            start: Flicks,
            duration: Flicks,
            sample_rate: u32,
            channels: u16,
        ) -> Result<AudioBuffer, AudioError> {
            self.reads.push((asset.clone(), start, duration));
            let level = *self
                .levels
                .get(asset)
                .ok_or_else(|| AudioError::SourceFailed {
                    asset: asset.to_string(),
                    message: "unknown".into(),
                })?;
            let frames = samples_in(duration, sample_rate) as usize;
            Ok(AudioBuffer {
                sample_rate,
                channels,
                samples: vec![level; frames * usize::from(channels)],
            })
        }
    }

    fn asset(project: &mut Project, kind: AssetKind, audio: bool) -> Id {
        let id = project.allocate_id(IdKind::Asset);
        project.media.assets.push(Asset {
            id: id.clone(),
            kind,
            name: "a".into(),
            path: PathHints::default(),
            identity: None,
            info: MediaInfo {
                duration: Some(frames(600)),
                container: None,
                video: None,
                audio: audio.then_some(AudioInfo {
                    sample_rate: 48_000,
                    channels: 2,
                    codec: None,
                }),
            },
        });
        id
    }

    fn clip(
        project: &mut Project,
        content: ClipContent,
        start: i64,
        length: i64,
        volume: f64,
    ) -> Clip {
        Clip {
            id: project.allocate_id(IdKind::Clip),
            start: frames(start),
            duration: frames(length),
            content,
            transform: Transform::default(),
            opacity: 1.0,
            volume,
        }
    }

    fn track(project: &mut Project, kind: TrackKind, muted: bool, clips: Vec<Clip>) {
        let id = project.allocate_id(IdKind::Track);
        let timeline = project.manifest.primary_timeline.clone();
        project
            .timelines
            .get_mut(&timeline)
            .unwrap()
            .tracks
            .push(Track {
                id,
                kind,
                name: None,
                muted,
                hidden: false,
                locked: false,
                clips,
            });
    }

    fn setup() -> (Project, Id, Levels) {
        let format = TimelineFormat {
            width: 1920,
            height: 1080,
            rate: Rate::FPS_30,
            sample_rate: 48_000,
        };
        let mut project = Project::new(Id::new(IdKind::Project, 1), "Mix", format);
        let video = asset(&mut project, AssetKind::Video, true);
        let silent_video = asset(&mut project, AssetKind::Video, false);
        let music = asset(&mut project, AssetKind::Audio, true);
        let base = vec![
            clip(
                &mut project,
                ClipContent::Video {
                    asset: video.clone(),
                    source_in: frames(30),
                },
                0,
                60,
                1.0,
            ),
            clip(
                &mut project,
                ClipContent::Video {
                    asset: silent_video.clone(),
                    source_in: Flicks::ZERO,
                },
                60,
                30,
                1.0,
            ),
        ];
        track(&mut project, TrackKind::Video, false, base);
        let bed = vec![clip(
            &mut project,
            ClipContent::Audio {
                asset: music.clone(),
                source_in: Flicks::ZERO,
            },
            30,
            60,
            0.5,
        )];
        track(&mut project, TrackKind::Audio, false, bed);
        let muted = vec![clip(
            &mut project,
            ClipContent::Audio {
                asset: music.clone(),
                source_in: Flicks::ZERO,
            },
            0,
            90,
            1.0,
        )];
        track(&mut project, TrackKind::Audio, true, muted);
        let timeline = project.manifest.primary_timeline.clone();
        let levels = Levels {
            levels: HashMap::from([(video, 0.4), (music, 0.4)]),
            reads: Vec::new(),
        };
        (project, timeline, levels)
    }

    fn level_at(buffer: &AudioBuffer, frame: i64) -> f32 {
        let index = samples_in(frames(frame), RATE) as usize;
        buffer.samples[index * 2]
    }

    #[test]
    fn clips_are_placed_to_the_sample_with_their_gain() {
        let (project, timeline, mut source) = setup();
        let out = mix(
            &project,
            &timeline,
            Flicks::ZERO,
            frames(100),
            RATE,
            2,
            &mut source,
        )
        .unwrap();
        assert_eq!(
            out.frames(),
            100 * 1_600,
            "1,600 samples per frame at 30 fps"
        );
        assert_eq!(level_at(&out, 10), 0.4, "video audio alone");
        assert_eq!(level_at(&out, 45), 0.4 + 0.2, "plus the bed at half volume");
        assert_eq!(level_at(&out, 75), 0.2, "silent video adds nothing");
        assert_eq!(level_at(&out, 95), 0.0, "after every clip");
        // The boundary sample at frame 30 already includes the bed; the one before does not.
        let boundary = samples_in(frames(30), RATE) as usize;
        assert_eq!(out.samples[(boundary - 1) * 2], 0.4);
        assert_eq!(out.samples[boundary * 2], 0.6);
        // Source times follow source_in plus the offset into the clip.
        assert!(
            source
                .reads
                .iter()
                .any(|(_, start, _)| *start == frames(30))
        );
    }

    #[test]
    fn windows_start_anywhere_and_muted_tracks_are_silent() {
        let (project, timeline, mut source) = setup();
        let out = mix(
            &project,
            &timeline,
            frames(40),
            frames(10),
            RATE,
            2,
            &mut source,
        )
        .unwrap();
        assert_eq!(out.frames(), 16_000);
        assert!(out.samples.iter().all(|s| (*s - 0.6).abs() < 1e-6));
        // The read for the video clip starts 40 frames in: source_in 30 + 40.
        assert!(
            source
                .reads
                .iter()
                .any(|(_, start, d)| *start == frames(70) && *d == frames(10))
        );
        assert!(
            source.reads.iter().all(|(_, _, d)| *d <= frames(10)),
            "only the window is read"
        );
    }

    #[test]
    fn loud_sums_are_clamped_and_mono_works() {
        let (mut project, timeline, mut source) = setup();
        let loud = project.media.assets[2].id.clone();
        source.levels.insert(loud.clone(), 0.9);
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks[1].clips[0].volume = 2.0;
        let out = mix(
            &project,
            &timeline,
            frames(45),
            frames(1),
            RATE,
            1,
            &mut source,
        )
        .unwrap();
        assert_eq!((out.channels, out.frames()), (1, 1_600));
        assert!(
            out.samples.iter().all(|s| *s == 1.0),
            "0.4 + 1.8 clamps to full scale"
        );
    }

    #[test]
    fn errors_are_typed() {
        let (project, _, mut source) = setup();
        let missing = Id::new(IdKind::Timeline, 9);
        let error = mix(
            &project,
            &missing,
            Flicks::ZERO,
            frames(1),
            RATE,
            2,
            &mut source,
        )
        .unwrap_err();
        assert_eq!(error.code(), "fosscut.timeline.not_found");
        let (project, timeline, mut source) = setup();
        source.levels.clear();
        let error = mix(
            &project,
            &timeline,
            Flicks::ZERO,
            frames(1),
            RATE,
            2,
            &mut source,
        )
        .unwrap_err();
        assert_eq!(error.code(), "fosscut.audio.source_failed");
        assert!(error.to_string().contains("asset_"));
        assert_eq!(samples_in(Flicks::from_seconds(1).unwrap(), 44_100), 44_100);
        assert_eq!(
            mix(
                &project,
                &timeline,
                frames(5),
                Flicks::ZERO,
                RATE,
                2,
                &mut source
            )
            .unwrap()
            .frames(),
            0
        );
    }
}
