//! Effects that read neighbouring frames, and optical-flow slow motion, end to end on
//! generated media (ADR-0035, ADR-0008): the preview playing forward, frame grabs reaching
//! a frame straight away and an export all read the same neighbouring frames from the
//! decoders, so a frame looks the same however it is reached.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{frames, project, renderer, workspace};
use fosscut_engine::frame::frame;
use fosscut_engine::preview::Previewer;
use fosscut_engine::{ExportRequest, Preset, export};
use fosscut_media::DecoderOptions;
use fosscut_media::ffmpeg::FfmpegVideoDecoder;
use fosscut_model::{
    Effect, Id, IdKind, Interpolate, ParamValue, Project, Speed, TrackKind, validate,
};
use fosscut_render::color::TransferFunction;
use fosscut_render::plan::{Layer, RenderPlan, SourceRef};
use fosscut_render::{DecoderFrames, ExportGraph, Image, Renderer};

fn bundle() -> std::path::PathBuf {
    workspace().join("Show.fosscut")
}

fn effect(id: u64, kind: &str, params: &[(&str, f64)]) -> Effect {
    Effect {
        id: Id::new(IdKind::Effect, id),
        effect_type: kind.into(),
        version: 1,
        enabled: true,
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), ParamValue::Number(*v)))
            .collect(),
        keyframes: Default::default(),
    }
}

/// The shared 1 s timeline without its title, its clip at half speed with in-between
/// frames along the flow, flicker removal, temporal noise reduction and motion blur.
fn temporal_project() -> Project {
    let mut project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let doc = project.timelines.get_mut(&timeline).unwrap();
    doc.tracks.retain(|t| t.kind != TrackKind::Text);
    let clip = &mut doc.tracks[0].clips[0];
    clip.effects = vec![
        effect(40, "color.deflicker", &[]),
        effect(41, "detail.denoise", &[("strength", 1.0)]),
        effect(42, "motion.blur", &[("amount", 1.0)]),
    ];
    clip.speed = Some(Speed {
        interpolate: Interpolate::Flow,
        ..Speed::constant(0.5)
    });
    project
}

/// `frame` drawn on its own at 320x180, as the export's decoded frames are compared.
fn drawn(
    renderer: &mut Renderer,
    decoder: FfmpegVideoDecoder,
    time: fosscut_time::Flicks,
) -> Image {
    let asset = Id::new(IdKind::Asset, 99);
    let mut frames = DecoderFrames::new();
    frames.insert(asset.clone(), Box::new(decoder));
    let plan = RenderPlan {
        canvas: (320, 180),
        layers: vec![Layer::new(SourceRef { asset, time })],
        skipped: Vec::new(),
    };
    renderer
        .render_image(&plan, &mut frames, (320, 180), TransferFunction::Bt709)
        .unwrap()
}

#[test]
fn temporal_frames_are_the_same_in_preview_grabs_and_exports() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let project = temporal_project();
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let timeline = project.manifest.primary_timeline.clone();

    // The preview plays forward over the whole clip, as playback does.
    let mut previewer = Previewer::new();
    let shown: Vec<Image> = (0..30)
        .map(|n| {
            previewer
                .grab(
                    &mut renderer,
                    &project,
                    &bundle(),
                    &timeline,
                    frames(n),
                    (320, 180),
                )
                .unwrap()
                .image
        })
        .collect();
    // Frame grabs open the media afresh and reach each frame straight away: the same
    // pixels, neighbouring frames and flow included.
    for n in [0, 7, 16, 29] {
        let grabbed = frame(
            &project,
            &bundle(),
            &timeline,
            frames(n),
            None,
            &mut renderer,
        )
        .unwrap();
        assert_eq!(shown[n as usize], grabbed.image, "frame {n}");
    }
    // The effects change the picture: the same timeline without them looks different.
    let mut plain = project.clone();
    plain.timelines.get_mut(&timeline).unwrap().tracks[0].clips[0]
        .effects
        .clear();
    let untouched = frame(
        &plain,
        &bundle(),
        &timeline,
        frames(16),
        None,
        &mut renderer,
    )
    .unwrap();
    let changed = untouched.image.psnr(&shown[16]).unwrap();
    eprintln!("the effects change frame 16 to {changed:.1} dB");
    assert!(changed < 45.0, "the effects show: {changed:.1} dB");

    // The export's own render path: the export graph over decoders that read forward,
    // every frame in order. Exactly the preview's pixels.
    let export_graph = ExportGraph::new(&project, &timeline).unwrap();
    let mut decoders = DecoderFrames::new();
    decoders.insert(
        Id::new(IdKind::Asset, 1),
        Box::new(
            FfmpegVideoDecoder::open(&workspace().join("clip.mp4"), DecoderOptions::default())
                .unwrap(),
        ),
    );
    for (n, time) in export_graph
        .frame_times(&project)
        .unwrap()
        .into_iter()
        .enumerate()
    {
        let rendered = export_graph
            .render(&mut renderer, &project, &mut decoders, time)
            .unwrap();
        assert_eq!(rendered, shown[n], "frame {n} rendered for export");
    }

    // An export renders every frame in order and encodes it. Decoded, each frame matches
    // the preview's as closely as an export without the effects matches its own frame
    // grabs: all that differs is what the codec loses (4:2:2 chroma at 320x180 costs
    // about 31 dB on testsrc2's sharp coloured edges).
    let Some(preset) = [Preset::MovProResHq, Preset::Mp4H264]
        .into_iter()
        .find(|p| p.available())
    else {
        return;
    };
    let exported = |project: &Project, name: &str, renderer: &mut Renderer| {
        let output = workspace().join(format!("{name}.{}", preset.extension()));
        let request = ExportRequest {
            bundle: bundle(),
            settings: preset.settings(&project.timelines[&timeline].format),
            timeline: timeline.clone(),
            output: output.clone(),
            allow_overwrite: true,
            expected_revision: None,
            request_id: None,
            cache: workspace().join("cache"),
            options: Default::default(),
        };
        let report = export(project, &request, renderer, &mut |_| true).unwrap();
        assert_eq!(report.frames, 30);
        output
    };
    let with_effects = exported(&project, "temporal", &mut renderer);
    let without = exported(&plain, "temporal-plain", &mut renderer);
    for n in [3, 16, 28] {
        let decoded = |path: &std::path::Path, renderer: &mut Renderer| {
            let decoder = FfmpegVideoDecoder::open(path, DecoderOptions::default()).unwrap();
            drawn(renderer, decoder, frames(n))
        };
        let temporal = decoded(&with_effects, &mut renderer);
        let reference = decoded(&without, &mut renderer);
        let grab = frame(&plain, &bundle(), &timeline, frames(n), None, &mut renderer).unwrap();
        let codec = reference.psnr(&grab.image).unwrap();
        let psnr = temporal.psnr(&shown[n as usize]).unwrap();
        eprintln!(
            "{}: frame {n} exported against the preview {psnr:.1} dB (the codec alone {codec:.1} dB)",
            preset.name()
        );
        assert!(
            psnr > codec - 1.0,
            "frame {n}: {psnr:.1} dB, the codec {codec:.1} dB"
        );
    }
}
