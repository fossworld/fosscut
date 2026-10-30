//! Fixtures shared by the engine's integration tests: a generated 2 s clip with a tone, and
//! a 1 s project that shows it under a title.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use fosscut_model::{
    Asset, AssetKind, AudioInfo, Clip, ClipContent, Id, IdKind, MediaInfo, PathHints, Project,
    TextContent, TextStyle, TimelineFormat, Track, TrackKind, Transform, VideoInfo,
};
use fosscut_render::Renderer;
use fosscut_time::{Flicks, Rate};

pub fn required(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty() && v != "0")
}

/// A renderer, or `None` to skip when FFmpeg or a GPU adapter is missing locally.
pub fn renderer() -> Option<Renderer> {
    let ffmpeg = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        ffmpeg || !required("FOSSCUT_REQUIRE_FFMPEG"),
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    let renderer = Renderer::headless();
    assert!(
        renderer.is_ok() || !required("FOSSCUT_REQUIRE_GPU"),
        "FOSSCUT_REQUIRE_GPU is set but there is no adapter"
    );
    if !ffmpeg {
        return None;
    }
    renderer.ok()
}

pub fn workspace() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-engine-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Show.fosscut")).unwrap();
        let status = Command::new("ffmpeg")
            .current_dir(&dir)
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=30:duration=2",
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=2",
            ])
            .args([
                "-c:v",
                "mpeg4",
                "-q:v",
                "4",
                "-c:a",
                "aac",
                "-shortest",
                "clip.mp4",
            ])
            .status()
            .unwrap();
        assert!(status.success());
        dir
    })
}

pub fn frames(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

/// A 1 s, 320x180 timeline: the clip from 0.5 s, and a title for the first half.
pub fn project(relative: &str) -> Project {
    let format = TimelineFormat {
        width: 320,
        height: 180,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Show", format);
    let asset = Id::new(IdKind::Asset, 1);
    project.media.assets.push(Asset {
        id: asset.clone(),
        kind: AssetKind::Video,
        name: "clip.mp4".into(),
        path: PathHints {
            relative: Some(relative.into()),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            duration: Some(frames(60)),
            container: None,
            video: Some(VideoInfo {
                width: 320,
                height: 180,
                rate: Some(Rate::FPS_30),
                codec: None,
                rotation: 0,
            }),
            audio: Some(AudioInfo {
                sample_rate: 48_000,
                channels: 1,
                codec: None,
            }),
        },
    });
    project.manifest.next_id = 10;
    let timeline = project.manifest.primary_timeline.clone();
    let doc = project.timelines.get_mut(&timeline).unwrap();
    doc.tracks.push(Track {
        id: Id::new(IdKind::Track, 2),
        kind: TrackKind::Video,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        caption_style: None,
        clips: vec![Clip {
            id: Id::new(IdKind::Clip, 3),
            start: Flicks::ZERO,
            duration: frames(30),
            content: ClipContent::Video {
                asset,
                source_in: frames(15),
            },
            transform: Transform::default(),
            opacity: 1.0,
            volume: 1.0,
        }],
    });
    doc.tracks.push(Track {
        id: Id::new(IdKind::Track, 4),
        kind: TrackKind::Text,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        caption_style: None,
        clips: vec![Clip {
            id: Id::new(IdKind::Clip, 5),
            start: Flicks::ZERO,
            duration: frames(15),
            content: ClipContent::Text(TextContent {
                text: "Title".into(),
                style: TextStyle::default(),
            }),
            transform: Transform::default(),
            opacity: 1.0,
            volume: 1.0,
        }],
    });
    project
}
