use fosscut_media::OutputReport;
use serde_json::{Value, json};
use thiserror::Error;

/// Why an export did not produce a verified file.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ExportError {
    /// The timeline does not exist.
    #[error("timeline {timeline} does not exist")]
    TimelineNotFound {
        /// The requested timeline.
        timeline: String,
    },
    /// The timeline has nothing to export.
    #[error("the timeline is empty")]
    EmptyTimeline,
    /// The settings cannot work: odd size, impossible container and codec, no streams.
    #[error("invalid export settings: {message}")]
    InvalidSettings {
        /// What is wrong.
        message: String,
    },
    /// This machine has no encoder for the requested codec. Never substituted (ADR-0009).
    #[error("this machine cannot encode {codec}")]
    CodecUnavailable {
        /// The requested codec.
        codec: String,
        /// Codecs that work here instead.
        available: Vec<String>,
    },
    /// The project changed since the revision the request expected (ADR-0006, item 1).
    #[error("the project is at revision {current}, not {expected}")]
    RevisionConflict {
        /// The revision the request expected.
        expected: u64,
        /// The project's revision.
        current: u64,
    },
    /// The output exists and overwriting it was not allowed (ADR-0006, item 6).
    #[error("{path} already exists")]
    OutputExists {
        /// The output path.
        path: String,
    },
    /// A clip's media file cannot be found.
    #[error("media for asset {asset} is missing")]
    MissingMedia {
        /// The asset.
        asset: String,
        /// Paths that were tried.
        tried: Vec<String>,
    },
    /// The job was cancelled; no output was written.
    #[error("the export was cancelled")]
    Cancelled,
    /// The request id was used before for a different export.
    #[error("request id {request_id} was already used for another export")]
    RequestReused {
        /// The request id.
        request_id: String,
    },
    /// The written file does not match the request (ADR-0009, item 6). The file is kept.
    #[error("the exported file does not match the request: {}", mismatches.join("; "))]
    VerificationFailed {
        /// Each difference between request and file.
        mismatches: Vec<String>,
        /// What the file contains.
        report: Box<OutputReport>,
    },
    /// Rendering, decoding, mixing or encoding failed.
    #[error("{message}")]
    Failed {
        /// The underlying error code, for example `fosscut.media.encode_failed`.
        code: String,
        /// What went wrong.
        message: String,
    },
}

impl ExportError {
    /// The stable error code used in FossCut API responses (ADR-0006).
    pub fn code(&self) -> &str {
        match self {
            ExportError::TimelineNotFound { .. } => "fosscut.timeline.not_found",
            ExportError::EmptyTimeline => "fosscut.export.empty_timeline",
            ExportError::InvalidSettings { .. } => "fosscut.export.invalid_settings",
            ExportError::CodecUnavailable { .. } => "fosscut.export.codec_unavailable",
            ExportError::RevisionConflict { .. } => "fosscut.revision.conflict",
            ExportError::OutputExists { .. } => "fosscut.guard.destructive",
            ExportError::MissingMedia { .. } => "fosscut.export.missing_media",
            ExportError::Cancelled => "fosscut.export.cancelled",
            ExportError::RequestReused { .. } => "fosscut.request.reused",
            ExportError::VerificationFailed { .. } => "fosscut.export.verification_failed",
            ExportError::Failed { code, .. } => code,
        }
    }

    /// What an agent or person can do about it.
    pub fn hint(&self) -> Option<String> {
        match self {
            ExportError::CodecUnavailable { available, .. } => Some(format!(
                "Choose one of the codecs this machine can encode: {}.",
                available.join(", ")
            )),
            ExportError::OutputExists { .. } => {
                Some(
                    "Pass allow_destructive (--allow-destructive on the command line) to overwrite it, or choose another path."
                        .into(),
                )
            }
            ExportError::MissingMedia { .. } => {
                Some("Relink the media or restore the file at one of the tried paths.".into())
            }
            ExportError::EmptyTimeline => Some("Add clips before exporting.".into()),
            ExportError::RevisionConflict { .. } => Some(
                "Read the timeline again, then export with the current revision.".into(),
            ),
            _ => None,
        }
    }

    /// Machine-readable details.
    pub fn details(&self) -> Value {
        match self {
            ExportError::CodecUnavailable { codec, available } => {
                json!({ "codec": codec, "available": available })
            }
            ExportError::MissingMedia { asset, tried } => json!({ "asset": asset, "tried": tried }),
            ExportError::VerificationFailed { mismatches, report } => {
                json!({ "mismatches": mismatches, "report": report })
            }
            ExportError::OutputExists { path } => json!({ "path": path }),
            ExportError::RevisionConflict { expected, current } => {
                json!({ "expected_revision": expected, "current_revision": current })
            }
            _ => Value::Null,
        }
    }
}

impl From<fosscut_media::MediaError> for ExportError {
    fn from(error: fosscut_media::MediaError) -> ExportError {
        match error {
            fosscut_media::MediaError::EncoderUnavailable { codec, available } => {
                ExportError::CodecUnavailable { codec, available }
            }
            fosscut_media::MediaError::Unsupported { message } => {
                ExportError::InvalidSettings { message }
            }
            other => ExportError::Failed {
                code: other.code().to_string(),
                message: other.to_string(),
            },
        }
    }
}

impl From<fosscut_render::RenderError> for ExportError {
    fn from(error: fosscut_render::RenderError) -> ExportError {
        ExportError::Failed {
            code: error.code().to_string(),
            message: error.to_string(),
        }
    }
}

impl From<fosscut_audio::AudioError> for ExportError {
    fn from(error: fosscut_audio::AudioError) -> ExportError {
        ExportError::Failed {
            code: error.code().to_string(),
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> OutputReport {
        OutputReport {
            container: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            brand: Some("isom".into()),
            size_bytes: 1,
            duration: None,
            video: None,
            audio: None,
        }
    }

    #[test]
    fn every_error_has_a_stable_code_message_hint_and_details() {
        let cases = [
            (
                ExportError::TimelineNotFound {
                    timeline: "timeline_0009".into(),
                },
                "fosscut.timeline.not_found",
                "timeline timeline_0009 does not exist",
                false,
            ),
            (
                ExportError::EmptyTimeline,
                "fosscut.export.empty_timeline",
                "the timeline is empty",
                true,
            ),
            (
                ExportError::InvalidSettings {
                    message: "odd width".into(),
                },
                "fosscut.export.invalid_settings",
                "invalid export settings: odd width",
                false,
            ),
            (
                ExportError::CodecUnavailable {
                    codec: "h264".into(),
                    available: vec!["av1".into(), "prores".into()],
                },
                "fosscut.export.codec_unavailable",
                "this machine cannot encode h264",
                true,
            ),
            (
                ExportError::RevisionConflict {
                    expected: 3,
                    current: 5,
                },
                "fosscut.revision.conflict",
                "the project is at revision 5, not 3",
                true,
            ),
            (
                ExportError::OutputExists {
                    path: "Out.mp4".into(),
                },
                "fosscut.guard.destructive",
                "Out.mp4 already exists",
                true,
            ),
            (
                ExportError::MissingMedia {
                    asset: "asset_0001".into(),
                    tried: vec!["clip.mp4".into()],
                },
                "fosscut.export.missing_media",
                "media for asset asset_0001 is missing",
                true,
            ),
            (
                ExportError::Cancelled,
                "fosscut.export.cancelled",
                "the export was cancelled",
                false,
            ),
            (
                ExportError::RequestReused {
                    request_id: "r1".into(),
                },
                "fosscut.request.reused",
                "request id r1 was already used for another export",
                false,
            ),
            (
                ExportError::VerificationFailed {
                    mismatches: vec!["frames 239, expected 240".into(), "no audio".into()],
                    report: Box::new(report()),
                },
                "fosscut.export.verification_failed",
                "the exported file does not match the request: frames 239, expected 240; no audio",
                false,
            ),
            (
                ExportError::Failed {
                    code: "fosscut.media.encode_failed".into(),
                    message: "encoder stopped".into(),
                },
                "fosscut.media.encode_failed",
                "encoder stopped",
                false,
            ),
        ];
        for (error, code, message, has_hint) in cases {
            assert_eq!(error.code(), code);
            assert_eq!(error.to_string(), message);
            assert_eq!(error.hint().is_some(), has_hint, "{code}");
        }

        let unavailable = ExportError::CodecUnavailable {
            codec: "h264".into(),
            available: vec!["av1".into(), "prores".into()],
        };
        assert!(unavailable.hint().unwrap().ends_with("av1, prores."));
        assert_eq!(
            unavailable.details(),
            json!({ "codec": "h264", "available": ["av1", "prores"] })
        );
        let missing = ExportError::MissingMedia {
            asset: "asset_0001".into(),
            tried: vec!["clip.mp4".into()],
        };
        assert_eq!(
            missing.details(),
            json!({ "asset": "asset_0001", "tried": ["clip.mp4"] })
        );
        let exists = ExportError::OutputExists {
            path: "Out.mp4".into(),
        };
        assert_eq!(exists.details(), json!({ "path": "Out.mp4" }));
        let conflict = ExportError::RevisionConflict {
            expected: 3,
            current: 5,
        };
        assert_eq!(
            conflict.details(),
            json!({ "expected_revision": 3, "current_revision": 5 })
        );
        let mismatch = ExportError::VerificationFailed {
            mismatches: vec!["no audio".into()],
            report: Box::new(report()),
        };
        assert_eq!(mismatch.details()["mismatches"], json!(["no audio"]));
        assert_eq!(mismatch.details()["report"]["brand"], json!("isom"));
        assert_eq!(ExportError::Cancelled.details(), Value::Null);
    }

    #[test]
    fn lower_layer_errors_keep_their_codes() {
        let unavailable = ExportError::from(fosscut_media::MediaError::EncoderUnavailable {
            codec: "hevc".into(),
            available: vec!["av1".into()],
        });
        assert_eq!(
            unavailable,
            ExportError::CodecUnavailable {
                codec: "hevc".into(),
                available: vec!["av1".into()],
            }
        );
        let unsupported = ExportError::from(fosscut_media::MediaError::Unsupported {
            message: "vp9 in mp4".into(),
        });
        assert_eq!(unsupported.code(), "fosscut.export.invalid_settings");
        let open = fosscut_media::MediaError::OpenFailed {
            path: "clip.mp4".into(),
            message: "no such file".into(),
        };
        let expected = open.code().to_string();
        assert_eq!(ExportError::from(open).code(), expected);

        let render = fosscut_render::RenderError::NoAdapter {
            message: "no GPU".into(),
        };
        let expected = render.code().to_string();
        let converted = ExportError::from(render);
        assert_eq!(converted.code(), expected);
        assert!(converted.to_string().contains("no GPU"));

        let audio = fosscut_audio::AudioError::SourceFailed {
            asset: "asset_0002".into(),
            message: "corrupt".into(),
        };
        let converted = ExportError::from(audio);
        assert_eq!(converted.code(), "fosscut.audio.source_failed");
        assert!(converted.to_string().contains("asset_0002"));
    }
}
