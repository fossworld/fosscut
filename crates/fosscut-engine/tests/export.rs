//! Export jobs end to end on generated media: every available preset, verification,
//! cancellation, the overwrite guard, request-id replay and typed errors (ADR-0006,
//! ADR-0009).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::PathBuf;
use std::sync::Mutex;

use common::{frames, project, renderer, workspace};
use fosscut_engine::{EngineError, ExportRequest, Preset, compare, export, presets};
use fosscut_media::{AudioCodec, AudioReport, AudioSettings, Container, OutputReport, VideoCodec};
use fosscut_model::{Id, IdKind, Project, TimelineFormat};
use fosscut_render::Renderer;
use fosscut_time::{Flicks, Rate};

fn request(project: &Project, preset: Preset, name: &str) -> ExportRequest {
    let timeline = project.manifest.primary_timeline.clone();
    ExportRequest {
        bundle: workspace().join("Show.fosscut"),
        settings: preset.settings(&project.timelines[&timeline].format),
        timeline,
        output: workspace().join(name),
        allow_overwrite: false,
        expected_revision: None,
        request_id: None,
        cache: workspace().join("cache"),
        options: Default::default(),
    }
}

fn run(
    project: &Project,
    request: &ExportRequest,
    renderer: &mut Renderer,
) -> Result<fosscut_engine::ExportReport, EngineError> {
    export(project, request, renderer, &mut |_| true)
}

/// One test at a time touches the shared output folder.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn every_available_preset_exports_and_verifies() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let _serial = serial();
    let project = project("../clip.mp4");
    let available: Vec<Preset> = Preset::ALL.into_iter().filter(|p| p.available()).collect();
    assert!(
        available.contains(&Preset::M4aAac),
        "AAC is always available"
    );
    assert!(Preset::default_video().is_some());
    for preset in available {
        let name = format!("every-{}.{}", preset.name(), preset.extension());
        let mut seen = Vec::new();
        let report = export(
            &project,
            &request(&project, preset, &name),
            &mut renderer,
            &mut |p| {
                seen.push(p);
                true
            },
        )
        .unwrap();
        let what = preset.name();
        assert!(!report.replayed);
        assert_eq!(report.duration, Flicks::from_seconds(1).unwrap(), "{what}");
        if report.settings.video.is_none() {
            assert_eq!(report.frames, 0);
            assert!(report.verified.video.is_none(), "{what}: audio only");
        } else {
            assert_eq!(report.frames, 30, "{what}");
            assert_eq!(
                report.verified.video.as_ref().unwrap().frame_count,
                30,
                "{what}"
            );
            assert!(report.encoders.video.is_some());
            assert_eq!(
                seen.last().map(|p| (p.done, p.total)),
                Some((30, 30)),
                "{what}: progress"
            );
        }
        assert_eq!(
            report.verified.audio.is_some(),
            report.settings.audio.is_some(),
            "{what}: audio from the clip, except in GIFs"
        );
        assert!(workspace().join(&name).exists());
        assert!(
            !workspace().join(format!(".{name}.partial")).exists(),
            "{what}: no partial file left"
        );
    }
    let listed = presets();
    assert_eq!(listed.len(), Preset::ALL.len());
    assert!(listed.iter().any(|p| p.name == "mp4-h264"));
    assert_eq!(Preset::parse("mov-prores"), Some(Preset::MovProRes));
    assert_eq!(Preset::parse("gif"), Some(Preset::Gif));
    assert_eq!(Preset::parse("avi"), None);
}

#[test]
fn cancelling_leaves_nothing_behind() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let _serial = serial();
    let project = project("../clip.mp4");
    let preset = Preset::default_video().unwrap();
    let request = request(
        &project,
        preset,
        &format!("cancelled.{}", preset.extension()),
    );
    let error = export(&project, &request, &mut renderer, &mut |p| p.done < 5).unwrap_err();
    assert_eq!(error.code(), "fosscut.export.cancelled");
    assert!(!request.output.exists());
    assert!(
        !workspace()
            .join(format!(".cancelled.{}.partial", preset.extension()))
            .exists()
    );
}

#[test]
fn existing_outputs_need_permission_and_request_ids_replay() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let _serial = serial();
    let project = project("../clip.mp4");
    let preset = Preset::default_video().unwrap();
    let mut request = request(&project, preset, &format!("guarded.{}", preset.extension()));
    let _ = std::fs::remove_file(&request.output);
    run(&project, &request, &mut renderer).unwrap();
    let error = run(&project, &request, &mut renderer).unwrap_err();
    assert_eq!(error.code(), "fosscut.guard.destructive");
    assert!(error.hint().unwrap().contains("allow_destructive"));
    request.allow_overwrite = true;
    run(&project, &request, &mut renderer).unwrap();

    request.request_id = Some("render 1/once".into());
    let first = run(&project, &request, &mut renderer).unwrap();
    let again = run(&project, &request, &mut renderer).unwrap();
    assert!(again.replayed && !first.replayed);
    assert_eq!(again.verified, first.verified);
    // Records hold absolute paths, so they live in the cache, never in the bundle.
    let records = request
        .cache
        .join("jobs")
        .join(project.manifest.id.to_string());
    assert_eq!(std::fs::read_dir(&records).unwrap().count(), 1);
    assert!(!request.bundle.join("cache").exists());
    let mut other = request.clone();
    other.output = workspace().join(format!("elsewhere.{}", preset.extension()));
    assert_eq!(
        run(&project, &other, &mut renderer).unwrap_err().code(),
        "fosscut.request.reused"
    );
    // An id that differs only in characters a file name cannot hold is another request.
    other.request_id = Some("render 1_once".into());
    assert!(!run(&project, &other, &mut renderer).unwrap().replayed);
    assert_eq!(std::fs::read_dir(&records).unwrap().count(), 2);
}

#[test]
fn dry_runs_predict_the_export_and_stale_revisions_fail() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let _serial = serial();
    let project = project("../clip.mp4");
    let preset = Preset::default_video().unwrap();
    let mut request = request(&project, preset, &format!("planned.{}", preset.extension()));
    let planned = fosscut_engine::plan(&project, &request).unwrap();
    assert!(!request.output.exists(), "a dry run writes nothing");
    let report = run(&project, &request, &mut renderer).unwrap();
    assert_eq!(planned.output, report.output);
    assert_eq!(planned.settings, report.settings);
    assert_eq!(planned.frames, report.frames);
    assert_eq!(planned.duration, report.duration);
    assert_eq!(planned.encoders, report.encoders);
    // A dry run applies the same guards as the export.
    assert_eq!(
        fosscut_engine::plan(&project, &request).unwrap_err().code(),
        "fosscut.guard.destructive"
    );

    request.allow_overwrite = true;
    let current = project.manifest.revision;
    request.expected_revision = Some(current + 1);
    let stale = fosscut_engine::plan(&project, &request).unwrap_err();
    assert_eq!(
        stale,
        EngineError::RevisionConflict {
            expected: current + 1,
            current
        }
    );
    let modified = std::fs::metadata(&request.output)
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        run(&project, &request, &mut renderer).unwrap_err().code(),
        "fosscut.revision.conflict"
    );
    assert_eq!(
        std::fs::metadata(&request.output)
            .unwrap()
            .modified()
            .unwrap(),
        modified,
        "a refused export leaves the file alone"
    );
    request.expected_revision = Some(current);
    assert_eq!(
        fosscut_engine::plan(&project, &request).unwrap().frames,
        report.frames
    );
}

#[test]
fn an_output_that_cannot_be_replaced_leaves_nothing_behind() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let _serial = serial();
    let project = project("../clip.mp4");
    let preset = Preset::default_video().unwrap();
    let mut request = request(&project, preset, &format!("taken.{}", preset.extension()));
    request.allow_overwrite = true;
    let partial = workspace().join(format!(".taken.{}.partial", preset.extension()));

    // A folder at the output path is refused before anything renders.
    std::fs::create_dir_all(request.output.join("inside")).unwrap();
    let folder = fosscut_engine::plan(&project, &request).unwrap_err();
    assert_eq!(folder.code(), "fosscut.export.invalid_settings");
    std::fs::remove_dir_all(&request.output).unwrap();

    // Something takes the path while the export runs, so the finished file cannot be moved
    // into place: the export fails with a typed error and removes its temporary file.
    let output = request.output.clone();
    let error = export(&project, &request, &mut renderer, &mut |progress| {
        if progress.done == progress.total {
            std::fs::create_dir_all(output.join("inside")).unwrap();
        }
        true
    })
    .unwrap_err();
    assert_eq!(error.code(), "fosscut.media.write_failed");
    assert!(!partial.exists(), "the temporary file is removed");
    assert!(
        request.output.join("inside").is_dir(),
        "the folder is untouched"
    );
    std::fs::remove_dir_all(&request.output).unwrap();
}

#[test]
fn impossible_exports_are_typed_errors() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let _serial = serial();
    let missing = project("../not-there.mp4");
    let preset = Preset::default_video().unwrap();
    let error = run(
        &missing,
        &request(&missing, preset, "missing.mp4"),
        &mut renderer,
    )
    .unwrap_err();
    assert_eq!(error.code(), "fosscut.export.missing_media");
    assert!(
        error.details()["tried"][0]
            .as_str()
            .unwrap()
            .ends_with("not-there.mp4")
    );
    assert!(error.hint().is_some());

    let project = project("../clip.mp4");
    let mut empty = project.clone();
    let timeline = empty.manifest.primary_timeline.clone();
    empty.timelines.get_mut(&timeline).unwrap().tracks.clear();
    let error = run(&empty, &request(&empty, preset, "empty.mp4"), &mut renderer).unwrap_err();
    assert_eq!(error.code(), "fosscut.export.empty_timeline");

    let mut unknown = request(&project, preset, "unknown.mp4");
    unknown.timeline = Id::new(IdKind::Timeline, 99);
    assert_eq!(
        run(&project, &unknown, &mut renderer).unwrap_err().code(),
        "fosscut.timeline.not_found"
    );

    let mut odd = request(&project, preset, "odd.mp4");
    if let Some(video) = &mut odd.settings.video {
        video.width = 321;
    }
    assert_eq!(
        run(&project, &odd, &mut renderer).unwrap_err().code(),
        "fosscut.export.invalid_settings"
    );

    let mut wrong = request(&project, Preset::WebmVp9, "wrong.webm");
    wrong.settings.audio = Some(AudioSettings::stereo(AudioCodec::Aac));
    assert_eq!(
        run(&project, &wrong, &mut renderer).unwrap_err().code(),
        "fosscut.export.invalid_settings"
    );

    let mut nowhere = request(&project, preset, "x.mp4");
    nowhere.output = PathBuf::from("/no/such/dir/x.mp4");
    assert_eq!(
        run(&project, &nowhere, &mut renderer).unwrap_err().code(),
        "fosscut.export.invalid_settings"
    );

    // A codec this machine cannot encode is refused with the alternatives (on CI runners
    // without hardware encoders this is H.264, the round 1 regression).
    for codec in VideoCodec::ALL {
        let preset = Preset::ALL.into_iter().find(|p| {
            p.settings(&project.timelines[&timeline].format)
                .video
                .map(|v| v.codec)
                == Some(codec)
        });
        if let Some(preset) = preset.filter(|p| !p.available()) {
            let error = run(
                &project,
                &request(&project, preset, "unavailable.mp4"),
                &mut renderer,
            )
            .unwrap_err();
            assert_eq!(error.code(), "fosscut.export.codec_unavailable");
            assert!(!error.details()["available"].as_array().unwrap().is_empty());
            assert!(error.hint().unwrap().contains("Choose one"));
        }
    }
}

#[test]
fn comparison_names_every_difference() {
    let format = TimelineFormat {
        width: 320,
        height: 180,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let settings = Preset::Mp4Av1.settings(&format);
    let good = OutputReport {
        container: "mov,mp4,m4a,3gp,3g2,mj2".into(),
        brand: Some("isom".into()),
        size_bytes: 1,
        duration: Some(frames(30)),
        video: Some(fosscut_media::VideoReport {
            codec: "av1".into(),
            profile: None,
            width: 320,
            height: 180,
            rate: Some(Rate::FPS_30),
            frame_count: 30,
            duration: Some(frames(30)),
            pixel_format: "yuv420p".into(),
            color: fosscut_media::ColorInfo::default(),
        }),
        audio: Some(AudioReport {
            codec: "aac".into(),
            sample_rate: 48_000,
            channels: 2,
            duration: None,
        }),
    };
    assert!(compare(&settings, 30, frames(30), &good).is_empty());

    let mut bad = good.clone();
    bad.brand = Some("qt  ".into());
    if let Some(video) = &mut bad.video {
        video.codec = "hevc".into();
        video.width = 640;
        video.rate = Some(Rate::FPS_25);
        video.frame_count = 29;
        video.duration = Some(frames(29));
    }
    if let Some(audio) = &mut bad.audio {
        audio.codec = "opus".into();
        audio.channels = 1;
    }
    let mismatches = compare(&settings, 30, frames(30), &bad);
    for expected in [
        "container",
        "video codec",
        "size",
        "frame rate",
        "frames",
        "video duration",
        "audio codec",
        "audio format",
    ] {
        assert!(
            mismatches.iter().any(|m| m.starts_with(expected)),
            "{expected} in {mismatches:?}"
        );
    }
    let mut silent = good.clone();
    silent.audio = None;
    silent.video = None;
    let mismatches = compare(&settings, 30, frames(30), &silent);
    assert!(mismatches.iter().any(|m| m.starts_with("video: requested")));
    assert!(mismatches.iter().any(|m| m.starts_with("audio: requested")));
    let audio_only = Preset::M4aAac.settings(&format);
    assert!(
        compare(&audio_only, 0, frames(30), &good)
            .iter()
            .any(|m| m.starts_with("video: not requested"))
    );
    let mov = fosscut_media::OutputSettings {
        container: Container::Mov,
        ..settings.clone()
    };
    assert!(
        compare(&mov, 30, frames(30), &good)
            .iter()
            .any(|m| m.starts_with("container"))
    );
    let webm = fosscut_media::OutputSettings {
        container: Container::Webm,
        ..settings
    };
    assert!(
        compare(&webm, 30, frames(30), &good)
            .iter()
            .any(|m| m.starts_with("container"))
    );
}

#[test]
fn gif_timing_is_compared_to_the_centisecond() {
    let format = TimelineFormat {
        width: 160,
        height: 90,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let settings = Preset::Gif.settings(&format);
    // 31 frames at 30 fps are stored as 103 centiseconds.
    let stored = Flicks::from_millis(1_030);
    let report = OutputReport {
        container: "gif".into(),
        brand: None,
        size_bytes: 1,
        duration: Some(stored),
        video: Some(fosscut_media::VideoReport {
            codec: "gif".into(),
            profile: None,
            width: 160,
            height: 90,
            rate: Rate::new(3_100, 103).ok(),
            frame_count: 31,
            duration: Some(stored),
            pixel_format: "bgra".into(),
            color: fosscut_media::ColorInfo::default(),
        }),
        audio: None,
    };
    assert!(compare(&settings, 31, frames(31), &report).is_empty());
    let mut late = report.clone();
    if let Some(video) = &mut late.video {
        video.duration = Some(Flicks::from_millis(1_100));
        video.frame_count = 30;
    }
    let mismatches = compare(&settings, 31, frames(31), &late);
    assert!(mismatches.iter().any(|m| m.starts_with("video duration")));
    assert!(mismatches.iter().any(|m| m.starts_with("frames")));
    let wav = Preset::Wav.settings(&format);
    let mut sound = report.clone();
    sound.container = "wav".into();
    sound.video = None;
    sound.audio = Some(AudioReport {
        codec: "pcm_s16le".into(),
        sample_rate: 48_000,
        channels: 2,
        duration: None,
    });
    assert!(compare(&wav, 0, frames(31), &sound).is_empty());
    sound.container = "flac".into();
    assert!(
        compare(&wav, 0, frames(31), &sound)
            .iter()
            .any(|m| m.starts_with("container"))
    );
}

#[test]
fn export_choices_parse_and_shape_the_settings() {
    use fosscut_engine::{SettingsChoice, bit_rate, frame_rate, resolution, settings_for};
    assert_eq!(resolution("4K").unwrap(), 2160);
    assert_eq!(resolution("720p").unwrap(), 720);
    assert_eq!(resolution("540").unwrap(), 540);
    assert!(resolution("huge").is_err() && resolution("2").is_err());
    assert_eq!(
        frame_rate("29.97").unwrap(),
        Rate::new(30_000, 1001).unwrap()
    );
    assert_eq!(frame_rate("25").unwrap(), Rate::new(25, 1).unwrap());
    assert!(frame_rate("fast").is_err());
    assert_eq!(bit_rate("8M").unwrap(), 8_000_000);
    assert_eq!(bit_rate("192k").unwrap(), 192_000);
    assert!(bit_rate("0").is_err() && bit_rate("lots").is_err());
    let tall = TimelineFormat {
        width: 1080,
        height: 1920,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let choice = SettingsChoice {
        resolution: Some(720),
        rate: Some(Rate::new(24, 1).unwrap()),
        video_bit_rate: Some(4_000_000),
        audio_bit_rate: Some(128_000),
    };
    let settings = settings_for(Preset::Mp4H264, &tall, &choice);
    let video = settings.video.unwrap();
    assert_eq!(
        (video.width, video.height),
        (720, 1280),
        "the shorter side is 720"
    );
    assert_eq!(video.rate, Rate::new(24, 1).unwrap());
    assert_eq!(video.bit_rate, Some(4_000_000));
    assert_eq!(settings.audio.unwrap().bit_rate, 128_000);
    // GIFs are small by default, and never grow past the canvas.
    let gif = settings_for(Preset::Gif, &tall, &SettingsChoice::default())
        .video
        .unwrap();
    assert_eq!(
        (gif.width, gif.height, gif.rate),
        (480, 854, Rate::new(15, 1).unwrap())
    );
}

#[test]
fn exports_take_a_range_a_size_a_cover_and_subtitles() {
    use fosscut_engine::{ExportOptions, SettingsChoice, SubtitleFormat, settings_for};
    use fosscut_model::{
        CaptionContent, CaptionLine, CaptionWord, Clip, ClipContent, Track, TrackKind,
    };
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let word = |text: &str, from: i64, to: i64| CaptionWord {
        text: text.into(),
        start: Flicks::from_millis(from),
        end: Flicks::from_millis(to),
        emphasis: false,
    };
    // A caption page from 0.2 s to 0.8 s of the timeline.
    let page = Clip::new(
        Id::new(IdKind::Clip, 30),
        frames(6),
        frames(18),
        ClipContent::Caption(CaptionContent {
            lines: vec![CaptionLine {
                words: vec![word("Hello", 0, 300), word("there", 300, 600)],
            }],
            translation: None,
        }),
    );
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 31),
            kind: TrackKind::Caption,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![page],
        });
    assert!(fosscut_model::validate(&project).is_empty());
    let format = project.timelines[&timeline].format.clone();
    // The most compatible video preset this machine has (CI's Linux has no H.264).
    let preset = Preset::default_video().unwrap();
    let name = format!("part.{}", preset.extension());
    let mut request = request(&project, preset, &name);
    request.settings = settings_for(
        preset,
        &format,
        &SettingsChoice {
            resolution: Some(90),
            rate: Some(Rate::new(15, 1).unwrap()),
            ..SettingsChoice::default()
        },
    );
    request.options = ExportOptions {
        range: Some((frames(9), frames(27))),
        cover: Some(frames(3)),
        subtitles: Some(SubtitleFormat::Vtt),
    };
    let report = run(&project, &request, &mut renderer).unwrap();
    // 0.6 s at 15 fps, 160x90.
    assert_eq!(report.frames, 9);
    assert_eq!(report.duration, Flicks::from_millis(600));
    let video = report.verified.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (160, 90));
    let cover = std::path::PathBuf::from(report.cover.clone().unwrap());
    assert_eq!(cover, workspace().join("part.cover.png"));
    assert!(std::fs::read(&cover).unwrap().starts_with(b"\x89PNG"));
    let subtitles = std::fs::read_to_string(report.subtitles.clone().unwrap()).unwrap();
    assert!(subtitles.starts_with("WEBVTT"), "{subtitles}");
    // The page shows from 0.2 s; the export starts at 0.3 s, so it runs from 0 to 0.5 s.
    assert!(
        subtitles.contains("00:00:00.000 --> 00:00:00.500\nHello there"),
        "{subtitles}"
    );
    assert_eq!(report.rights.len(), 1);
    assert_eq!(report.rights[0].origin, "yours");
    // Ranges and covers outside the export are refused before anything renders.
    for options in [
        ExportOptions {
            range: Some((frames(40), frames(50))),
            ..ExportOptions::default()
        },
        ExportOptions {
            range: Some((frames(10), frames(5))),
            ..ExportOptions::default()
        },
        ExportOptions {
            cover: Some(frames(40)),
            ..ExportOptions::default()
        },
    ] {
        let mut bad = self::request(&project, Preset::Mp4H264, "bad.mp4");
        bad.options = options;
        assert!(matches!(
            fosscut_engine::plan(&project, &bad),
            Err(EngineError::InvalidSettings { .. })
        ));
    }
    assert!(SubtitleFormat::named("ass").is_err());
}
