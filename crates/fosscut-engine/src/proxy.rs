//! Proxies (ADR-0032, item 2): ProRes Proxy copies of video assets at half or a quarter of
//! their size, without sound, that only the desktop preview decodes
//! ([`crate::preview::PreviewOptions::proxies`]). Sound, exports, frame grabs and contact
//! sheets always read the original.
//!
//! A proxy is kept in `proxies/` of the cache directory, named by its source's content
//! identity, its size and [`PROXY_VERSION`], so it is made once per content and size
//! whatever project uses the file. It is written under a temporary name and moved into
//! place when complete and verified, so a proxy that exists is always whole.

use std::path::{Path, PathBuf};
use std::time::Instant;

use fosscut_media::ffmpeg::{FfmpegVideoDecoder, FfmpegWriter, WriterOptions, probe_output};
use fosscut_media::{
    Container, HardwareEncoding, OutputReport, OutputSettings, Range, VideoCodec, VideoDecoder,
    VideoSettings,
};
use fosscut_model::{Asset, AssetKind, ContentIdentity, Id, Project};
use fosscut_time::Rate;
use serde::{Deserialize, Serialize};

use crate::cache::{content_name, source_of, temporary_path, touch};
use crate::{EngineError, Progress};

/// The version of the proxies' format, part of their file names: bump it when proxies are
/// made differently, so older ones are made again (and age out of the cache).
pub const PROXY_VERSION: u32 = 1;

/// With [`ProxyPolicy::Auto`], assets with a side longer than this many pixels get proxies.
pub const AUTO_PROXY_SIDE: u32 = 1920;

/// How much smaller than its source a proxy is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxySize {
    /// Half the width and height.
    Half,
    /// A quarter of the width and height.
    Quarter,
}

impl ProxySize {
    /// Every size.
    pub const ALL: [ProxySize; 2] = [ProxySize::Half, ProxySize::Quarter];

    /// `half` or `quarter`.
    pub fn name(self) -> &'static str {
        match self {
            ProxySize::Half => "half",
            ProxySize::Quarter => "quarter",
        }
    }

    /// The size named `name`.
    pub fn parse(name: &str) -> Result<ProxySize, EngineError> {
        ProxySize::ALL
            .into_iter()
            .find(|size| size.name() == name)
            .ok_or_else(|| EngineError::InvalidArguments {
                message: format!("a proxy size is half or quarter, not {name:?}"),
            })
    }

    /// Each side is divided by this.
    pub fn divisor(self) -> u32 {
        match self {
            ProxySize::Half => 2,
            ProxySize::Quarter => 4,
        }
    }

    /// The proxy's size for a `source`-sized picture: each side divided by
    /// [`ProxySize::divisor`] and rounded to the nearest even number of pixels (at least 2),
    /// as encoders need. The aspect can differ by up to a pixel; the preview corrects the
    /// placement for it, so a proxy shows exactly the original's framing.
    pub fn dimensions(self, source: (u32, u32)) -> (u32, u32) {
        let even = |side: u32| {
            let reduced = f64::from(side) / f64::from(self.divisor());
            ((reduced / 2.0).round() as u32 * 2).max(2)
        };
        (even(source.0), even(source.1))
    }
}

/// When the desktop app makes proxies (`performance.proxies`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyPolicy {
    /// Never.
    #[default]
    Off,
    /// For video assets with a side longer than [`AUTO_PROXY_SIDE`] pixels.
    Auto,
    /// For every video asset.
    On,
}

impl ProxyPolicy {
    /// `off`, `auto` or `on`.
    pub fn name(self) -> &'static str {
        match self {
            ProxyPolicy::Off => "off",
            ProxyPolicy::Auto => "auto",
            ProxyPolicy::On => "on",
        }
    }

    /// The policy named `name`.
    pub fn parse(name: &str) -> Result<ProxyPolicy, EngineError> {
        [ProxyPolicy::Off, ProxyPolicy::Auto, ProxyPolicy::On]
            .into_iter()
            .find(|policy| policy.name() == name)
            .ok_or_else(|| EngineError::InvalidArguments {
                message: format!("proxies are off, auto or on, not {name:?}"),
            })
    }
}

/// Whether `policy` wants a proxy of `asset`: never for anything but video, and with
/// [`ProxyPolicy::Auto`] only when a side is longer than [`AUTO_PROXY_SIDE`] pixels.
pub fn wants_proxy(asset: &Asset, policy: ProxyPolicy) -> bool {
    let Some(video) = asset
        .info
        .video
        .as_ref()
        .filter(|_| asset.kind == AssetKind::Video)
    else {
        return false;
    };
    match policy {
        ProxyPolicy::Off => false,
        ProxyPolicy::Auto => video.width.max(video.height) > AUTO_PROXY_SIDE,
        ProxyPolicy::On => true,
    }
}

/// Where the `size` proxy of the file with `identity` is kept, below the cache directory
/// `cache`: `proxies/<content>/<size>@<version>.mov`.
pub fn proxy_path(cache: &Path, identity: &ContentIdentity, size: ProxySize) -> PathBuf {
    cache
        .join("proxies")
        .join(content_name(identity))
        .join(format!("{}@{PROXY_VERSION}.mov", size.name()))
}

/// The `size` proxy of `asset` if it has been made, for front ends that show which assets
/// have one; nothing is read but the media table and, without a recorded identity, the
/// asset's file.
pub fn existing_proxy(
    project: &Project,
    bundle: &Path,
    asset: &Id,
    size: ProxySize,
    cache: &Path,
) -> Result<Option<PathBuf>, EngineError> {
    let (_, identity) = source_of(project, bundle, asset)?;
    let path = proxy_path(cache, &identity, size);
    Ok(path.is_file().then_some(path))
}

/// A proxy that exists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProxyReport {
    /// The asset.
    pub asset: String,
    /// Its size.
    pub size: ProxySize,
    /// The proxy file.
    pub path: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Frames, as many as the source has.
    pub frames: u64,
    /// The frame rate, the source's.
    pub rate: Option<Rate>,
    /// True when the proxy existed already and nothing was made.
    pub reused: bool,
    /// What the proxy file contains, as probed.
    pub verified: OutputReport,
    /// Wall-clock time in milliseconds.
    pub elapsed_ms: u64,
}

/// Makes the `size` proxy of `asset`, or reports the one that exists: every frame of the
/// asset's video decoded, scaled with area averaging, and written as ProRes Proxy in
/// QuickTime without sound, tagged with the source's colors. The finished file is probed
/// and must have the expected codec, size, frame count and rate, or it is removed and the
/// job fails with `fosscut.cache.verification_failed`. `progress` counts frames and may
/// cancel by returning `false`, which keeps nothing (`fosscut.job.cancelled`).
pub fn make_proxy(
    project: &Project,
    bundle: &Path,
    asset: &Id,
    size: ProxySize,
    cache: &Path,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<ProxyReport, EngineError> {
    let started = Instant::now();
    let info = video_asset(project, asset)?;
    let (source, identity) = source_of(project, bundle, asset)?;
    let path = proxy_path(cache, &identity, size);
    let report = |verified: OutputReport, reused: bool| {
        let video = verified.video.clone();
        ProxyReport {
            asset: asset.to_string(),
            size,
            path: path.display().to_string(),
            width: video.as_ref().map_or(0, |v| v.width),
            height: video.as_ref().map_or(0, |v| v.height),
            frames: video.as_ref().map_or(0, |v| v.frame_count),
            rate: video.and_then(|v| v.rate),
            reused,
            verified,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    };
    if path.is_file() {
        match probe_output(&path) {
            Ok(verified) if verified.video.as_ref().is_some_and(|v| v.codec == "prores") => {
                touch(&path);
                return Ok(report(verified, true));
            }
            // A damaged proxy is made again.
            _ => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    let mut decoder = FfmpegVideoDecoder::open(&source, crate::hardware::decoder_options())?;
    let stream = decoder.info().clone();
    let rate = stream
        .rate
        .or(info.rate)
        .ok_or_else(|| EngineError::InvalidArguments {
            message: format!("asset {asset} has no constant frame rate to make a proxy at"),
        })?;
    let (width, height) = size.dimensions((stream.width, stream.height));
    let total = stream.frame_count.unwrap_or_else(|| {
        stream
            .duration
            .map_or(0, |d| rate.frame_at(d).max(0) as u64)
    });
    let Some(first) = decoder.next_frame()? else {
        return Err(EngineError::NotVideo {
            asset: asset.to_string(),
        });
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| failed_io(parent, &e))?;
    }
    let temporary = temporary_path(&path);
    let settings = OutputSettings {
        container: Container::Mov,
        video: Some(VideoSettings {
            codec: VideoCodec::ProResProxy,
            width,
            height,
            rate,
            bit_rate: None,
        }),
        audio: None,
    };
    // The source's colors, in limited range, which ProRes always uses.
    let mut color = first.color;
    color.range = Range::Limited;
    let written = (|| -> Result<u64, EngineError> {
        let mut writer = FfmpegWriter::create_with(
            &temporary,
            &settings,
            &WriterOptions {
                // ProRes is only encoded in software anyway.
                hardware: HardwareEncoding::Off,
                color: Some(color),
            },
        )?;
        let mut count = 0u64;
        let mut frame = Some(first);
        while let Some(current) = frame {
            writer.push_frame_scaled(&current)?;
            count += 1;
            if !progress(Progress {
                done: count,
                total: total.max(count),
            }) {
                return Err(EngineError::JobCancelled {
                    kind: "proxy".into(),
                });
            }
            frame = decoder.next_frame()?;
        }
        writer.finish()?;
        Ok(count)
    })();
    let frames = match written {
        Ok(frames) => frames,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(failed_io(&path, &error));
    }
    let verified = probe_output(&path)?;
    let mut mismatches = Vec::new();
    match &verified.video {
        None => mismatches.push("there is no video stream".to_string()),
        Some(video) => {
            if video.codec != "prores" || video.profile.as_deref() != Some("Proxy") {
                mismatches.push(format!(
                    "codec {} {}, expected prores Proxy",
                    video.codec,
                    video.profile.as_deref().unwrap_or("")
                ));
            }
            if (video.width, video.height) != (width, height) {
                mismatches.push(format!(
                    "{}x{}, expected {width}x{height}",
                    video.width, video.height
                ));
            }
            if video.frame_count != frames {
                mismatches.push(format!(
                    "{} frames, expected the source's {frames}",
                    video.frame_count
                ));
            }
            if video.rate != Some(rate) {
                mismatches.push(format!(
                    "rate {}, expected the source's {rate}",
                    video.rate.map_or("unknown".to_string(), |r| r.to_string())
                ));
            }
        }
    }
    if verified.audio.is_some() {
        mismatches.push("it has sound".to_string());
    }
    if !mismatches.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Err(EngineError::CopyMismatch {
            kind: "proxy".into(),
            mismatches,
        });
    }
    Ok(report(verified, false))
}

/// The video information of `asset`, which must be a video.
pub(crate) fn video_asset<'a>(
    project: &'a Project,
    asset: &Id,
) -> Result<&'a fosscut_model::VideoInfo, EngineError> {
    let found = project
        .media
        .get(asset)
        .ok_or_else(|| EngineError::AssetNotFound {
            asset: asset.to_string(),
        })?;
    found
        .info
        .video
        .as_ref()
        .filter(|_| found.kind == AssetKind::Video)
        .ok_or_else(|| EngineError::NotVideo {
            asset: asset.to_string(),
        })
}

pub(crate) fn failed_io(path: &Path, error: &std::io::Error) -> EngineError {
    EngineError::Failed {
        code: "fosscut.media.write_failed".into(),
        message: format!("cannot write {}: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use fosscut_model::{IdKind, MediaInfo, PathHints, VideoInfo};

    use super::*;

    fn asset(kind: AssetKind, width: u32, height: u32) -> Asset {
        Asset {
            folder: None,
            id: Id::new(IdKind::Asset, 1),
            kind,
            name: "clip".into(),
            path: PathHints::default(),
            identity: None,
            info: MediaInfo {
                video: Some(VideoInfo {
                    width,
                    height,
                    rate: Some(Rate::FPS_30),
                    codec: None,
                    rotation: 0,
                }),
                ..Default::default()
            },
        }
    }

    #[test]
    fn proxy_sizes_are_even_and_keep_the_aspect() {
        assert_eq!(ProxySize::Half.dimensions((3840, 2160)), (1920, 1080));
        assert_eq!(ProxySize::Quarter.dimensions((3840, 2160)), (960, 540));
        assert_eq!(ProxySize::Half.dimensions((1366, 768)), (684, 384));
        assert_eq!(ProxySize::Quarter.dimensions((1080, 1920)), (270, 480));
        assert_eq!(ProxySize::Quarter.dimensions((6, 2)), (2, 2), "at least 2");
        for size in ProxySize::ALL {
            assert_eq!(ProxySize::parse(size.name()).unwrap(), size);
        }
        assert_eq!(
            ProxySize::parse("third").unwrap_err().code(),
            "fosscut.request.invalid_arguments"
        );
    }

    #[test]
    fn the_policy_chooses_which_assets_get_proxies() {
        let uhd = asset(AssetKind::Video, 3840, 2160);
        let portrait = asset(AssetKind::Video, 1080, 2400);
        let hd = asset(AssetKind::Video, 1920, 1080);
        let image = asset(AssetKind::Image, 6000, 4000);
        assert!(wants_proxy(&uhd, ProxyPolicy::Auto));
        assert!(wants_proxy(&portrait, ProxyPolicy::Auto), "either side");
        assert!(!wants_proxy(&hd, ProxyPolicy::Auto), "1920 is not longer");
        assert!(wants_proxy(&hd, ProxyPolicy::On));
        assert!(!wants_proxy(&uhd, ProxyPolicy::Off));
        assert!(!wants_proxy(&image, ProxyPolicy::On), "only video");
        for policy in [ProxyPolicy::Off, ProxyPolicy::Auto, ProxyPolicy::On] {
            assert_eq!(ProxyPolicy::parse(policy.name()).unwrap(), policy);
        }
    }

    #[test]
    fn proxies_are_kept_by_content_size_and_version() {
        let identity = ContentIdentity {
            size: 99,
            sample_blake3: "abc123".into(),
        };
        let half = proxy_path(Path::new("/cache"), &identity, ProxySize::Half);
        assert_eq!(
            half,
            Path::new(&format!(
                "/cache/proxies/abc123-99/half@{PROXY_VERSION}.mov"
            ))
        );
        assert_ne!(
            proxy_path(Path::new("/cache"), &identity, ProxySize::Quarter),
            half
        );
    }
}
