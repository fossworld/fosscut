//! Timeline audio for FossCut (ADR-0008, item 5; first step).
//!
//! [`mix`] renders a span of a timeline to interleaved `f32` at a fixed rate and channel
//! count: audio clips and the embedded audio of video clips on unmuted tracks, placed to
//! the sample. Each enabled clip plays its chosen channels, scaled by its volume (keyframed
//! per sample) and its fades, and balanced by its pan. Sources come from an
//! [`AudioSource`], so the mixer is independent of decoders and fully testable. Clips with
//! speed are resampled or time-stretched (ADR-0023). Effects, buses and loudness
//! normalization come later.

#![forbid(unsafe_code)]

pub mod effects;
mod stretch;
pub mod synth;
pub mod voice;
pub mod wav;

use fosscut_media::AudioBuffer;
use fosscut_model::{
    AssetKind, ChannelMode, Clip, ClipContent, Id, ParamValue, Project, TrackKind,
};
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
        for (index, clip) in track.clips.iter().enumerate() {
            let (asset, source_in) = match (&clip.content, track.kind) {
                (ClipContent::Audio { asset, source_in }, TrackKind::Audio) => (asset, *source_in),
                // Detached sound plays from its extracted audio clip instead (ADR-0024).
                (ClipContent::Video { asset, source_in }, TrackKind::Video)
                    if has_audio(project, asset) && !clip.audio.detached =>
                {
                    (asset, *source_in)
                }
                // A compound clip sounds like its timeline's mix (ADR-0025).
                (
                    ClipContent::Timeline {
                        timeline,
                        source_in,
                    },
                    TrackKind::Video,
                ) if !clip.audio.detached => (timeline, *source_in),
                _ => continue,
            };
            let mut nested = clip.content.nested().map(|timeline| Nested {
                project,
                timeline,
                inner: &mut *source,
            });
            let reader: &mut dyn AudioSource = match &mut nested {
                Some(nested) => nested,
                None => &mut *source,
            };
            if !clip.enabled {
                continue;
            }
            // A transition plays both clips over the cut: the sound crossfades there.
            let (lead, tail) = transition_halves(&track.clips, index);
            let (from, to) = ((clip.start - lead).max(start), (clip.end() + tail).min(end));
            if from >= to || silent(clip) {
                continue;
            }
            // Before the media's start there is nothing to read: begin later, in silence.
            let wanted = source_in + (from - clip.start);
            let skip = if wanted.is_negative() && clip.speed.is_none() {
                Flicks::ZERO - wanted
            } else {
                Flicks::ZERO
            };
            let read_from = from + skip;
            if read_from >= to {
                continue;
            }
            // Positions in samples on the timeline, so clips meet without gaps or overlaps.
            let offset = (samples_in(read_from, sample_rate) - first).max(0) as usize;
            let count =
                (samples_in(to, sample_rate) - samples_in(read_from, sample_rate)).max(0) as usize;
            let clip_first = samples_in(read_from - clip.start, sample_rate);
            let chain = sound_effects(clip);
            let audio = if chain.is_empty() {
                clip_samples(
                    reader,
                    asset,
                    clip,
                    source_in,
                    clip_first,
                    count,
                    sample_rate,
                    channels,
                )?
            } else {
                // Effects keep state: start them up to WARM_UP_SECONDS earlier (as much as
                // the chain remembers, never before the clip's start, where their state is
                // fresh) and give them their look-ahead, silence past the clip's end, so
                // every part of the clip sounds the same however the mix is cut into blocks
                // (ADR-0026, item 5).
                let warm = effects::warm_up_frames(&chain, sample_rate) as i64;
                let from = if clip_first <= 0 {
                    clip_first
                } else {
                    (clip_first - warm).max(0)
                };
                let last = samples_in(clip.duration + tail, sample_rate);
                let ahead = effects::lookahead_frames(&chain, sample_rate) as i64;
                let until =
                    (clip_first + count as i64 + ahead).min(last.max(clip_first + count as i64));
                let read = (until - from).max(0) as usize;
                let input = clip_samples(
                    reader,
                    asset,
                    clip,
                    source_in,
                    from,
                    read,
                    sample_rate,
                    channels,
                )?;
                let processed = effects::process_at(&chain, &input, channels, sample_rate, from);
                let skip = (clip_first - from) as usize * channels_n;
                processed[skip..skip + count * channels_n].to_vec()
            };
            let shaped = Shaping::of(clip);
            let mut frame = vec![0.0f32; channels_n];
            for i in 0..count.min(audio.len() / channels_n.max(1)) {
                let target = offset + i;
                if target >= frames {
                    break;
                }
                frame.copy_from_slice(&audio[i * channels_n..(i + 1) * channels_n]);
                let at = sample_time(clip_first + i as i64, sample_rate);
                shaped.apply(clip, at, &mut frame);
                let cross = crossfade(clip, at, lead, tail);
                for (c, value) in frame.iter().enumerate() {
                    out.samples[target * channels_n + c] += value * cross;
                }
            }
        }
    }
    for sample in &mut out.samples {
        *sample = sample.clamp(-1.0, 1.0);
    }
    Ok(out)
}

/// Exactly `count` sample frames of a clip's sound from clip sample `first` (which may be
/// negative within a transition's lead), through its speed when it has one.
#[allow(clippy::too_many_arguments)]
fn clip_samples(
    reader: &mut dyn AudioSource,
    asset: &Id,
    clip: &Clip,
    source_in: Flicks,
    first: i64,
    count: usize,
    sample_rate: u32,
    channels: u16,
) -> Result<Vec<f32>, AudioError> {
    if clip.speed.is_some() {
        return stretch::clip_audio(reader, asset, clip, first, count, sample_rate, channels);
    }
    let from = sample_time(first, sample_rate);
    let to = sample_time(first + count as i64, sample_rate);
    let mut samples = reader
        .read(asset, source_in + from, to - from, sample_rate, channels)?
        .samples;
    samples.resize(count * usize::from(channels), 0.0);
    Ok(samples)
}

/// A clip's enabled sound effects, in order, with their parameters (ADR-0026).
fn sound_effects(clip: &Clip) -> Vec<effects::AudioEffect> {
    clip.effects
        .iter()
        .filter(|e| e.enabled)
        .filter_map(|e| {
            let values = e.values_at(Flicks::ZERO)?;
            let number = |name: &str| values.get(name).and_then(ParamValue::number).unwrap_or(0.0);
            let amount = number("amount") as f32;
            match e.effect_type.as_str() {
                "audio.denoise" => Some(effects::AudioEffect::Denoise { amount }),
                "audio.enhance_voice" => Some(effects::AudioEffect::EnhanceVoice { amount }),
                "audio.limiter" => Some(effects::AudioEffect::Limiter {
                    gain_db: number("gain") as f32,
                    ceiling_db: number("ceiling") as f32,
                }),
                "audio.vocals" => Some(effects::AudioEffect::Vocals {
                    mode: match values.get("mode") {
                        Some(ParamValue::Name(mode)) if mode == "remove" => {
                            effects::VocalMode::Remove
                        }
                        _ => effects::VocalMode::Isolate,
                    },
                }),
                other => {
                    voice::VoiceEffect::from_values(other, &values).map(effects::AudioEffect::Voice)
                }
            }
        })
        .collect()
}

/// A compound clip's sound: the mix of the timeline it shows, read like an asset's.
struct Nested<'a> {
    project: &'a Project,
    timeline: &'a Id,
    inner: &'a mut dyn AudioSource,
}

impl AudioSource for Nested<'_> {
    fn read(
        &mut self,
        _asset: &Id,
        start: Flicks,
        duration: Flicks,
        sample_rate: u32,
        channels: u16,
    ) -> Result<AudioBuffer, AudioError> {
        mix(
            self.project,
            self.timeline,
            start,
            duration,
            sample_rate,
            channels,
            self.inner,
        )
    }
}

/// How far a clip's sound reaches before its start and after its end for transitions: half
/// of the previous clip's transition into it, and half of its own into the next clip.
fn transition_halves(clips: &[Clip], index: usize) -> (Flicks, Flicks) {
    let half = |out: &Clip, into: &Clip| match &out.transition {
        Some(t) if into.start == out.end() && t.duration.is_positive() => {
            Flicks::new(t.duration.get() / 2)
        }
        _ => Flicks::ZERO,
    };
    let lead = index
        .checked_sub(1)
        .map_or(Flicks::ZERO, |i| half(&clips[i], &clips[index]));
    let tail = clips
        .get(index + 1)
        .map_or(Flicks::ZERO, |next| half(&clips[index], next));
    (lead, tail)
}

/// The equal-power crossfade gain of a clip at `at` from its start: rising over the
/// `lead` around its start, falling over the `tail` around its end, 1 in between.
fn crossfade(clip: &Clip, at: Flicks, lead: Flicks, tail: Flicks) -> f32 {
    let quarter = std::f64::consts::FRAC_PI_2;
    let mut gain = 1.0f64;
    if lead.is_positive() && at < lead {
        let p = ((at + lead).get() as f64 / (2 * lead.get()) as f64).clamp(0.0, 1.0);
        gain *= (p * quarter).sin();
    }
    if tail.is_positive() && at > clip.duration - tail {
        let p =
            ((at - (clip.duration - tail)).get() as f64 / (2 * tail.get()) as f64).clamp(0.0, 1.0);
        gain *= (p * quarter).cos();
    }
    gain as f32
}

/// The time of sample `index` counted from a clip's start.
fn sample_time(index: i64, sample_rate: u32) -> Flicks {
    Flicks::new(
        i64::try_from(i128::from(index) * i128::from(FLICKS_PER_SECOND) / i128::from(sample_rate))
            .unwrap_or(i64::MAX),
    )
}

/// True when a clip cannot be heard at all: zero volume and no volume keyframes.
fn silent(clip: &Clip) -> bool {
    clip.volume == 0.0 && !clip.keyframes.contains_key("volume")
}

/// How a clip's sound is shaped, decided once per clip: which per-sample work it needs.
struct Shaping {
    /// Volume keyframes or fades change the gain over time.
    varying: bool,
    /// The constant gain when nothing varies.
    gain: f32,
}

impl Shaping {
    fn of(clip: &Clip) -> Shaping {
        let varying = clip.keyframes.contains_key("volume")
            || clip.keyframes.contains_key("audio.pan")
            || clip.audio.fade_in.is_positive()
            || clip.audio.fade_out.is_positive();
        Shaping {
            varying,
            gain: clip.volume.max(0.0) as f32,
        }
    }

    /// Shapes one sample frame of `clip` at `at` from its start, in place.
    fn apply(&self, clip: &Clip, at: Flicks, frame: &mut [f32]) {
        if frame.len() == 2 {
            let (l, r) = (frame[0], frame[1]);
            let (l, r) = match clip.audio.channels {
                ChannelMode::Stereo => (l, r),
                ChannelMode::Left => (l, l),
                ChannelMode::Right => (r, r),
                ChannelMode::Mono => ((l + r) / 2.0, (l + r) / 2.0),
            };
            let pan = if self.varying {
                clip.pan_at(at)
            } else {
                clip.audio.pan
            };
            let (pl, pr) = balance(pan);
            frame[0] = l * pl;
            frame[1] = r * pr;
        }
        let gain = if self.varying {
            (clip.volume_at(at) * clip.fade_at(at)) as f32
        } else {
            self.gain
        };
        for value in frame.iter_mut() {
            *value *= gain;
        }
    }
}

/// Left and right gains for a pan from -1 (left) to 1 (right): centred leaves both sides
/// unchanged, and moving to one side lowers the other on a quarter cosine.
pub fn balance(pan: f64) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    let lower = |amount: f64| (amount.max(0.0) * std::f64::consts::FRAC_PI_2).cos() as f32;
    (lower(pan), lower(-pan))
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
        Asset, AudioInfo, Clip, IdKind, MediaInfo, PathHints, TimelineFormat, Track,
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
            folder: None,
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
                lut: None,
                ..Default::default()
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
            volume,
            ..Clip::new(
                project.allocate_id(IdKind::Clip),
                frames(start),
                frames(length),
                content,
            )
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
                caption_style: None,
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
    fn compound_clips_sound_like_their_timeline() {
        let (mut project, timeline, mut source) = setup();
        // The whole edit inside a compound timeline, shown at half volume from frame 40.
        let inner_id = project.allocate_id(IdKind::Timeline);
        let mut inner = project.timelines[&timeline].clone();
        inner.id = inner_id.clone();
        project
            .manifest
            .timelines
            .push(fosscut_model::TimelineEntry {
                id: inner_id.clone(),
                name: "Inside".into(),
                role: fosscut_model::TimelineRole::Compound,
            });
        project.timelines.insert(inner_id.clone(), inner);
        let compound = clip(
            &mut project,
            ClipContent::Timeline {
                timeline: inner_id,
                source_in: frames(40),
            },
            0,
            50,
            0.5,
        );
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks.clear();
        track(&mut project, TrackKind::Video, false, vec![compound]);
        let out = mix(
            &project,
            &timeline,
            Flicks::ZERO,
            frames(60),
            RATE,
            2,
            &mut source,
        )
        .unwrap();
        // Frame 5 shows inside frame 45: the video's sound and the bed, halved.
        assert!(
            (level_at(&out, 5) - 0.3).abs() < 1e-6,
            "{}",
            level_at(&out, 5)
        );
        assert_eq!(level_at(&out, 55), 0.0, "after the compound clip");
    }

    #[test]
    fn detached_video_sound_is_not_heard() {
        let (mut project, timeline, mut source) = setup();
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks[0].clips[0].audio.detached = true;
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
            level_at(&out, 10),
            0.0,
            "its extracted clip plays it instead"
        );
        assert_eq!(level_at(&out, 45), 0.2, "the bed still plays");
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

    /// Every asset is a constant left and right level.
    struct Stereo(f32, f32);

    impl AudioSource for Stereo {
        fn read(
            &mut self,
            _asset: &Id,
            _start: Flicks,
            duration: Flicks,
            sample_rate: u32,
            channels: u16,
        ) -> Result<AudioBuffer, AudioError> {
            let frames = samples_in(duration, sample_rate) as usize;
            let mut samples = Vec::with_capacity(frames * usize::from(channels));
            for _ in 0..frames {
                samples.extend([self.0, self.1].iter().take(usize::from(channels)));
            }
            Ok(AudioBuffer {
                sample_rate,
                channels,
                samples,
            })
        }
    }

    /// One audio clip of 90 frames, shaped by `edit`, mixed over its length.
    fn shaped(edit: impl FnOnce(&mut Clip), source: &mut dyn AudioSource) -> AudioBuffer {
        let format = TimelineFormat {
            width: 1920,
            height: 1080,
            rate: Rate::FPS_30,
            sample_rate: 48_000,
        };
        let mut project = Project::new(Id::new(IdKind::Project, 1), "Shape", format);
        let music = asset(&mut project, AssetKind::Audio, true);
        let mut voice = clip(
            &mut project,
            ClipContent::Audio {
                asset: music,
                source_in: Flicks::ZERO,
            },
            0,
            90,
            1.0,
        );
        edit(&mut voice);
        track(&mut project, TrackKind::Audio, false, vec![voice]);
        let timeline = project.manifest.primary_timeline.clone();
        mix(
            &project,
            &timeline,
            Flicks::ZERO,
            frames(90),
            RATE,
            2,
            source,
        )
        .unwrap()
    }

    fn stereo_at(buffer: &AudioBuffer, frame: i64) -> (f32, f32) {
        let index = samples_in(frames(frame), RATE) as usize;
        (buffer.samples[index * 2], buffer.samples[index * 2 + 1])
    }

    #[test]
    fn fades_and_volume_keyframes_shape_the_gain() {
        let faded = shaped(
            |c| {
                c.audio.fade_in = frames(30);
                c.audio.fade_out = frames(30);
            },
            &mut Stereo(0.5, 0.5),
        );
        assert_eq!(stereo_at(&faded, 0).0, 0.0, "silent at the start");
        let half = stereo_at(&faded, 15).0;
        assert!(
            (half - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "{half}"
        );
        assert_eq!(stereo_at(&faded, 45).0, 0.5, "full in the middle");
        assert!(stereo_at(&faded, 89).0 < 0.05, "nearly silent at the end");

        let ramp = shaped(
            |c| {
                c.keyframes.insert(
                    "volume".into(),
                    vec![
                        fosscut_model::Keyframe {
                            time: Flicks::ZERO,
                            value: fosscut_model::KeyValue::Number(0.0),
                            interpolation: fosscut_model::Interpolation::Linear,
                        },
                        fosscut_model::Keyframe {
                            time: frames(60),
                            value: fosscut_model::KeyValue::Number(2.0),
                            interpolation: fosscut_model::Interpolation::Linear,
                        },
                    ],
                );
                c.volume = 0.0;
            },
            &mut Stereo(0.25, 0.25),
        );
        let quarter = stereo_at(&ramp, 15).0;
        assert!(
            (quarter - 0.125).abs() < 1e-3,
            "gain 0.5 at frame 15: {quarter}"
        );
        assert!(
            (stereo_at(&ramp, 75).0 - 0.5).abs() < 1e-6,
            "held at 2 after"
        );
    }

    #[test]
    fn channels_pan_and_deactivation() {
        let mut source = Stereo(0.6, 0.2);
        let plain = shaped(|_| {}, &mut source);
        assert_eq!(stereo_at(&plain, 10), (0.6, 0.2));
        let left = shaped(|c| c.audio.channels = ChannelMode::Left, &mut source);
        assert_eq!(stereo_at(&left, 10), (0.6, 0.6));
        let right = shaped(|c| c.audio.channels = ChannelMode::Right, &mut source);
        assert_eq!(stereo_at(&right, 10), (0.2, 0.2));
        let mono = shaped(|c| c.audio.channels = ChannelMode::Mono, &mut source);
        let (l, r) = stereo_at(&mono, 10);
        assert!((l - 0.4).abs() < 1e-6 && (r - 0.4).abs() < 1e-6);
        let panned = shaped(|c| c.audio.pan = 1.0, &mut source);
        let (l, r) = stereo_at(&panned, 10);
        assert!(l.abs() < 1e-6, "hard right silences the left: {l}");
        assert_eq!(r, 0.2, "and leaves the right as it was");
        let off = shaped(|c| c.enabled = false, &mut source);
        assert!(
            off.samples.iter().all(|s| *s == 0.0),
            "deactivated clips are silent"
        );
        assert_eq!(balance(0.0), (1.0, 1.0));
        let (l, r) = balance(-0.5);
        assert!((r - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6 && l == 1.0);
    }

    #[test]
    fn transitions_crossfade_the_two_clips() {
        let format = TimelineFormat {
            width: 1920,
            height: 1080,
            rate: Rate::FPS_30,
            sample_rate: 48_000,
        };
        let mut project = Project::new(Id::new(IdKind::Project, 1), "Cross", format);
        let one = asset(&mut project, AssetKind::Audio, true);
        let two = asset(&mut project, AssetKind::Audio, true);
        let mut first = clip(
            &mut project,
            ClipContent::Audio {
                asset: one.clone(),
                source_in: frames(0),
            },
            0,
            60,
            1.0,
        );
        first.transition = Some(fosscut_model::Transition {
            transition_type: "dissolve".into(),
            duration: frames(30),
            params: Default::default(),
        });
        let second = clip(
            &mut project,
            ClipContent::Audio {
                asset: two.clone(),
                source_in: frames(5),
            },
            60,
            60,
            1.0,
        );
        track(&mut project, TrackKind::Audio, false, vec![first, second]);
        let timeline = project.manifest.primary_timeline.clone();
        let mut source = Levels {
            levels: HashMap::from([(one.clone(), 0.4), (two.clone(), 0.4)]),
            reads: Vec::new(),
        };
        let out = mix(
            &project,
            &timeline,
            Flicks::ZERO,
            frames(120),
            RATE,
            2,
            &mut source,
        )
        .unwrap();
        assert_eq!(
            level_at(&out, 30),
            0.4,
            "before the transition: the first clip alone"
        );
        let both = level_at(&out, 60);
        assert!(
            (both - 0.8 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "at the cut both play at -3 dB: {both}"
        );
        let late = level_at(&out, 72);
        assert!(
            late > 0.4 * 0.9 && late < 0.4 * 1.2,
            "near the end the second dominates: {late}"
        );
        assert_eq!(level_at(&out, 100), 0.4, "after it: the second clip alone");
        // The second clip starts reading 15 frames early, from frame 5 of its media: the
        // first 10 of those frames are before the media and stay silent.
        let early = source.reads.iter().find(|(a, ..)| a == &two).unwrap();
        assert_eq!(early.1, Flicks::ZERO, "read from the media's start");
        let silent = level_at(&out, 46);
        assert!((silent - 0.4 * crossfade_out(46)).abs() < 1e-3, "{silent}");
    }

    /// The first clip's gain in the transition test at `frame`.
    fn crossfade_out(frame: i64) -> f32 {
        let p = (frame - 45) as f64 / 30.0;
        (p * std::f64::consts::FRAC_PI_2).cos() as f32
    }

    /// A sine tone of `hz` in every channel, from the asset's start for `seconds`.
    struct Tone {
        hz: f64,
        seconds: f64,
    }

    impl AudioSource for Tone {
        fn read(
            &mut self,
            _asset: &Id,
            start: Flicks,
            duration: Flicks,
            sample_rate: u32,
            channels: u16,
        ) -> Result<AudioBuffer, AudioError> {
            let first = samples_in(start, sample_rate);
            let frames = samples_in(duration, sample_rate) as usize;
            let mut samples = Vec::with_capacity(frames * usize::from(channels));
            for i in 0..frames as i64 {
                let t = (first + i) as f64 / f64::from(sample_rate);
                let value = if (0.0..self.seconds).contains(&t) {
                    (std::f64::consts::TAU * self.hz * t).sin() as f32 * 0.5
                } else {
                    0.0
                };
                samples.extend(std::iter::repeat_n(value, usize::from(channels)));
            }
            Ok(AudioBuffer {
                sample_rate,
                channels,
                samples,
            })
        }
    }

    /// A project with one audio clip of `length` frames playing a tone asset at `speed`.
    fn tone_project(speed: fosscut_model::Speed, length: i64) -> (Project, Id) {
        let format = TimelineFormat {
            width: 1920,
            height: 1080,
            rate: Rate::FPS_30,
            sample_rate: RATE,
        };
        let mut project = Project::new(Id::new(IdKind::Project, 1), "Speed", format);
        let tone = asset(&mut project, AssetKind::Audio, true);
        let mut clip = clip(
            &mut project,
            ClipContent::Audio {
                asset: tone,
                source_in: Flicks::ZERO,
            },
            0,
            length,
            1.0,
        );
        clip.speed = Some(speed);
        track(&mut project, TrackKind::Audio, false, vec![clip]);
        let timeline = project.manifest.primary_timeline.clone();
        (project, timeline)
    }

    /// The frequency of the first channel of `buffer` between `from` and `to` seconds,
    /// from its zero crossings.
    fn frequency(buffer: &AudioBuffer, from: f64, to: f64) -> f64 {
        let channels = usize::from(buffer.channels);
        let at = |s: f64| (s * f64::from(buffer.sample_rate)) as usize;
        let mono: Vec<f32> = (at(from)..at(to))
            .map(|i| buffer.samples[i * channels])
            .collect();
        let crossings = mono
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        crossings as f64 / 2.0 / (to - from)
    }

    #[test]
    fn speed_keeps_or_shifts_the_pitch() {
        use fosscut_model::{Pitch, Speed};
        let mut tone = Tone {
            hz: 440.0,
            seconds: 10.0,
        };
        let mixed = |speed: Speed, source: &mut Tone| {
            let (project, timeline) = tone_project(speed, 60);
            mix(
                &project,
                &timeline,
                Flicks::ZERO,
                frames(60),
                RATE,
                2,
                source,
            )
            .unwrap()
        };
        let kept = mixed(Speed::constant(2.0), &mut tone);
        let f = frequency(&kept, 0.2, 1.8);
        assert!((f - 440.0).abs() < 10.0, "2x keeps the pitch: {f} Hz");
        let slow = mixed(Speed::constant(0.5), &mut tone);
        let f = frequency(&slow, 0.2, 1.8);
        assert!(
            (f - 440.0).abs() < 10.0,
            "half speed keeps the pitch: {f} Hz"
        );
        let shifted = mixed(
            Speed {
                pitch: Pitch::Shift,
                ..Speed::constant(2.0)
            },
            &mut tone,
        );
        let f = frequency(&shifted, 0.2, 1.8);
        assert!((f - 880.0).abs() < 10.0, "2x shifted doubles it: {f} Hz");
        // At rate 1 the windows line up with the source and add back to it.
        let same = mixed(Speed::constant(1.0), &mut tone);
        let plain = Tone {
            hz: 440.0,
            seconds: 10.0,
        }
        .read(
            &Id::new(IdKind::Asset, 1),
            Flicks::ZERO,
            frames(60),
            RATE,
            2,
        )
        .unwrap();
        let worst = same
            .samples
            .iter()
            .zip(&plain.samples)
            .skip(2 * 2000)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "rate 1 reproduces the source: {worst}");
        let frozen = mixed(Speed::constant(0.0), &mut tone);
        assert!(
            frozen.samples.iter().all(|s| *s == 0.0),
            "a freeze is silent"
        );
    }

    #[test]
    fn reversed_sound_plays_backwards_and_windows_do_not_matter() {
        use fosscut_model::{Pitch, Speed};
        // A rising tone: reversed, it falls.
        struct Sweep;
        impl AudioSource for Sweep {
            fn read(
                &mut self,
                _asset: &Id,
                start: Flicks,
                duration: Flicks,
                sample_rate: u32,
                channels: u16,
            ) -> Result<AudioBuffer, AudioError> {
                let first = samples_in(start, sample_rate);
                let frames = samples_in(duration, sample_rate) as usize;
                let samples = (0..frames as i64)
                    .flat_map(|i| {
                        let t = (first + i) as f64 / f64::from(sample_rate);
                        let v = if (0.0..2.0).contains(&t) {
                            (std::f64::consts::TAU * (200.0 * t + 100.0 * t * t)).sin() as f32
                        } else {
                            0.0
                        };
                        std::iter::repeat_n(v * 0.5, usize::from(channels))
                    })
                    .collect();
                Ok(AudioBuffer {
                    sample_rate,
                    channels,
                    samples,
                })
            }
        }
        for pitch in [Pitch::Preserve, Pitch::Shift] {
            let (project, timeline) = tone_project(
                Speed {
                    reverse: true,
                    pitch,
                    ..Speed::constant(1.0)
                },
                60,
            );
            let whole = mix(
                &project,
                &timeline,
                Flicks::ZERO,
                frames(60),
                RATE,
                2,
                &mut Sweep,
            )
            .unwrap();
            let early = frequency(&whole, 0.1, 0.5);
            let late = frequency(&whole, 1.5, 1.9);
            // The sweep runs from 200 Hz up to 600 Hz over two seconds.
            assert!(
                early > 500.0 && late < 300.0,
                "{pitch:?}: {early} then {late} Hz"
            );
            // Mixing in two windows gives the same samples as one.
            let a = mix(
                &project,
                &timeline,
                Flicks::ZERO,
                frames(25),
                RATE,
                2,
                &mut Sweep,
            )
            .unwrap();
            let b = mix(
                &project,
                &timeline,
                frames(25),
                frames(35),
                RATE,
                2,
                &mut Sweep,
            )
            .unwrap();
            let joined: Vec<f32> = a.samples.iter().chain(&b.samples).copied().collect();
            assert_eq!(joined, whole.samples, "{pitch:?}");
        }
    }

    /// A project with one audio clip of a centred 1 kHz tone carrying `effects`.
    fn effect_project(effects: &[(&str, &[(&str, ParamValue)])]) -> (Project, Id) {
        let format = TimelineFormat {
            width: 1920,
            height: 1080,
            rate: Rate::FPS_30,
            sample_rate: RATE,
        };
        let mut project = Project::new(Id::new(IdKind::Project, 1), "Sound", format);
        let tone = asset(&mut project, AssetKind::Audio, true);
        let mut clip = clip(
            &mut project,
            ClipContent::Audio {
                asset: tone,
                source_in: Flicks::ZERO,
            },
            0,
            120,
            1.0,
        );
        for (kind, params) in effects {
            let id = project.allocate_id(IdKind::Effect);
            clip.effects.push(fosscut_model::Effect {
                id,
                effect_type: (*kind).into(),
                version: 1,
                enabled: true,
                params: params
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.clone()))
                    .collect(),
                keyframes: Default::default(),
            });
        }
        track(&mut project, TrackKind::Audio, false, vec![clip]);
        let timeline = project.manifest.primary_timeline.clone();
        (project, timeline)
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |m, v| m.max(v.abs()))
    }

    #[test]
    fn sound_effects_shape_the_clip_and_blocks_do_not_matter() {
        let tone = || Tone {
            hz: 1000.0,
            seconds: 10.0,
        };
        let mix_all = |project: &Project, timeline: &Id, from: i64, to: i64| {
            mix(
                project,
                timeline,
                Flicks::from_millis(from),
                Flicks::from_millis(to - from),
                RATE,
                2,
                &mut tone(),
            )
            .unwrap()
            .samples
        };
        let (plain, timeline) = effect_project(&[]);
        let reference = mix_all(&plain, &timeline, 1000, 2000);
        assert!(peak(&reference) > 0.45);

        // A centred tone is the "voice": removing it leaves silence, isolating keeps it.
        let remove: &[(&str, ParamValue)] = &[("mode", ParamValue::Name("remove".into()))];
        let (removed, _) = effect_project(&[("audio.vocals", remove)]);
        assert!(peak(&mix_all(&removed, &timeline, 1000, 2000)) < 1e-4);
        let (isolated, _) = effect_project(&[("audio.vocals", &[])]);
        let kept = peak(&mix_all(&isolated, &timeline, 1000, 2000));
        assert!((kept / peak(&reference) - 1.0).abs() < 0.12, "{kept}");
        // Disabled effects do nothing.
        let (mut disabled, _) = effect_project(&[("audio.vocals", remove)]);
        let doc = disabled.timelines.get_mut(&timeline).unwrap();
        doc.tracks[0].clips[0].effects[0].enabled = false;
        assert_eq!(mix_all(&disabled, &timeline, 1000, 2000), reference);

        // Stateful effects sound the same whether the mix is cut into blocks or not.
        let (shaped, _) = effect_project(&[
            (
                "audio.enhance_voice",
                &[("amount", ParamValue::Number(1.0))],
            ),
            ("audio.denoise", &[("amount", ParamValue::Number(0.8))]),
        ]);
        let whole = mix_all(&shaped, &timeline, 0, 4000);
        let mut blocks = Vec::new();
        for (from, to) in [(0, 1300), (1300, 2700), (2700, 4000)] {
            blocks.extend(mix_all(&shaped, &timeline, from, to));
        }
        assert_eq!(blocks.len(), whole.len());
        let worst = whole
            .iter()
            .zip(&blocks)
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst < 1e-3, "blocks differ by {worst}");
        assert_ne!(
            whole[..48_000 * 2],
            plain_mix(&plain, &timeline)[..48_000 * 2]
        );
    }

    #[test]
    fn voice_filters_from_the_registry_play_the_same_in_any_blocks() {
        // 14 700 flicks per sample at 48 kHz.
        let at = |sample: i64| Flicks::new(sample * 14_700);
        let mix_range = |project: &Project, timeline: &Id, from: i64, to: i64| {
            mix(
                project,
                timeline,
                at(from),
                at(to) - at(from),
                RATE,
                2,
                &mut Tone {
                    hz: 220.0,
                    seconds: 10.0,
                },
            )
            .unwrap()
            .samples
        };
        // The whole span at once, and its last 0.3 s in blocks of 512, 1000 and 4800
        // samples, each fed its own warm-up the way playback asks for it.
        let (span, first) = (72_000, 57_600);
        let (plain, timeline) = effect_project(&[]);
        let reference = mix_range(&plain, &timeline, 0, span);
        let filters: [(&str, &[(&str, ParamValue)]); 10] = [
            ("voice.pitch", &[("semitones", ParamValue::Number(5.0))]),
            (
                "voice.pitch",
                &[
                    ("semitones", ParamValue::Number(-7.0)),
                    ("keep_formants", ParamValue::Flag(false)),
                ],
            ),
            ("voice.robot", &[]),
            ("voice.telephone", &[]),
            ("voice.megaphone", &[]),
            ("voice.echo", &[]),
            ("voice.reverb", &[("mix", ParamValue::Number(0.5))]),
            ("voice.chorus", &[]),
            ("voice.broken", &[]),
            ("voice.whisper", &[]),
        ];
        for (kind, params) in filters {
            let (project, _) = effect_project(&[(kind, params)]);
            let whole = mix_range(&project, &timeline, 0, span);
            let mut blocks = Vec::new();
            let mut from = first;
            for size in [512, 1000, 4800].iter().cycle() {
                if from >= span {
                    break;
                }
                let to = (from + size).min(span);
                blocks.extend(mix_range(&project, &timeline, from, to));
                from = to;
            }
            let tail = &whole[2 * first as usize..];
            let worst = tail
                .iter()
                .zip(&blocks)
                .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
            assert_eq!(blocks.len(), tail.len());
            assert!(worst <= 1e-6, "{kind} {params:?}: blocks differ by {worst}");
            assert_ne!(whole, reference, "{kind} changes the sound");
        }
    }

    fn plain_mix(project: &Project, timeline: &Id) -> Vec<f32> {
        mix(
            project,
            timeline,
            Flicks::ZERO,
            Flicks::from_seconds(4).unwrap(),
            RATE,
            2,
            &mut Tone {
                hz: 1000.0,
                seconds: 10.0,
            },
        )
        .unwrap()
        .samples
    }
}
