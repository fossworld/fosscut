//! Preview and export parity (ADR-0016, item 5; ADR-0008, item 3): the preview graph at
//! half resolution and the export graph at full resolution, downscaled, show the same
//! frame. Both drive the same plan and renderer, so a difference means the paths diverged.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_model::{
    Asset, AssetKind, Clip, ClipContent, Id, IdKind, MediaInfo, PathHints, Project, TimelineFormat,
    Track, TrackKind, Transform, VideoInfo,
};
use fosscut_render::color::TransferFunction;
use fosscut_render::{ExportGraph, Image, PreviewGraph};
use fosscut_time::{Flicks, Rate};
use support::*;

fn frames(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

/// Two overlapping video clips on two tracks; the top one is moved, scaled and faded.
fn project() -> (Project, Id) {
    let format = TimelineFormat {
        width: 1280,
        height: 720,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 7), "Parity", format);
    for n in [1, 2] {
        project.media.assets.push(Asset {
            id: asset(n),
            kind: AssetKind::Video,
            name: format!("pattern-{n}.mp4"),
            path: PathHints::default(),
            identity: None,
            info: MediaInfo {
                duration: Some(frames(300)),
                container: None,
                video: Some(VideoInfo {
                    width: 1280,
                    height: 720,
                    rate: Some(Rate::FPS_30),
                    codec: None,
                    rotation: 0,
                }),
                audio: None,
            },
        });
    }
    project.manifest.next_id = 10;
    let timeline = project.manifest.primary_timeline.clone();
    let clip = |id: u64, n: u64, start: i64, transform: Transform, opacity: f64| Clip {
        id: Id::new(IdKind::Clip, id),
        start: frames(start),
        duration: frames(90),
        content: ClipContent::Video {
            asset: asset(n),
            source_in: Flicks::ZERO,
        },
        transform,
        opacity,
        volume: 1.0,
    };
    let overlay = Transform {
        position: [0.2, 0.15],
        scale: 0.4,
        rotation: -8.0,
    };
    let tracks = [
        (20, clip(30, 1, 0, Transform::default(), 1.0)),
        (21, clip(31, 2, 30, overlay, 0.7)),
    ];
    for (id, clip) in tracks {
        project
            .timelines
            .get_mut(&timeline)
            .unwrap()
            .tracks
            .push(Track {
                id: Id::new(IdKind::Track, id),
                kind: TrackKind::Video,
                name: None,
                muted: false,
                hidden: false,
                locked: false,
                clips: vec![clip],
            });
    }
    (project, timeline)
}

/// Averages each 2x2 block of an image in linear light, as the preview's mipmaps do.
fn half(image: &Image) -> Image {
    let (w, h) = (image.width / 2, image.height / 2);
    let bt709 = TransferFunction::Bt709;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let linear: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                    .iter()
                    .map(|(dx, dy)| {
                        let code = image.pixel(2 * x + dx, 2 * y + dy)[c];
                        bt709.decode(f32::from(code) / 255.0)
                    })
                    .sum::<f32>()
                    / 4.0;
                rgba.push((bt709.encode(linear).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }
    Image {
        width: w,
        height: h,
        rgba,
    }
}

#[test]
fn preview_matches_export_at_three_timestamps() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (project, timeline) = project();
    let export = ExportGraph::new(&project, &timeline).unwrap();
    let preview = PreviewGraph::new(&project, &timeline, (640, 640)).unwrap();
    assert_eq!(export.size, (1280, 720));
    assert_eq!(
        preview.size,
        (640, 360),
        "fitted to the preview box, same aspect"
    );
    // Sources at the export resolution, so export draws them 1:1 and the preview has to
    // shrink them, as with 4K sources in a 1080p preview.
    let mut provider = Frames::default();
    provider.moving.insert(asset(1), (1280, 720));
    provider.moving.insert(asset(2), (1280, 720));

    // Before the overlay, while both clips show, and on the last frame of the overlay.
    for time in [frames(10), frames(60), frames(119)] {
        let full = export
            .render(&mut renderer, &project, &mut provider, time)
            .unwrap();
        let small = preview
            .grab(&mut renderer, &project, &mut provider, time)
            .unwrap();
        // Both downscale in linear light; what remains is filtering and rounding at sharp
        // edges. Above 40 dB the difference is invisible, and a real divergence (a missing,
        // misplaced or differently blended layer) drops far below it.
        let psnr = half(&full).psnr(&small).unwrap();
        eprintln!(
            "parity at frame {}: {psnr:.1} dB",
            Rate::FPS_30.frame_at(time)
        );
        assert!(psnr > 40.0, "frame at {time:?}: PSNR {psnr:.1} dB");
    }
}

#[test]
fn export_visits_every_frame_of_the_timeline() {
    let (project, timeline) = project();
    let export = ExportGraph::new(&project, &timeline).unwrap();
    let times = export.frame_times(&project).unwrap();
    assert_eq!(times.len(), 120, "the overlay ends at frame 120");
    assert_eq!(times[0], Flicks::ZERO);
    assert_eq!(times[119], frames(119));
    let missing = ExportGraph::new(&project, &Id::new(IdKind::Timeline, 99));
    assert_eq!(missing.unwrap_err().code(), "fosscut.timeline.not_found");
    let preview = PreviewGraph::new(&project, &timeline, (4000, 4000)).unwrap();
    assert_eq!(preview.size, (1280, 720), "preview never upscales");
}

#[test]
fn graphs_render_into_caller_targets_and_report_missing_frames() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (project, timeline) = project();
    let preview = PreviewGraph::new(&project, &timeline, (320, 180)).unwrap();
    let mut empty = Frames::default();
    let error = preview
        .grab(&mut renderer, &project, &mut empty, frames(10))
        .unwrap_err();
    assert_eq!(error.code(), "fosscut.render.missing_frame");

    // A window surface is just a texture view to the renderer.
    let texture = renderer.device().create_texture(&wgpu_texture((320, 180)));
    let view = texture.create_view(&Default::default());
    let mut provider = Frames::default();
    provider.moving.insert(asset(1), (640, 360));
    preview
        .render_to(
            &mut renderer,
            &project,
            &mut provider,
            frames(10),
            &view,
            texture.format(),
        )
        .unwrap();
    let _ = renderer.queue();
}

fn wgpu_texture(size: (u32, u32)) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("window stand-in"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    }
}

#[test]
fn text_matches_between_preview_and_export() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (mut project, timeline) = project();
    let title = Clip {
        id: Id::new(IdKind::Clip, 40),
        start: Flicks::ZERO,
        duration: frames(90),
        content: ClipContent::Text(fosscut_model::TextContent {
            text: "Preview equals export".into(),
            style: fosscut_model::TextStyle {
                size: 0.08,
                bold: true,
                ..fosscut_model::TextStyle::default()
            },
        }),
        transform: Transform {
            position: [0.0, 0.3],
            ..Transform::default()
        },
        opacity: 1.0,
        volume: 1.0,
    };
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 22),
            kind: TrackKind::Text,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            clips: vec![title],
        });
    let export = ExportGraph::new(&project, &timeline).unwrap();
    let preview = PreviewGraph::new(&project, &timeline, (640, 640)).unwrap();
    let mut provider = Frames::default();
    provider.moving.insert(asset(1), (1280, 720));
    provider.moving.insert(asset(2), (1280, 720));
    let time = frames(10);
    let full = export
        .render(&mut renderer, &project, &mut provider, time)
        .unwrap();
    let small = preview
        .grab(&mut renderer, &project, &mut provider, time)
        .unwrap();
    let reference = half(&full);
    let psnr = reference.psnr(&small).unwrap();
    eprintln!("text parity: {psnr:.1} dB");
    // Text is rasterized at each output size rather than scaled, so glyph edges differ
    // between 1280x720 and 640x360 (35.6 dB on Apple M3 Pro; rasterization is on the CPU,
    // so other GPUs measure the same). A missing or misplaced title costs far more.
    assert!(psnr > 33.0, "text parity: PSNR {psnr:.1} dB");

    // The title must sit in the same place at both sizes: no one-pixel shift of the
    // preview may match the export better than no shift.
    for (dx, dy) in [
        (-1, 0),
        (1, 0),
        (0, -1),
        (0, 1),
        (-1, -1),
        (1, 1),
        (-1, 1),
        (1, -1),
    ] {
        let mut shifted = small.clone();
        for y in 1..small.height - 1 {
            for x in 1..small.width - 1 {
                let p = small.pixel((x as i32 + dx) as u32, (y as i32 + dy) as u32);
                let i = ((y * small.width + x) * 4) as usize;
                shifted.rgba[i..i + 4].copy_from_slice(&p);
            }
        }
        let moved = reference.psnr(&shifted).unwrap();
        assert!(
            moved + 2.0 < psnr,
            "a ({dx}, {dy}) shift matches better: {moved:.1} vs {psnr:.1} dB"
        );
    }
}
