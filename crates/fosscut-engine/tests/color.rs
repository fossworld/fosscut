//! Automatic color and LUT files on generated media: clips are measured through the
//! export's color pipeline, suggestions move towards neutral or towards a reference, and
//! `color.lut` effects read `.cube` files from the media table in frame grabs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::process::Command;

use common::{frames, project, renderer, workspace};
use fosscut_engine::color::{clip_stats, suggest};
use fosscut_engine::frame::frame;
use fosscut_model::{
    Asset, AssetKind, AssetRef, Clip, ClipContent, Effect, Id, IdKind, LutInfo, MediaInfo,
    ParamValue, PathHints, Project, VideoInfo,
};
use fosscut_time::Rate;

fn bundle() -> std::path::PathBuf {
    workspace().join("Show.fosscut")
}

/// A one-second flat blue-gray clip next to the fixture, made once.
fn blue_clip() -> std::path::PathBuf {
    let path = workspace().join("blue.mp4");
    if !path.exists() {
        let status = Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("color=c=0x405080:s=160x90:r=30:d=1")
            .args(["-c:v", "mpeg4", "-q:v", "2"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
    }
    path
}

fn with_blue(project: &mut Project) -> Id {
    blue_clip();
    let asset = Id::new(IdKind::Asset, 5);
    project.media.assets.push(Asset {
        folder: None,
        id: asset.clone(),
        kind: AssetKind::Video,
        name: "blue.mp4".into(),
        path: PathHints {
            relative: Some("../blue.mp4".into()),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            duration: Some(frames(30)),
            video: Some(VideoInfo {
                width: 160,
                height: 90,
                rate: Some(Rate::FPS_30),
                codec: None,
                rotation: 0,
            }),
            ..MediaInfo::default()
        },
    });
    let clip = Id::new(IdKind::Clip, 6);
    let timeline = project.manifest.primary_timeline.clone();
    let doc = project.timelines.get_mut(&timeline).unwrap();
    doc.tracks[0].clips.push(Clip::new(
        clip.clone(),
        frames(30),
        frames(30),
        ClipContent::Video {
            asset,
            source_in: frames(0),
        },
    ));
    clip
}

#[test]
fn clips_are_measured_and_corrected_towards_neutral() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut project = project("../clip.mp4");
    let blue = with_blue(&mut project);
    let stats = clip_stats(&project, &bundle(), &blue, &mut renderer).unwrap();
    assert!(
        stats.mean[2] > stats.mean[0] + 0.1,
        "a blue cast: {stats:?}"
    );
    assert!((stats.low - stats.high).abs() < 0.02, "flat");
    let fix = suggest(&project, &bundle(), &blue, None, &mut renderer).unwrap();
    assert_eq!(fix.effect_add["clip"], blue.as_str());
    assert_eq!(fix.effect_add["type"], "color.adjust");
    assert_eq!(fix.effect_add["index"], 0);
    let params = &fix.effect_add["params"];
    assert!(
        params["temperature"].as_f64().unwrap() > 0.0,
        "warmer: {params}"
    );

    let pattern = Id::new(IdKind::Clip, 3);
    let matched = suggest(&project, &bundle(), &pattern, Some(&blue), &mut renderer).unwrap();
    let reference = matched.reference.unwrap();
    assert_eq!(reference, stats);
    let params = &matched.effect_add["params"];
    assert!(
        params["temperature"].as_f64().unwrap() < 0.0,
        "cooler, towards the blue clip: {params}"
    );
    assert!(
        params["saturation"].as_f64().unwrap() < 0.0,
        "the blue clip is muted"
    );
    let itself = suggest(&project, &bundle(), &blue, Some(&blue), &mut renderer);
    assert!(itself.is_err());
    let title = Id::new(IdKind::Clip, 5);
    let words = suggest(&project, &bundle(), &title, None, &mut renderer).unwrap_err();
    assert!(words.to_string().contains("text clip"), "{words}");
}

#[test]
fn lut_effects_read_cube_files_in_frame_grabs() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let cube = workspace().join("invert.cube");
    let mut text = String::from("LUT_3D_SIZE 2\n");
    for b in [0.0, 1.0] {
        for g in [0.0, 1.0] {
            for r in [0.0, 1.0] {
                text.push_str(&format!("{} {} {}\n", 1.0 - r, 1.0 - g, 1.0 - b));
            }
        }
    }
    std::fs::write(&cube, text).unwrap();
    let probed = fosscut_engine::import::probe(&cube).unwrap();
    assert_eq!(probed.kind, AssetKind::Lut);
    assert_eq!(probed.info.lut, Some(LutInfo { size: 2 }));

    let mut project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let plain = frame(
        &project,
        &bundle(),
        &timeline,
        frames(20),
        None,
        &mut renderer,
    )
    .unwrap();
    let lut = Id::new(IdKind::Asset, 7);
    project.media.assets.push(Asset {
        folder: None,
        id: lut.clone(),
        kind: AssetKind::Lut,
        name: "invert.cube".into(),
        path: PathHints {
            relative: Some("../invert.cube".into()),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            lut: Some(LutInfo { size: 2 }),
            ..MediaInfo::default()
        },
    });
    let doc = project.timelines.get_mut(&timeline).unwrap();
    doc.tracks[0].clips[0].effects.push(Effect {
        id: Id::new(IdKind::Effect, 8),
        effect_type: "color.lut".into(),
        version: 1,
        enabled: true,
        params: [(
            "lut".to_string(),
            ParamValue::Asset(AssetRef { asset: lut.clone() }),
        )]
        .into_iter()
        .collect(),
        keyframes: Default::default(),
    });
    assert!(fosscut_model::validate(&project).is_empty());
    let inverted = frame(
        &project,
        &bundle(),
        &timeline,
        frames(20),
        None,
        &mut renderer,
    )
    .unwrap();
    let (a, b) = (plain.image.pixel(160, 150), inverted.image.pixel(160, 150));
    for c in 0..3 {
        let sum = u16::from(a[c]) + u16::from(b[c]);
        assert!(
            (235..=275).contains(&sum),
            "channel {c} inverted: {a:?} and {b:?}"
        );
    }
    std::fs::remove_file(&cube).unwrap();
    let missing = frame(
        &project,
        &bundle(),
        &timeline,
        frames(20),
        None,
        &mut renderer,
    );
    assert!(
        missing.is_err(),
        "a missing LUT file is an error, not a silent skip"
    );
    let bad = workspace().join("bad.cube");
    std::fs::write(&bad, "LUT_3D_SIZE 1\n0 0 0\n").unwrap();
    assert!(fosscut_engine::import::probe(&bad).is_err());
}
