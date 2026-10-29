//! Frame grabs and contact sheets (ADR-0002, item 4) on generated media: they show exactly
//! what the timeline plans at each moment, at the requested size, with typed errors.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{frames, project, renderer, workspace};
use fosscut_engine::EngineError;
use fosscut_engine::frame::{contact_sheet, frame};
use fosscut_model::{Id, IdKind, Project, TrackKind};
use fosscut_render::Image;
use fosscut_time::Flicks;

fn bundle() -> std::path::PathBuf {
    workspace().join("Show.fosscut")
}

/// The project without its title track.
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

/// The largest difference of any colour channel between two images of the same size.
fn largest_difference(a: &Image, b: &Image) -> u8 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.rgba
        .iter()
        .zip(&b.rgba)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap()
}

#[test]
fn frames_show_what_the_timeline_plans_at_that_moment() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let project = project("../clip.mp4");
    let plain = untitled(&project);
    let timeline = project.manifest.primary_timeline.clone();
    let at = |project: &Project, time: Flicks, renderer: &mut _| {
        frame(project, &bundle(), &timeline, time, None, renderer).unwrap()
    };

    // The title is on for the first 15 frames only.
    let titled = at(&project, frames(6), &mut renderer);
    assert_eq!(titled.at.frames, 6);
    assert_eq!((titled.image.width, titled.image.height), (320, 180));
    let without = at(&plain, frames(6), &mut renderer);
    assert!(largest_difference(&titled.image, &without.image) > 100);
    let later = at(&project, frames(20), &mut renderer);
    assert_eq!(
        largest_difference(&later.image, &at(&plain, frames(20), &mut renderer).image),
        0
    );

    // A time inside a frame shows that frame; different frames of the clip differ.
    let inside = at(&project, frames(20) + Flicks::new(1000), &mut renderer);
    assert_eq!(inside.at.frames, 20);
    assert_eq!(inside.at.flicks, frames(20));
    assert_eq!(inside.image, later.image);
    assert!(largest_difference(&later.image, &at(&project, frames(28), &mut renderer).image) > 0);

    // Smaller sizes keep the aspect ratio.
    let small = frame(
        &project,
        &bundle(),
        &timeline,
        frames(6),
        Some(160),
        &mut renderer,
    )
    .unwrap();
    assert_eq!((small.image.width, small.image.height), (160, 90));
}

#[test]
fn contact_sheets_tile_evenly_spaced_frames() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let sheet = contact_sheet(&project, &bundle(), &timeline, 4, 80, &mut renderer).unwrap();

    // Four 80x45 tiles in two columns with 4-pixel gaps.
    assert_eq!(sheet.columns, 2);
    assert_eq!((sheet.image.width, sheet.image.height), (164, 94));
    let positions: Vec<(u32, u32)> = sheet.tiles.iter().map(|t| (t.x, t.y)).collect();
    assert_eq!(positions, [(0, 0), (84, 0), (0, 49), (84, 49)]);

    // Each tile shows the middle of its quarter of the 1 s timeline, exactly as a frame grab.
    let shown: Vec<i64> = sheet.tiles.iter().map(|t| t.at.frames).collect();
    assert_eq!(shown, [3, 11, 18, 26]);
    for tile in &sheet.tiles {
        let still = frame(
            &project,
            &bundle(),
            &timeline,
            tile.at.flicks,
            Some(80),
            &mut renderer,
        )
        .unwrap();
        for row in 0..tile.height {
            let start = (((tile.y + row) * sheet.image.width + tile.x) * 4) as usize;
            let line = &sheet.image.rgba[start..start + (tile.width * 4) as usize];
            let expected = &still.image.rgba[(row * tile.width * 4) as usize..][..line.len()];
            assert_eq!(line, expected, "tile at frame {} row {row}", tile.at.frames);
        }
    }
    // The gaps are the dark background.
    let gap = ((10 * sheet.image.width + 81) * 4) as usize;
    assert_eq!(&sheet.image.rgba[gap..gap + 4], &[24, 24, 24, 255]);

    // One tile is just the frame.
    let single = contact_sheet(&project, &bundle(), &timeline, 1, 80, &mut renderer).unwrap();
    assert_eq!((single.image.width, single.image.height), (80, 45));
    assert_eq!(single.tiles[0].at.frames, 15);
}

#[test]
fn impossible_frames_are_typed_errors() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let grab =
        |project: &Project, timeline: &Id, time: Flicks, width: Option<u32>, renderer: &mut _| {
            frame(project, &bundle(), timeline, time, width, renderer).unwrap_err()
        };

    let end = grab(&project, &timeline, frames(30), None, &mut renderer);
    assert_eq!(
        end,
        EngineError::TimeOutOfRange {
            time: frames(30),
            duration: frames(30)
        }
    );
    assert!(end.hint().is_some());
    let before = grab(&project, &timeline, Flicks::new(-1), None, &mut renderer);
    assert_eq!(before.code(), "fosscut.time.out_of_range");
    assert_eq!(
        grab(&project, &timeline, frames(1), Some(0), &mut renderer).code(),
        "fosscut.request.invalid_arguments"
    );
    assert_eq!(
        grab(&project, &timeline, frames(1), Some(5000), &mut renderer).code(),
        "fosscut.request.invalid_arguments"
    );
    let unknown = Id::new(IdKind::Timeline, 99);
    assert_eq!(
        grab(&project, &unknown, frames(1), None, &mut renderer).code(),
        "fosscut.timeline.not_found"
    );
    let missing = common::project("missing.mp4");
    assert_eq!(
        grab(&missing, &timeline, frames(20), None, &mut renderer).code(),
        "fosscut.export.missing_media"
    );

    let mut empty = project.clone();
    empty.timelines.get_mut(&timeline).unwrap().tracks.clear();
    assert_eq!(
        grab(&empty, &timeline, frames(1), None, &mut renderer).code(),
        "fosscut.export.empty_timeline"
    );
    for count in [0, 101] {
        assert_eq!(
            contact_sheet(&project, &bundle(), &timeline, count, 80, &mut renderer)
                .unwrap_err()
                .code(),
            "fosscut.request.invalid_arguments"
        );
    }
}
