//! Remuxed copies (ADR-0032, item 3): media in containers that seek slowly or inexactly
//! (MPEG transport and program streams, AVI, FLV) copied without re-encoding into
//! QuickTime, which seeks exactly, for the desktop preview
//! ([`crate::preview::PreviewOptions::remuxed`]). The packets are the same, so the copy
//! decodes to the same frames; exports keep reading the original.
//!
//! A copy is kept in `remux/` of the cache directory, named by its source's content
//! identity and [`REMUX_VERSION`], so it is made once per content. It is written under a
//! temporary name and moved into place when complete.

use std::path::{Path, PathBuf};
use std::time::Instant;

use fosscut_media::OutputReport;
use fosscut_media::ffmpeg::{probe_output, probe_video, remux_with_progress};
use fosscut_model::{ContentIdentity, Id, Project};
use serde::{Deserialize, Serialize};

use crate::cache::{content_name, source_of, temporary_path, touch};
use crate::proxy::{failed_io, video_asset};
use crate::{EngineError, Progress};

/// The version of the copies' format, part of their file names.
pub const REMUX_VERSION: u32 = 1;

/// File extensions of the containers that are remuxed, lowercase.
pub const REMUX_EXTENSIONS: [&str; 8] = ["ts", "mts", "m2ts", "mpg", "mpeg", "vob", "avi", "flv"];

/// FFmpeg's names for those containers, as probes report them: MPEG-TS, MPEG-PS, AVI and
/// FLV.
const REMUX_DEMUXERS: [&str; 4] = ["mpegts", "mpeg", "avi", "flv"];

/// Whether media at `path`, in the probed `container` (FFmpeg's demuxer names, such as
/// `mpegts`, or a list of them separated by commas), seeks badly enough to remux: an MPEG
/// transport stream (`.ts`, `.mts`, `.m2ts`), an MPEG program stream (`.mpg`, `.mpeg`,
/// `.vob`), AVI or FLV, by its extension or its probed container.
pub fn needs_remux(path: &Path, container: Option<&str>) -> bool {
    let by_extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|e| REMUX_EXTENSIONS.contains(&e.as_str()));
    let by_probe = container.is_some_and(|names| {
        names
            .split(',')
            .any(|name| REMUX_DEMUXERS.contains(&name.trim()))
    });
    by_extension || by_probe
}

/// Where the remuxed copy of the file with `identity` is kept, below the cache directory
/// `cache`: `remux/<content>@<version>.mov`.
pub fn remux_path(cache: &Path, identity: &ContentIdentity) -> PathBuf {
    cache
        .join("remux")
        .join(format!("{}@{REMUX_VERSION}.mov", content_name(identity)))
}

/// The remuxed copy of `asset` if it has been made.
pub fn existing_remux(
    project: &Project,
    bundle: &Path,
    asset: &Id,
    cache: &Path,
) -> Result<Option<PathBuf>, EngineError> {
    let (_, identity) = source_of(project, bundle, asset)?;
    let path = remux_path(cache, &identity);
    Ok(path.is_file().then_some(path))
}

/// A remuxed copy that exists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemuxReport {
    /// The asset.
    pub asset: String,
    /// The copy.
    pub path: String,
    /// True when the copy existed already and nothing was made.
    pub reused: bool,
    /// What the copy contains, as probed.
    pub verified: OutputReport,
    /// Wall-clock time in milliseconds.
    pub elapsed_ms: u64,
}

/// Copies the streams of `asset`'s media into QuickTime without re-encoding them, or
/// reports the copy that exists. Media that does not need it ([`needs_remux`]) fails with
/// `fosscut.cache.remux_not_needed`. The copy must have the source's video codec and size,
/// or it is removed and the job fails with `fosscut.cache.verification_failed`. `progress`
/// counts milliseconds copied and may cancel by returning `false`, which keeps nothing
/// (`fosscut.job.cancelled`).
pub fn remux(
    project: &Project,
    bundle: &Path,
    asset: &Id,
    cache: &Path,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<RemuxReport, EngineError> {
    let started = Instant::now();
    video_asset(project, asset)?;
    let (source, identity) = source_of(project, bundle, asset)?;
    let container = project
        .media
        .get(asset)
        .and_then(|a| a.info.container.clone());
    if !needs_remux(&source, container.as_deref()) {
        return Err(EngineError::RemuxNotNeeded {
            asset: asset.to_string(),
            container: container.unwrap_or_else(|| "an unknown container".into()),
        });
    }
    let path = remux_path(cache, &identity);
    let report = |verified: OutputReport, reused: bool| RemuxReport {
        asset: asset.to_string(),
        path: path.display().to_string(),
        reused,
        verified,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    if path.is_file() {
        match probe_output(&path) {
            Ok(verified) if verified.video.is_some() => {
                touch(&path);
                return Ok(report(verified, true));
            }
            _ => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let original = probe_video(&source)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| failed_io(parent, &e))?;
    }
    let temporary = temporary_path(&path);
    let copied = remux_with_progress(&source, &temporary, &mut |done, total| {
        progress(Progress { done, total })
    });
    let verified = match copied {
        Ok(verified) => verified,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(match error {
                fosscut_media::MediaError::Cancelled => EngineError::JobCancelled {
                    kind: "remux".into(),
                },
                other => other.into(),
            });
        }
    };
    let mut mismatches = Vec::new();
    match &verified.video {
        None => mismatches.push("there is no video stream".to_string()),
        Some(video) => {
            if video.codec != original.codec {
                mismatches.push(format!(
                    "codec {}, expected the source's {}",
                    video.codec, original.codec
                ));
            }
            if (video.width, video.height) != (original.width, original.height) {
                mismatches.push(format!(
                    "{}x{}, expected the source's {}x{}",
                    video.width, video.height, original.width, original.height
                ));
            }
            if video.frame_count == 0 {
                mismatches.push("it has no frames".to_string());
            }
        }
    }
    if !mismatches.is_empty() {
        let _ = std::fs::remove_file(&temporary);
        return Err(EngineError::CopyMismatch {
            kind: "remux".into(),
            mismatches,
        });
    }
    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(failed_io(&path, &error));
    }
    Ok(report(verified, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_and_program_streams_avi_and_flv_are_remuxed() {
        for name in [
            "a.ts", "a.MTS", "a.m2ts", "a.mpg", "a.mpeg", "a.VOB", "a.avi", "a.flv",
        ] {
            assert!(needs_remux(Path::new(name), None), "{name}");
        }
        for name in ["a.mp4", "a.mov", "a.mkv", "a.webm", "ts", "a.ts.mp4"] {
            assert!(!needs_remux(Path::new(name), None), "{name}");
        }
        assert!(needs_remux(Path::new("capture.bin"), Some("mpegts")));
        assert!(needs_remux(Path::new("capture"), Some("avi")));
        assert!(!needs_remux(
            Path::new("clip.mp4"),
            Some("mov,mp4,m4a,3gp,3g2,mj2")
        ));
        assert!(!needs_remux(Path::new("clip.mkv"), Some("matroska,webm")));
    }

    #[test]
    fn copies_are_kept_by_content_and_version() {
        let identity = ContentIdentity {
            size: 5,
            sample_blake3: "ff00".into(),
        };
        assert_eq!(
            remux_path(Path::new("/cache"), &identity),
            Path::new(&format!("/cache/remux/ff00-5@{REMUX_VERSION}.mov"))
        );
    }
}
