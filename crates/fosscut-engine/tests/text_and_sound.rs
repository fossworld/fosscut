//! Project fonts and sound on generated media: font files are imported as font assets and
//! titles in their families render with them (ADR-0027); loudness normalization measures
//! a clip as it plays and finds the volume that reaches the target (ADR-0026, item 7).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{frames, project, renderer, workspace};
use fosscut_analysis::loudness::LoudnessMeter;
use fosscut_engine::frame::frame;
use fosscut_engine::loudness::{Normalization, normalize};
use fosscut_engine::preview::Sound;
use fosscut_model::{
    Asset, AssetKind, ClipContent, FontInfo, Id, IdKind, MediaInfo, PathHints, Project,
};
use fosscut_time::Flicks;

fn bundle() -> std::path::PathBuf {
    workspace().join("Show.fosscut")
}

fn title_font(project: &mut Project, font: Option<&str>, bold: bool) {
    let timeline = project.manifest.primary_timeline.clone();
    let doc = project.timelines.get_mut(&timeline).unwrap();
    let ClipContent::Text(text) = &mut doc.tracks[1].clips[0].content else {
        unreachable!()
    };
    text.style.font = font.map(String::from);
    text.style.bold = bold;
    text.style.size = 0.2;
}

#[test]
fn font_files_become_assets_that_titles_render_with() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let file = workspace().join("Fixture Sans.ttf");
    std::fs::write(&file, fosscut_render::text::fixture_font("Fixture Sans")).unwrap();
    let probed = fosscut_engine::import::probe(&file).unwrap();
    assert_eq!(probed.kind, AssetKind::Font);
    let info = probed.info.font.clone().unwrap();
    assert_eq!(
        info,
        FontInfo {
            families: vec!["Fixture Sans".into()],
            faces: 1
        }
    );
    let broken = workspace().join("broken.otf");
    std::fs::write(&broken, b"not a font").unwrap();
    let Err(error) = fosscut_engine::import::probe(&broken) else {
        panic!("text is not a font");
    };
    assert_eq!(error.code, "fosscut.media.probe_failed");

    let mut project = project("../clip.mp4");
    project.media.assets.push(Asset {
        folder: None,
        id: Id::new(IdKind::Asset, 8),
        kind: AssetKind::Font,
        name: "Fixture Sans.ttf".into(),
        path: PathHints {
            relative: Some("../Fixture Sans.ttf".into()),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            font: Some(info),
            ..MediaInfo::default()
        },
    });
    let timeline = project.manifest.primary_timeline.clone();
    let mut grab = |project: &Project| {
        frame(
            project,
            &bundle(),
            &timeline,
            frames(5),
            None,
            &mut renderer,
        )
        .unwrap()
        .image
    };
    title_font(&mut project, None, false);
    let regular = grab(&project);
    title_font(&mut project, None, true);
    let bold = grab(&project);
    // The fixture has only a regular face: bold text in it draws regular, where Inter
    // would draw it bold, so the frame shows the project font was used.
    title_font(&mut project, Some("Fixture Sans"), true);
    assert!(fosscut_model::validate(&project).is_empty());
    let fixture = grab(&project);
    assert_eq!(fixture.rgba, regular.rgba);
    assert_ne!(fixture.rgba, bold.rgba);
}

/// The loudness and sample peak of the clip's first second as it plays.
fn measured(project: &Project, timeline: &Id) -> (f64, f64) {
    let mut sound = Sound::new();
    let mixed = sound
        .mix(
            project,
            &bundle(),
            timeline,
            Flicks::ZERO,
            frames(30),
            48_000,
            2,
        )
        .unwrap();
    let mut meter = LoudnessMeter::new(2);
    meter.push(&mixed.samples);
    let found = meter.finish();
    (
        found.integrated_lufs.unwrap(),
        found.sample_peak_dbfs.unwrap(),
    )
}

/// Applies a normalization's commands to `project` directly, as the command framework
/// would: volumes, and the limiter as the clip's last effect.
fn apply(project: &mut Project, timeline: &Id, normalization: &Normalization) {
    let doc = project.timelines.get_mut(timeline).unwrap();
    let clip = &mut doc.tracks[0].clips[0];
    for command in &normalization.commands {
        let args = &command["args"];
        match command["command"].as_str().unwrap() {
            "clip.set" => clip.volume = args["set"]["volume"].as_f64().unwrap(),
            "effect.add" | "effect.set" => {
                clip.effects.retain(|e| e.effect_type != "audio.limiter");
                clip.effects.push(fosscut_model::Effect {
                    id: Id::new(IdKind::Effect, 40),
                    effect_type: "audio.limiter".into(),
                    version: 1,
                    enabled: true,
                    params: ["gain", "ceiling"]
                        .into_iter()
                        .map(|name| {
                            let value = args["params"][name].as_f64().unwrap();
                            (name.to_string(), fosscut_model::ParamValue::Number(value))
                        })
                        .collect(),
                    keyframes: Default::default(),
                });
            }
            "effect.remove" => clip.effects.retain(|e| e.effect_type != "audio.limiter"),
            other => panic!("unexpected {other}"),
        }
    }
}

#[test]
fn normalizing_reaches_the_target_with_a_volume_or_a_limiter() {
    if renderer().is_none() {
        return;
    }
    let mut project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let clip = Id::new(IdKind::Clip, 3);
    let title = Id::new(IdKind::Clip, 5);
    let one = std::slice::from_ref(&clip);
    // A quieter clip and its fade: both are measured away, at volume 1.
    {
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks[0].clips[0].volume = 0.25;
        doc.tracks[0].clips[0].audio.fade_in = frames(10);
    }
    let found = normalize(&project, &bundle(), one, -20.0, -1.0, true).unwrap();
    let leveled = &found.clips[0];
    let lufs = leveled.measured_lufs.unwrap();
    assert!((-30.0..-10.0).contains(&lufs), "{lufs}");
    assert!(!leveled.limited && leveled.limiter.is_none());
    assert!((leveled.gain_db - (-20.0 - lufs)).abs() < 0.01);
    assert_eq!(found.commands.len(), 1);
    assert_eq!(found.commands[0]["command"], "clip.set");
    // With the volume applied and no fade, the clip plays at the target.
    {
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks[0].clips[0].audio.fade_in = Flicks::ZERO;
    }
    apply(&mut project, &timeline, &found);
    let (now, _) = measured(&project, &timeline);
    assert!((now - -20.0).abs() < 0.3, "{now}");

    // A tone's peaks leave little room: without a limiter the ceiling holds it back.
    let peak = leveled.peak_dbfs.unwrap();
    let held = normalize(&project, &bundle(), one, -10.0, -12.0, false).unwrap();
    assert!(held.clips[0].limited && held.clips[0].limiter.is_none());
    assert!((held.clips[0].gain_db - (-12.0 - peak)).abs() < 0.02);

    // Titles have no sound; volume keyframes and odd targets are refused.
    assert!(normalize(&project, &bundle(), &[title], -14.0, -1.0, true).is_err());
    assert!(normalize(&project, &bundle(), one, 5.0, -1.0, true).is_err());
    assert!(normalize(&project, &bundle(), one, -14.0, 3.0, true).is_err());
    assert!(normalize(&project, &bundle(), one, -14.0, -20.0, true).is_err());
    assert!(normalize(&project, &bundle(), &[], -14.0, -1.0, true).is_err());
    let missing = [Id::new(IdKind::Clip, 99)];
    assert!(normalize(&project, &bundle(), &missing, -14.0, -1.0, true).is_err());
    let doc = project.timelines.get_mut(&timeline).unwrap();
    doc.tracks[0].clips[0].keyframes.insert(
        "volume".into(),
        vec![fosscut_model::Keyframe {
            time: Flicks::ZERO,
            value: fosscut_model::KeyValue::Number(1.0),
            interpolation: Default::default(),
        }],
    );
    let error = normalize(&project, &bundle(), one, -14.0, -1.0, true).unwrap_err();
    assert!(error.to_string().contains("keyframes"), "{error}");
}

/// Speech from the fixtures, alone on an audio track of a project next to the workspace.
fn speech_project() -> (Project, Id, Id) {
    let mut project = project("../clip.mp4");
    let speech = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/speech/librispeech-5105-28233-0005/audio.flac");
    let asset = Id::new(IdKind::Asset, 20);
    project.media.assets.push(Asset {
        folder: None,
        id: asset.clone(),
        kind: AssetKind::Audio,
        name: "speech.flac".into(),
        path: PathHints {
            relative: None,
            absolute: Some(speech.display().to_string()),
        },
        identity: None,
        info: MediaInfo {
            duration: Some(Flicks::from_millis(11_390)),
            audio: Some(fosscut_model::AudioInfo {
                sample_rate: 16_000,
                channels: 1,
                codec: None,
            }),
            ..MediaInfo::default()
        },
    });
    let timeline = project.manifest.primary_timeline.clone();
    let doc = project.timelines.get_mut(&timeline).unwrap();
    let clip = Id::new(IdKind::Clip, 21);
    doc.tracks = vec![fosscut_model::Track {
        id: Id::new(IdKind::Track, 22),
        kind: fosscut_model::TrackKind::Audio,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        caption_style: None,
        clips: vec![fosscut_model::Clip::new(
            clip.clone(),
            Flicks::ZERO,
            Flicks::from_seconds(10).unwrap(),
            ClipContent::Audio {
                asset,
                source_in: Flicks::ZERO,
            },
        )],
    }];
    (project, timeline, clip)
}

#[test]
fn a_limiter_lifts_speech_past_its_peaks_to_the_target() {
    if renderer().is_none() {
        return;
    }
    let (mut project, timeline, clip) = speech_project();
    let one = std::slice::from_ref(&clip);
    let loudness_now = |project: &Project| {
        let mut sound = Sound::new();
        let mixed = sound
            .mix(
                project,
                &bundle(),
                &timeline,
                Flicks::ZERO,
                Flicks::from_seconds(10).unwrap(),
                48_000,
                2,
            )
            .unwrap();
        let mut meter = LoudnessMeter::new(2);
        meter.push(&mixed.samples);
        let found = meter.finish();
        (
            found.integrated_lufs.unwrap(),
            found.sample_peak_dbfs.unwrap(),
        )
    };
    let (lufs, peak) = loudness_now(&project);
    // Speech peaks sit well above its loudness: a -14 LUFS target leaves no room.
    assert!(-14.0 - lufs > -1.0 - peak + 2.0, "{lufs} LUFS, {peak} dBFS");
    let held = normalize(&project, &bundle(), one, -14.0, -1.0, false).unwrap();
    assert!(held.clips[0].limited);
    let found = normalize(&project, &bundle(), one, -14.0, -1.0, true).unwrap();
    let leveled = &found.clips[0];
    assert!(!leveled.limited, "{leveled:?}");
    assert!(
        (leveled.reached_lufs.unwrap() - -14.0).abs() < 0.15,
        "{leveled:?}"
    );
    assert_eq!(leveled.volume, 1.0);
    apply(&mut project, &timeline, &found);
    let (now, peak_now) = loudness_now(&project);
    assert!((now - -14.0).abs() < 0.3, "{now}");
    assert!(peak_now <= -0.95, "{peak_now}");
    // Again: the clip's own limiter is adjusted, or removed when the volume suffices.
    let again = normalize(&project, &bundle(), one, -14.0, -1.0, true).unwrap();
    assert_eq!(again.commands[1]["command"], "effect.set");
    let quiet = normalize(&project, &bundle(), one, -30.0, -1.0, true).unwrap();
    assert_eq!(quiet.commands[1]["command"], "effect.remove");
}

#[test]
fn a_very_quiet_clip_is_lifted_past_the_volume_range_by_the_limiter() {
    if renderer().is_none() {
        return;
    }
    // A mono 440 Hz tone at about -38 LUFS on both channels: reaching -16 needs 22 dB, more
    // than the volume's 20, while its peaks leave plenty of room.
    let quiet = workspace().join("quiet.wav");
    if !quiet.exists() {
        let status = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("aevalsrc=0.0137*sin(2*PI*440*t):s=48000:d=2")
            .args(["-c:a", "pcm_s16le"])
            .arg(&quiet)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let (mut project, timeline, clip) = speech_project();
    {
        let asset = &mut project.media.assets[1];
        asset.path = PathHints {
            relative: None,
            absolute: Some(quiet.display().to_string()),
        };
        asset.info.duration = Some(Flicks::from_seconds(2).unwrap());
        asset.info.audio = Some(fosscut_model::AudioInfo {
            sample_rate: 48_000,
            channels: 1,
            codec: None,
        });
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks[0].clips[0].duration = Flicks::from_seconds(2).unwrap();
    }
    let one = std::slice::from_ref(&clip);
    let found = normalize(&project, &bundle(), one, -16.0, -1.0, true).unwrap();
    let leveled = &found.clips[0];
    let lufs = leveled.measured_lufs.unwrap();
    assert!((-40.0..-36.0).contains(&lufs), "{lufs}");
    assert!(-16.0 - lufs > 20.0, "more than the volume's range");
    assert!(!leveled.limited, "{leveled:?}");
    assert!(leveled.limiter.is_some(), "{leveled:?}");
    let reached = leveled.reached_lufs.unwrap();
    assert!((reached - -16.0).abs() < 0.5, "{reached}");
    // Without the limiter, the volume's range holds it back.
    let held = normalize(&project, &bundle(), one, -16.0, -1.0, false).unwrap();
    assert!(held.clips[0].limited, "{:?}", held.clips[0]);
    assert!((held.clips[0].gain_db - 20.0).abs() < 0.01);
}
