use fosscut_media::ffmpeg::{audio_encoder, video_encoder};
use fosscut_media::{
    AudioCodec, AudioSettings, Container, OutputSettings, VideoCodec, VideoSettings,
};
use fosscut_model::TimelineFormat;
use serde::{Deserialize, Serialize};

/// A named combination of container and codecs. Size and frame rate come from the
/// timeline, so presets work for landscape, vertical and square projects alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Preset {
    /// H.264 video and AAC audio in MP4, the most compatible choice (hardware encoders).
    Mp4H264,
    /// HEVC video and AAC audio in MP4 (hardware encoders).
    Mp4Hevc,
    /// AV1 video and AAC audio in MP4.
    Mp4Av1,
    /// ProRes 422 and AAC in QuickTime, for handing off to other editors.
    MovProRes,
    /// ProRes 422 Proxy and 24-bit PCM in QuickTime, for offline editing.
    MovProResProxy,
    /// ProRes 422 LT and 24-bit PCM in QuickTime, a lighter intermediate.
    MovProResLt,
    /// ProRes 422 HQ and 24-bit PCM in QuickTime, for masters.
    MovProResHq,
    /// ProRes 4444 with alpha and 24-bit PCM in QuickTime, for graphics and compositing.
    MovProRes4444,
    /// VP9 and Opus in WebM.
    WebmVp9,
    /// An animated GIF that loops forever, with no sound.
    Gif,
    /// AAC audio only, in an MP4 (`.m4a`) file.
    M4aAac,
    /// MP3 audio only, when the FFmpeg build has LAME.
    Mp3,
    /// 16-bit PCM audio only, in WAV.
    Wav,
    /// 24-bit PCM audio only, in WAV.
    Wav24,
    /// FLAC audio only, lossless, 16-bit.
    Flac,
}

/// A preset as listed to people and agents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetInfo {
    /// Name to pass to `export`, for example `mp4-h264`.
    pub name: String,
    /// What it produces.
    pub description: String,
    /// The usual file extension.
    pub extension: String,
    /// Whether this machine can encode it.
    pub available: bool,
}

impl Preset {
    /// Every preset, most compatible first: video presets, then audio-only ones.
    pub const ALL: [Preset; 15] = [
        Preset::Mp4H264,
        Preset::Mp4Hevc,
        Preset::Mp4Av1,
        Preset::MovProRes,
        Preset::MovProResProxy,
        Preset::MovProResLt,
        Preset::MovProResHq,
        Preset::MovProRes4444,
        Preset::WebmVp9,
        Preset::Gif,
        Preset::M4aAac,
        Preset::Mp3,
        Preset::Wav,
        Preset::Wav24,
        Preset::Flac,
    ];

    /// The preset's name, for example `mp4-h264`.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Mp4H264 => "mp4-h264",
            Preset::Mp4Hevc => "mp4-hevc",
            Preset::Mp4Av1 => "mp4-av1",
            Preset::MovProRes => "mov-prores",
            Preset::MovProResProxy => "mov-prores-proxy",
            Preset::MovProResLt => "mov-prores-lt",
            Preset::MovProResHq => "mov-prores-hq",
            Preset::MovProRes4444 => "mov-prores-4444",
            Preset::WebmVp9 => "webm-vp9",
            Preset::Gif => "gif",
            Preset::M4aAac => "m4a-aac",
            Preset::Mp3 => "mp3",
            Preset::Wav => "wav",
            Preset::Wav24 => "wav-24",
            Preset::Flac => "flac",
        }
    }

    /// The preset with this name.
    pub fn parse(name: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.name() == name)
    }

    fn description(self) -> &'static str {
        match self {
            Preset::Mp4H264 => "H.264 and AAC in MP4: plays everywhere (hardware encoder)",
            Preset::Mp4Hevc => "HEVC and AAC in MP4: smaller files (hardware encoder)",
            Preset::Mp4Av1 => "AV1 and AAC in MP4: small files, modern players",
            Preset::MovProRes => "ProRes 422 and AAC in QuickTime: for other editors",
            Preset::MovProResProxy => {
                "ProRes 422 Proxy and 24-bit PCM in QuickTime: light files for offline editing"
            }
            Preset::MovProResLt => {
                "ProRes 422 LT and 24-bit PCM in QuickTime: a lighter intermediate"
            }
            Preset::MovProResHq => "ProRes 422 HQ and 24-bit PCM in QuickTime: for masters",
            Preset::MovProRes4444 => {
                "ProRes 4444 with alpha and 24-bit PCM in QuickTime: for graphics and compositing"
            }
            Preset::WebmVp9 => "VP9 and Opus in WebM: for the web",
            Preset::Gif => "Animated GIF, looping, no sound: pick a small size and frame rate",
            Preset::M4aAac => "AAC audio only",
            Preset::Mp3 => "MP3 audio only: plays everywhere (LAME encoder)",
            Preset::Wav => "16-bit PCM audio only in WAV: uncompressed",
            Preset::Wav24 => "24-bit PCM audio only in WAV: uncompressed, for mastering",
            Preset::Flac => "FLAC audio only: lossless 16-bit, about half the size of WAV",
        }
    }

    fn codecs(self) -> (Container, Option<VideoCodec>, Option<AudioCodec>) {
        let (container, video, audio) = match self {
            Preset::Mp4H264 => (Container::Mp4, Some(VideoCodec::H264), AudioCodec::Aac),
            Preset::Mp4Hevc => (Container::Mp4, Some(VideoCodec::Hevc), AudioCodec::Aac),
            Preset::Mp4Av1 => (Container::Mp4, Some(VideoCodec::Av1), AudioCodec::Aac),
            Preset::MovProRes => (Container::Mov, Some(VideoCodec::ProRes422), AudioCodec::Aac),
            Preset::MovProResProxy => (
                Container::Mov,
                Some(VideoCodec::ProResProxy),
                AudioCodec::Pcm24,
            ),
            Preset::MovProResLt => (
                Container::Mov,
                Some(VideoCodec::ProResLt),
                AudioCodec::Pcm24,
            ),
            Preset::MovProResHq => (
                Container::Mov,
                Some(VideoCodec::ProResHq),
                AudioCodec::Pcm24,
            ),
            Preset::MovProRes4444 => (
                Container::Mov,
                Some(VideoCodec::ProRes4444),
                AudioCodec::Pcm24,
            ),
            Preset::WebmVp9 => (Container::Webm, Some(VideoCodec::Vp9), AudioCodec::Opus),
            Preset::Gif => return (Container::Gif, Some(VideoCodec::Gif), None),
            Preset::M4aAac => (Container::Mp4, None, AudioCodec::Aac),
            Preset::Mp3 => (Container::Mp3, None, AudioCodec::Mp3),
            Preset::Wav => (Container::Wav, None, AudioCodec::Pcm16),
            Preset::Wav24 => (Container::Wav, None, AudioCodec::Pcm24),
            Preset::Flac => (Container::Flac, None, AudioCodec::Flac),
        };
        (container, video, Some(audio))
    }

    /// The usual file extension.
    pub fn extension(self) -> &'static str {
        match self {
            Preset::M4aAac => "m4a",
            other => other.codecs().0.extension(),
        }
    }

    /// Output settings for a timeline of `format`.
    pub fn settings(self, format: &TimelineFormat) -> OutputSettings {
        let (container, video, audio) = self.codecs();
        OutputSettings {
            container,
            video: video.map(|codec| VideoSettings {
                codec,
                width: format.width,
                height: format.height,
                rate: format.rate,
                bit_rate: None,
            }),
            audio: audio.map(AudioSettings::stereo),
        }
    }

    /// Whether this machine can encode the preset: an encoder for each of its codecs.
    pub fn available(self) -> bool {
        let (_, video, audio) = self.codecs();
        video.is_none_or(|v| video_encoder(v).is_some())
            && audio.is_none_or(|a| audio_encoder(a).is_some())
    }

    /// The preset an output file implies: `.mov` is ProRes, `.webm` is VP9, `.gif` is GIF,
    /// `.m4a`, `.mp3`, `.wav` (16-bit) and `.flac` are audio only, and anything else is the
    /// most compatible video preset this machine can encode (`None` when it can encode no
    /// video). An implied preset this machine cannot encode, such as MP3 without LAME, is
    /// still returned, so the export reports the codec unavailable instead of writing
    /// something else.
    pub fn for_output(path: &std::path::Path) -> Option<Preset> {
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "mov" => Some(Preset::MovProRes),
            "webm" => Some(Preset::WebmVp9),
            "gif" => Some(Preset::Gif),
            "m4a" => Some(Preset::M4aAac),
            "mp3" => Some(Preset::Mp3),
            "wav" => Some(Preset::Wav),
            "flac" => Some(Preset::Flac),
            _ => Preset::default_video(),
        }
    }

    /// The most compatible video preset this machine can encode. GIF, with no sound and a
    /// palette per frame, is never the default.
    pub fn default_video() -> Option<Preset> {
        Preset::ALL
            .into_iter()
            .filter(|p| p.codecs().1.is_some_and(|v| v != VideoCodec::Gif))
            .find(|p| p.available())
    }
}

/// Every preset with its availability on this machine.
pub fn presets() -> Vec<PresetInfo> {
    Preset::ALL
        .into_iter()
        .map(|preset| PresetInfo {
            name: preset.name().to_string(),
            description: preset.description().to_string(),
            extension: preset.extension().to_string(),
            available: preset.available(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn format() -> TimelineFormat {
        TimelineFormat {
            width: 320,
            height: 180,
            rate: fosscut_time::Rate::FPS_30,
            sample_rate: 48_000,
        }
    }

    #[test]
    fn extensions_choose_presets() {
        assert_eq!(
            Preset::for_output(Path::new("a.mov")),
            Some(Preset::MovProRes)
        );
        assert_eq!(
            Preset::for_output(Path::new("a.WEBM")),
            Some(Preset::WebmVp9)
        );
        assert_eq!(Preset::for_output(Path::new("a.m4a")), Some(Preset::M4aAac));
        assert_eq!(Preset::for_output(Path::new("a.gif")), Some(Preset::Gif));
        assert_eq!(Preset::for_output(Path::new("a.GIF")), Some(Preset::Gif));
        assert_eq!(Preset::for_output(Path::new("a.wav")), Some(Preset::Wav));
        assert_eq!(Preset::for_output(Path::new("a.flac")), Some(Preset::Flac));
        assert_eq!(Preset::for_output(Path::new("a.mp3")), Some(Preset::Mp3));
        assert_eq!(
            Preset::for_output(Path::new("a.mp4")),
            Preset::default_video()
        );
        assert_eq!(Preset::for_output(Path::new("a")), Preset::default_video());
        assert_ne!(Preset::default_video(), Some(Preset::Gif));
    }

    #[test]
    fn names_parse_and_extensions_match_containers() {
        let mut names: Vec<&str> = Preset::ALL.iter().map(|p| p.name()).collect();
        for preset in Preset::ALL {
            assert_eq!(Preset::parse(preset.name()), Some(preset));
            assert!(!preset.description().is_empty());
        }
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Preset::ALL.len(), "names are unique");
        for (name, extension) in [
            ("mov-prores-proxy", "mov"),
            ("mov-prores-lt", "mov"),
            ("mov-prores-hq", "mov"),
            ("mov-prores-4444", "mov"),
            ("gif", "gif"),
            ("wav", "wav"),
            ("wav-24", "wav"),
            ("flac", "flac"),
            ("mp3", "mp3"),
        ] {
            let preset = Preset::parse(name).unwrap();
            assert_eq!(preset.extension(), extension, "{name}");
        }
        assert_eq!(Preset::parse("avi"), None);
    }

    #[test]
    fn settings_use_the_presets_codecs_in_containers_that_hold_them() {
        for preset in Preset::ALL {
            let settings = preset.settings(&format());
            let video = settings.video.as_ref().map(|v| v.codec);
            let audio = settings.audio.as_ref().map(|a| a.codec);
            assert!(
                settings.container.supports(video, audio),
                "{}: {settings:?}",
                preset.name()
            );
            assert!(video.is_some() || audio.is_some(), "{}", preset.name());
            if let Some(v) = &settings.video {
                assert_eq!((v.width, v.height, v.rate), (320, 180, format().rate));
            }
        }
        let codecs = |preset: Preset| {
            let settings = preset.settings(&format());
            (
                settings.container,
                settings.video.map(|v| v.codec),
                settings.audio.map(|a| a.codec),
            )
        };
        assert_eq!(
            codecs(Preset::MovProResHq),
            (
                Container::Mov,
                Some(VideoCodec::ProResHq),
                Some(AudioCodec::Pcm24)
            )
        );
        assert_eq!(
            codecs(Preset::MovProRes4444).1,
            Some(VideoCodec::ProRes4444)
        );
        assert_eq!(
            codecs(Preset::MovProResProxy).1,
            Some(VideoCodec::ProResProxy)
        );
        assert_eq!(codecs(Preset::MovProResLt).1, Some(VideoCodec::ProResLt));
        assert_eq!(
            codecs(Preset::Gif),
            (Container::Gif, Some(VideoCodec::Gif), None)
        );
        assert_eq!(
            codecs(Preset::Wav),
            (Container::Wav, None, Some(AudioCodec::Pcm16))
        );
        assert_eq!(
            codecs(Preset::Wav24),
            (Container::Wav, None, Some(AudioCodec::Pcm24))
        );
        assert_eq!(
            codecs(Preset::Flac),
            (Container::Flac, None, Some(AudioCodec::Flac))
        );
        assert_eq!(
            codecs(Preset::Mp3),
            (Container::Mp3, None, Some(AudioCodec::Mp3))
        );
    }

    #[test]
    fn availability_follows_the_encoders() {
        for preset in Preset::ALL {
            let settings = preset.settings(&format());
            let encoders = settings
                .video
                .as_ref()
                .is_none_or(|v| video_encoder(v.codec).is_some())
                && settings
                    .audio
                    .as_ref()
                    .is_none_or(|a| audio_encoder(a.codec).is_some());
            assert_eq!(preset.available(), encoders, "{}", preset.name());
        }
        // FFmpeg's own encoders are in every build FossCut supports.
        for preset in [Preset::Wav, Preset::Wav24, Preset::Flac, Preset::Gif] {
            assert!(preset.available(), "{}", preset.name());
        }
        assert_eq!(
            Preset::Mp3.available(),
            audio_encoder(AudioCodec::Mp3).is_some()
        );
        let listed = presets();
        assert_eq!(listed.len(), Preset::ALL.len());
        let gif = listed.iter().find(|p| p.name == "gif").unwrap();
        assert_eq!((gif.extension.as_str(), gif.available), ("gif", true));
    }
}
