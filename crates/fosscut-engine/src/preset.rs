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
    /// VP9 and Opus in WebM.
    WebmVp9,
    /// AAC audio only, in an MP4 (`.m4a`) file.
    M4aAac,
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
    /// Every preset, most compatible first.
    pub const ALL: [Preset; 6] = [
        Preset::Mp4H264,
        Preset::Mp4Hevc,
        Preset::Mp4Av1,
        Preset::MovProRes,
        Preset::WebmVp9,
        Preset::M4aAac,
    ];

    /// The preset's name, for example `mp4-h264`.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Mp4H264 => "mp4-h264",
            Preset::Mp4Hevc => "mp4-hevc",
            Preset::Mp4Av1 => "mp4-av1",
            Preset::MovProRes => "mov-prores",
            Preset::WebmVp9 => "webm-vp9",
            Preset::M4aAac => "m4a-aac",
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
            Preset::WebmVp9 => "VP9 and Opus in WebM: for the web",
            Preset::M4aAac => "AAC audio only",
        }
    }

    fn codecs(self) -> (Container, Option<VideoCodec>, AudioCodec) {
        match self {
            Preset::Mp4H264 => (Container::Mp4, Some(VideoCodec::H264), AudioCodec::Aac),
            Preset::Mp4Hevc => (Container::Mp4, Some(VideoCodec::Hevc), AudioCodec::Aac),
            Preset::Mp4Av1 => (Container::Mp4, Some(VideoCodec::Av1), AudioCodec::Aac),
            Preset::MovProRes => (Container::Mov, Some(VideoCodec::ProRes422), AudioCodec::Aac),
            Preset::WebmVp9 => (Container::Webm, Some(VideoCodec::Vp9), AudioCodec::Opus),
            Preset::M4aAac => (Container::Mp4, None, AudioCodec::Aac),
        }
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
            audio: Some(AudioSettings::stereo(audio)),
        }
    }

    /// Whether this machine can encode the preset.
    pub fn available(self) -> bool {
        let (_, video, audio) = self.codecs();
        video.is_none_or(|v| video_encoder(v).is_some()) && audio_encoder(audio).is_some()
    }

    /// The preset an output file implies: `.mov` is ProRes, `.webm` is VP9, `.m4a` is audio
    /// only, and anything else is the most compatible video preset this machine can encode
    /// (`None` when it can encode no video).
    pub fn for_output(path: &std::path::Path) -> Option<Preset> {
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "mov" => Some(Preset::MovProRes),
            "webm" => Some(Preset::WebmVp9),
            "m4a" => Some(Preset::M4aAac),
            _ => Preset::default_video(),
        }
    }

    /// The most compatible video preset this machine can encode.
    pub fn default_video() -> Option<Preset> {
        Preset::ALL
            .into_iter()
            .filter(|p| p.codecs().1.is_some())
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
        assert_eq!(
            Preset::for_output(Path::new("a.mp4")),
            Preset::default_video()
        );
        assert_eq!(Preset::for_output(Path::new("a")), Preset::default_video());
    }
}
