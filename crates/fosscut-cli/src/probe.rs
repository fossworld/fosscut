//! Media probing with ffprobe and content identity hashing.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path};
use std::process::Command;

use fosscut_core::{ApiError, codes};
use fosscut_model::{AssetKind, AudioInfo, ContentIdentity, MediaInfo, PathHints, VideoInfo};
use fosscut_time::{Flicks, Rate};
use serde_json::{Value, json};

const SAMPLE: u64 = 1 << 20;

/// Probed media ready for `media.add`.
pub struct Probed {
    pub kind: AssetKind,
    pub info: MediaInfo,
}

fn ffprobe_path() -> String {
    std::env::var("FOSSCUT_FFPROBE").unwrap_or_else(|_| "ffprobe".to_string())
}

/// Runs ffprobe on `file`.
pub fn probe(file: &Path) -> Result<Probed, ApiError> {
    let output = Command::new(ffprobe_path())
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(file)
        .output()
        .map_err(|e| ApiError::new(codes::PROBE_UNAVAILABLE, format!("cannot run ffprobe: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ApiError::new(
            codes::PROBE_FAILED,
            format!("ffprobe could not read {}", file.display()),
        )
        .with_details(json!({ "ffprobe": stderr })));
    }
    let report: Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("unexpected ffprobe output: {e}"),
        )
    })?;
    interpret(&report).ok_or_else(|| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!(
                "{} has no usable audio, video or image stream",
                file.display()
            ),
        )
    })
}

fn interpret(report: &Value) -> Option<Probed> {
    let format = &report["format"];
    let format_name = format["format_name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let streams = report["streams"].as_array()?;
    let is_cover = |s: &Value| s["disposition"]["attached_pic"].as_i64() == Some(1);
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && !is_cover(s));
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    let duration = format["duration"]
        .as_str()
        .and_then(|d| Flicks::parse_seconds(d).ok())
        .filter(|d| d.is_positive());

    let still_formats = [
        "image2",
        "png_pipe",
        "jpeg_pipe",
        "webp_pipe",
        "bmp_pipe",
        "tiff_pipe",
    ];
    let is_still = video.is_some()
        && audio.is_none()
        && (still_formats.iter().any(|f| format_name.contains(f)) || duration.is_none());

    let video_info = video.map(|s| VideoInfo {
        width: s["width"].as_u64().unwrap_or(0) as u32,
        height: s["height"].as_u64().unwrap_or(0) as u32,
        rate: ["r_frame_rate", "avg_frame_rate"]
            .iter()
            .filter_map(|key| s[*key].as_str())
            .find_map(|r| r.parse::<Rate>().ok())
            .filter(|_| !is_still),
        codec: s["codec_name"].as_str().map(str::to_string),
        rotation: rotation(s),
    });
    let audio_info = audio.map(|s| AudioInfo {
        sample_rate: s["sample_rate"]
            .as_str()
            .and_then(|r| r.parse().ok())
            .unwrap_or(0),
        channels: s["channels"].as_u64().unwrap_or(0) as u16,
        codec: s["codec_name"].as_str().map(str::to_string),
    });
    let kind = if is_still {
        AssetKind::Image
    } else if video_info.is_some() {
        AssetKind::Video
    } else if audio_info.is_some() {
        AssetKind::Audio
    } else {
        return None;
    };
    Some(Probed {
        kind,
        info: MediaInfo {
            duration: if kind == AssetKind::Image {
                None
            } else {
                duration
            },
            container: (!format_name.is_empty()).then_some(format_name),
            video: video_info,
            audio: if kind == AssetKind::Image {
                None
            } else {
                audio_info
            },
        },
    })
}

fn rotation(stream: &Value) -> i32 {
    if let Some(r) = stream["tags"]["rotate"]
        .as_str()
        .and_then(|r| r.parse().ok())
    {
        return r;
    }
    stream["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|d| d["rotation"].as_f64()))
        .map(|r| r.round() as i32)
        .unwrap_or(0)
}

/// Size plus a BLAKE3 hash of the size, the first MiB and the last MiB.
pub fn identity(file: &Path) -> Result<ContentIdentity, ApiError> {
    let error = |e: std::io::Error| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("cannot read {}: {e}", file.display()),
        )
    };
    let mut handle = File::open(file).map_err(error)?;
    let size = handle.metadata().map_err(error)?.len();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    let mut buffer = vec![0u8; SAMPLE.min(size) as usize];
    handle.read_exact(&mut buffer).map_err(error)?;
    hasher.update(&buffer);
    if size > SAMPLE {
        let tail = SAMPLE.min(size - SAMPLE);
        handle.seek(SeekFrom::End(-(tail as i64))).map_err(error)?;
        let mut buffer = vec![0u8; tail as usize];
        handle.read_exact(&mut buffer).map_err(error)?;
        hasher.update(&buffer);
    }
    Ok(ContentIdentity {
        size,
        sample_blake3: hasher.finalize().to_hex().to_string(),
    })
}

/// Absolute path and the path relative to the bundle directory.
pub fn path_hints(file: &Path, bundle: &Path) -> Result<PathHints, ApiError> {
    let absolute = std::fs::canonicalize(file).map_err(|e| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("cannot find {}: {e}", file.display()),
        )
    })?;
    let relative = std::fs::canonicalize(bundle)
        .ok()
        .and_then(|bundle| relative_path(&absolute, &bundle));
    Ok(PathHints {
        relative,
        absolute: Some(absolute.to_string_lossy().into_owned()),
    })
}

fn relative_path(target: &Path, base: &Path) -> Option<String> {
    let target: Vec<Component> = target.components().collect();
    let base: Vec<Component> = base.components().collect();
    if target.first() != base.first() {
        return None;
    }
    let common = target.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> =
        std::iter::repeat_n("..".to_string(), base.len() - common).collect();
    parts.extend(
        target[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interprets_a_video_report() {
        let report = json!({
            "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "8.000000" },
            "streams": [
                { "codec_type": "video", "codec_name": "h264", "width": 1280, "height": 720, "r_frame_rate": "30/1" },
                { "codec_type": "audio", "codec_name": "aac", "sample_rate": "48000", "channels": 1 }
            ]
        });
        let probed = interpret(&report).unwrap();
        assert_eq!(probed.kind, AssetKind::Video);
        assert_eq!(probed.info.duration, Some(Flicks::from_seconds(8).unwrap()));
        assert_eq!(probed.info.video.unwrap().rate, Some(Rate::FPS_30));
    }

    #[test]
    fn interprets_images_and_audio_with_cover_art() {
        let image = json!({
            "format": { "format_name": "png_pipe" },
            "streams": [{ "codec_type": "video", "codec_name": "png", "width": 64, "height": 64, "r_frame_rate": "25/1" }]
        });
        assert_eq!(interpret(&image).unwrap().kind, AssetKind::Image);
        let song = json!({
            "format": { "format_name": "mp3", "duration": "3.5" },
            "streams": [
                { "codec_type": "audio", "codec_name": "mp3", "sample_rate": "44100", "channels": 2 },
                { "codec_type": "video", "codec_name": "mjpeg", "disposition": { "attached_pic": 1 } }
            ]
        });
        let probed = interpret(&song).unwrap();
        assert_eq!(probed.kind, AssetKind::Audio);
        assert!(probed.info.video.is_none());
    }

    #[test]
    fn relative_paths_climb_out_of_the_bundle() {
        let rel = relative_path(
            Path::new("/a/media/clip.mp4"),
            Path::new("/a/projects/Promo.fosscut"),
        );
        assert_eq!(rel.as_deref(), Some("../../media/clip.mp4"));
    }
}
