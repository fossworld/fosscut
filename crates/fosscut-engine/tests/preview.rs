//! The desktop preview (ADR-0008, ADR-0010): it shows exactly what frame grabs and exports
//! show, holds the first and last frame outside the timeline, and keeps media open across
//! edits.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{frames, project, renderer, workspace};
use fosscut_engine::EngineError;
use fosscut_engine::frame::frame;
use fosscut_engine::preview::{Previewer, Sound};
use fosscut_model::{Id, IdKind, Project, TrackKind};
use fosscut_time::Flicks;

fn bundle() -> std::path::PathBuf {
    workspace().join("Show.fosscut")
}

fn untitled(project: &Project) -> Project {
    let mut copy = project.clone();
    let timeline = copy.manifest.primary_timeline.clone();
    copy.timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .retain(|t| t.kind != TrackKind::Text);
    copy
}

#[test]
fn the_preview_shows_what_a_frame_grab_shows() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let mut previewer = Previewer::new();
    for at in [frames(0), frames(6), frames(20), frames(29)] {
        let shown = previewer
            .grab(
                &mut renderer,
                &project,
                &bundle(),
                &timeline,
                at,
                (320, 180),
            )
            .unwrap();
        let grabbed = frame(&project, &bundle(), &timeline, at, None, &mut renderer).unwrap();
        assert_eq!(shown.at, grabbed.at);
        assert_eq!(shown.image, grabbed.image, "frame {}", grabbed.at.frames);
    }

    // Larger windows never render above the canvas; smaller ones keep its aspect.
    assert_eq!(
        Previewer::size(&project, &timeline, (1920, 1080)).unwrap(),
        (320, 180)
    );
    let small = previewer
        .grab(
            &mut renderer,
            &project,
            &bundle(),
            &timeline,
            frames(6),
            (200, 200),
        )
        .unwrap();
    assert_eq!((small.image.width, small.image.height), (200, 113));
}

#[test]
fn times_outside_the_timeline_hold_its_first_and_last_frames() {
    let project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let at = |time: Flicks| Previewer::shown_at(&project, &timeline, time).unwrap();
    assert_eq!(at(Flicks::new(-5)).frames, 0);
    assert_eq!(at(frames(12) + Flicks::new(9)).frames, 12);
    assert_eq!(at(frames(12)).flicks, frames(12));
    assert_eq!(
        at(frames(30)).frames,
        29,
        "the 1 s timeline ends after frame 29"
    );
    assert_eq!(at(frames(900)).frames, 29);

    let mut empty = project.clone();
    empty.timelines.get_mut(&timeline).unwrap().tracks.clear();
    assert_eq!(
        Previewer::shown_at(&empty, &timeline, frames(40))
            .unwrap()
            .frames,
        0
    );
}

#[test]
fn edits_render_with_the_media_already_open() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let mut previewer = Previewer::new();
    let before = previewer
        .grab(
            &mut renderer,
            &project,
            &bundle(),
            &timeline,
            frames(6),
            (320, 180),
        )
        .unwrap();
    let edited = untitled(&project);
    let after = previewer
        .grab(
            &mut renderer,
            &edited,
            &bundle(),
            &timeline,
            frames(6),
            (320, 180),
        )
        .unwrap();
    let expected = frame(
        &edited,
        &bundle(),
        &timeline,
        frames(6),
        None,
        &mut renderer,
    )
    .unwrap();
    assert_eq!(after.image, expected.image);
    assert_ne!(after.image, before.image, "the title is gone");

    // An empty timeline previews as black.
    let mut empty = project.clone();
    empty.timelines.get_mut(&timeline).unwrap().tracks.clear();
    let black = previewer
        .grab(
            &mut renderer,
            &empty,
            &bundle(),
            &timeline,
            frames(3),
            (320, 180),
        )
        .unwrap();
    assert!(black.image.rgba.chunks(4).all(|px| px[..3] == [0, 0, 0]));
}

#[test]
fn bad_requests_get_typed_errors() {
    let project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let unknown = Id::new(IdKind::Timeline, 99);
    assert!(matches!(
        Previewer::shown_at(&project, &unknown, Flicks::ZERO),
        Err(EngineError::TimelineNotFound { .. })
    ));
    assert!(matches!(
        Previewer::size(&project, &timeline, (0, 100)),
        Err(EngineError::InvalidArguments { .. })
    ));
    assert!(Previewer::size(&project, &unknown, (100, 100)).is_err());
}

fn has_ffmpeg() -> bool {
    let present = std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        present || !common::required("FOSSCUT_REQUIRE_FFMPEG"),
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    present
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

#[test]
fn playback_sound_is_the_timeline_mixed_in_spans() {
    if !has_ffmpeg() {
        return;
    }
    let project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let mut sound = Sound::new();
    let half = Flicks::from_millis(500);
    let first = sound
        .mix(
            &project,
            &bundle(),
            &timeline,
            Flicks::ZERO,
            half,
            48_000,
            2,
        )
        .unwrap();
    assert_eq!((first.sample_rate, first.channels), (48_000, 2));
    assert_eq!(first.samples.len(), 48_000);
    assert!(rms(&first.samples) > 0.02, "the clip's tone plays");
    // The next span continues from the same decoder; past the 1 s timeline it is silent.
    let second = sound
        .mix(&project, &bundle(), &timeline, half, half, 48_000, 2)
        .unwrap();
    assert!(rms(&second.samples) > 0.02);
    let after = sound
        .mix(
            &project,
            &bundle(),
            &timeline,
            Flicks::from_millis(1200),
            half,
            48_000,
            2,
        )
        .unwrap();
    assert_eq!(rms(&after.samples), 0.0);

    let gone = common::project("../gone.mp4");
    let error = Sound::new()
        .mix(&gone, &bundle(), &timeline, Flicks::ZERO, half, 48_000, 2)
        .unwrap_err();
    assert!(error.to_string().contains("asset"), "{error}");
}
