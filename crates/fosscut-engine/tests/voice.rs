//! Voiceovers and caption translation (ADR-0033 items 5 and 6): speech is written into the
//! bundle once and reused, and its calls import and place it; caption translations become
//! `captions.edit` calls that fill each page's translation or replace its words inside the
//! page; dubbing places one speech clip per page. The calls are applied to a project in a
//! core session, as a front end applies them. Tests that run models need `onnxruntime` and
//! the model in `FOSSCUT_MODELS_DIR` and skip without them; `FOSSCUT_REQUIRE_MODELS` turns a
//! missing model into a failure.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use fosscut_analysis::{models, onnx};
use fosscut_core::{Request, Session};
use fosscut_engine::captions::{
    self, TranslateMode, TranslateRequest, pages, spread_words, translation_calls,
};
use fosscut_engine::voice::{self, DubRequest, DubText, FOLDER, SpeakRequest};
use fosscut_model::{
    AssetKind, ClipContent, Id, IdKind, MemoryStore, Project, TimelineFormat, TrackKind,
};
use fosscut_time::{Flicks, Rate};
use serde_json::{Value, json};

fn required() -> bool {
    std::env::var_os("FOSSCUT_REQUIRE_MODELS").is_some_and(|v| !v.is_empty() && v != "0")
}

/// The models folder when ONNX Runtime and `model` are installed.
fn installed(model: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("FOSSCUT_MODELS_DIR").map(PathBuf::from);
    let manifest = models::manifest(model).unwrap();
    let ready = dir
        .as_ref()
        .is_some_and(|d| onnx::runtime_path(d).is_some() && models::is_installed(d, manifest));
    assert!(
        ready || !required(),
        "FOSSCUT_REQUIRE_MODELS is set but ONNX Runtime or {model} is not installed in FOSSCUT_MODELS_DIR"
    );
    ready.then(|| dir.unwrap())
}

fn ms(millis: i64) -> Flicks {
    Flicks::from_millis(millis)
}

fn at(millis: i64) -> Value {
    json!({ "flicks": ms(millis).get() })
}

/// A 1080p project with a caption track of the given pages: each page is its words with
/// start and end in milliseconds, starting a new page.
fn session(pages: &[&[(&str, i64, i64)]]) -> Session<MemoryStore> {
    let project = Project::new(
        Id::new(IdKind::Project, 7),
        "Voices",
        TimelineFormat {
            width: 1920,
            height: 1080,
            rate: Rate::FPS_30,
            sample_rate: 48_000,
        },
    );
    let mut session = Session::create(MemoryStore::default(), project).unwrap();
    let mut words = Vec::new();
    for page in pages {
        for (index, (text, start, end)) in page.iter().enumerate() {
            words.push(json!({
                "text": text, "start": at(*start), "end": at(*end), "new_page": index == 0,
            }));
        }
    }
    if !words.is_empty() {
        apply(
            &mut session,
            &[json!({ "command": "captions.add", "args": {
                "words": words,
                "paging": {
                    "max_words": 100, "max_lines": 10, "max_chars_per_line": 200,
                    "max_gap_ms": 100000, "linger_ms": 0,
                },
            } })],
            None,
        );
    }
    session
}

/// Applies `calls` in one request, as a front end does.
fn apply(session: &mut Session<MemoryStore>, calls: &[Value], revision: Option<u64>) {
    let mut request: Request = serde_json::from_value(json!({ "commands": calls })).unwrap();
    request.expected_revision = revision;
    request.actor = "test".into();
    session
        .execute(request)
        .unwrap_or_else(|e| panic!("{calls:?}: {e:?}"));
}

/// The caption pages of the project's caption track: (start, end, words with timeline
/// times, translation).
#[allow(clippy::type_complexity)]
fn caption_pages(
    project: &Project,
) -> Vec<(
    Flicks,
    Flicks,
    Vec<(String, Flicks, Flicks)>,
    Option<String>,
)> {
    let timeline = project.primary_timeline().unwrap();
    let track = timeline
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Caption)
        .unwrap();
    track
        .clips
        .iter()
        .map(|clip| {
            let ClipContent::Caption(caption) = &clip.content else {
                unreachable!()
            };
            let words = caption
                .words()
                .map(|w| (w.text.clone(), clip.start + w.start, clip.start + w.end))
                .collect();
            (clip.start, clip.end(), words, caption.translation.clone())
        })
        .collect()
}

/// Audio clips as (asset, start, duration).
fn audio_clips(project: &Project) -> Vec<(Id, Flicks, Flicks, Id)> {
    let timeline = project.primary_timeline().unwrap();
    timeline
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .flat_map(|t| {
            t.clips.iter().filter_map(|c| match &c.content {
                ClipContent::Audio { asset, .. } => {
                    Some((asset.clone(), c.start, c.duration, t.id.clone()))
                }
                _ => None,
            })
        })
        .collect()
}

/// A clip's length for media of `duration`: the whole frames it holds at 30 fps.
fn whole_frames(duration: Flicks) -> Flicks {
    let frame = Rate::FPS_30.frame_duration();
    Flicks::new(duration.get() / frame.get() * frame.get())
}

fn bundle(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-voice-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const HUNGARIAN: &[&[(&str, i64, i64)]] = &[
    &[("Köszönöm", 0, 500), ("szépen.", 500, 1200)],
    &[("Jó", 2000, 2300), ("reggelt!", 2300, 3000)],
];

#[test]
fn translations_fill_pages_or_replace_their_words_inside_them() {
    let mut session = session(HUNGARIAN);
    let (_, _, found) = pages(session.project(), None, None).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].text, "Köszönöm szépen.");
    assert_eq!((found[1].start, found[1].end), (ms(2000), ms(3000)));
    let english = [
        "Thank you very much.".to_string(),
        "Good morning!".to_string(),
    ];

    // Bilingual: each page keeps its words and gains a translation.
    let (calls, skipped) = translation_calls(&found, &english, TranslateMode::Bilingual);
    assert!(skipped.is_empty());
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["command"], "captions.edit");
    let revision = session.revision();
    apply(&mut session, &calls, Some(revision));
    let after = caption_pages(session.project());
    assert_eq!(after[0].3.as_deref(), Some("Thank you very much."));
    assert_eq!(after[1].3.as_deref(), Some("Good morning!"));
    assert_eq!(after[0].2[0].0, "Köszönöm");

    // Replace: the translated words share the page's time by length, page edges kept.
    let (calls, skipped) = translation_calls(
        &found,
        &["Thank you very much.".into(), "  ".into()],
        TranslateMode::Replace,
    );
    assert_eq!(
        skipped,
        [found[1].clip.clone()],
        "an empty translation is skipped"
    );
    assert_eq!(calls.len(), 1);
    let (calls, _) = translation_calls(&found, &english, TranslateMode::Replace);
    let revision = session.revision();
    apply(&mut session, &calls, Some(revision));
    let after = caption_pages(session.project());
    for ((start, end, words, _), (page, text)) in after.iter().zip(found.iter().zip(&english)) {
        assert_eq!((*start, *end), (page.start, page.end), "page edges kept");
        let shown: Vec<&str> = words.iter().map(|w| w.0.as_str()).collect();
        assert_eq!(shown.join(" "), *text);
        assert_eq!(words[0].1, *start);
        assert_eq!(words.last().unwrap().2, *end);
        for pair in words.windows(2) {
            assert_eq!(pair[0].2, pair[1].1, "back to back");
        }
        for (word, from, to) in words {
            assert!(
                start <= from && from <= to && to <= end,
                "{word} inside its page"
            );
        }
    }
    // "Thank you very much." over 1.2 s: 17 letters, "very" gets 4 of them.
    let very = &after[0].2[2];
    assert_eq!(very.0, "very");
    assert_eq!(very.2 - very.1, Flicks::new(ms(1200).get() * 4 / 17));
    assert_eq!(after[0].3.as_deref(), Some("Thank you very much."));

    let spread = spread_words("a bb", ms(0), ms(300));
    assert_eq!(spread[1]["start"], at(100));
    assert_eq!(spread[1]["end"], at(300));
    assert!(spread_words("", ms(0), ms(300)).is_empty());
    assert_eq!(
        TranslateMode::named("replace").unwrap(),
        TranslateMode::Replace
    );
    assert!(TranslateMode::named("both").is_err());
}

#[test]
fn caption_requests_fail_with_typed_errors() {
    let session = session(HUNGARIAN);
    let request = |pair: &str| TranslateRequest {
        models: std::env::temp_dir().join("fosscut-no-models"),
        pair: pair.into(),
        ..TranslateRequest::default()
    };
    let error = captions::translate(session.project(), &request("en>de")).unwrap_err();
    assert_eq!(error.code(), "fosscut.translate.unsupported_pair");
    assert!(error.to_string().contains("hu>en"), "{error}");
    let error = captions::translate(session.project(), &request("hu>en")).unwrap_err();
    assert_eq!(error.code(), "fosscut.model.not_installed");
    let bare = self::session(&[]);
    let error = captions::translate(bare.project(), &request("hu>en")).unwrap_err();
    assert_eq!(error.code(), "fosscut.captions.none");
    let timeline = session.project().manifest.primary_timeline.clone();
    let error = pages(
        session.project(),
        Some(&timeline),
        Some(&Id::new(IdKind::Track, 999)),
    )
    .unwrap_err();
    assert_eq!(error.code(), "fosscut.request.invalid_arguments");
    let error = voice::dub(
        session.project(),
        &DubRequest {
            language: Some("hu".into()),
            ..DubRequest::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.code(), "fosscut.voice.language_unavailable");
    let error = voice::speak(
        session.project(),
        &SpeakRequest {
            text: "Hello".into(),
            voice: Some("zz_nobody".into()),
            ..SpeakRequest::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.code(), "fosscut.voice.unknown_voice");
}

#[test]
fn caption_tracks_translate_with_the_model() {
    let Some(models) = installed("opus-mt-hu-en") else {
        return;
    };
    let mut session = session(HUNGARIAN);
    let request = TranslateRequest {
        models: models.clone(),
        pair: "hu>en".into(),
        mode: TranslateMode::Bilingual,
        ..TranslateRequest::default()
    };
    let translated = captions::translate(session.project(), &request).unwrap();
    assert_eq!(translated.model, "opus-mt-hu-en");
    assert!(
        translated
            .attribution
            .as_deref()
            .unwrap()
            .contains("Helsinki")
    );
    eprintln!("{:?}", translated.pages);
    assert!(
        translated.pages[0]
            .translation
            .to_lowercase()
            .contains("thank")
    );
    assert!(
        translated.pages[1]
            .translation
            .to_lowercase()
            .contains("morning")
    );
    apply(&mut session, &translated.calls, Some(translated.revision));
    let after = caption_pages(session.project());
    assert!(
        after[0]
            .3
            .as_deref()
            .unwrap()
            .to_lowercase()
            .contains("thank")
    );

    let replace = TranslateRequest {
        mode: TranslateMode::Replace,
        ..request
    };
    let translated = captions::translate(session.project(), &replace).unwrap();
    apply(&mut session, &translated.calls, Some(translated.revision));
    let after = caption_pages(session.project());
    let words: Vec<&str> = after[0].2.iter().map(|w| w.0.as_str()).collect();
    assert_eq!(words.join(" "), translated.pages[0].translation);
    assert_eq!((after[1].0, after[1].1), (ms(2000), ms(3000)));
}

#[test]
fn speech_is_written_once_reused_and_placed() {
    let Some(models) = installed("kokoro-82m") else {
        return;
    };
    let bundle = bundle("speak");
    let mut session = session(&[]);
    let request = SpeakRequest {
        bundle: bundle.clone(),
        models: models.clone(),
        text: "Hello there, this is a voiceover made by FossCut.".into(),
        start: Some(json!({ "seconds": "1" })),
        ..SpeakRequest::default()
    };
    let spoken = voice::speak(session.project(), &request).unwrap();
    assert!(!spoken.file.reused);
    assert!(spoken.file.relative.starts_with(FOLDER));
    let file = bundle.join(&spoken.file.relative);
    assert!(file.is_file());
    let written = std::fs::metadata(&file).unwrap().modified().unwrap();
    assert!(
        (1.5..8.0).contains(&spoken.file.seconds),
        "{}",
        spoken.file.seconds
    );
    let commands: Vec<&str> = spoken
        .calls
        .iter()
        .map(|c| c["command"].as_str().unwrap())
        .collect();
    assert_eq!(commands, ["media.add", "clip.add"]);
    assert_eq!(spoken.calls[1]["args"]["free_track"], true);

    // The same request finds the same file and synthesizes nothing.
    let again = voice::speak(session.project(), &request).unwrap();
    assert!(again.file.reused);
    assert_eq!(again.file.relative, spoken.file.relative);
    assert_eq!(again.file.media_add, spoken.file.media_add);
    assert_eq!(again.calls, spoken.calls);
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        written
    );

    apply(&mut session, &spoken.calls, Some(spoken.revision));
    let project = session.project();
    let asset = project.media.get(&spoken.asset).unwrap();
    assert_eq!(asset.kind, AssetKind::Audio);
    assert_eq!(asset.name, "Speech: Hello there, this is a voiceover...");
    assert_eq!(asset.info.duration, Some(spoken.file.duration));
    assert_eq!(
        audio_clips(project)
            .iter()
            .map(|c| (c.0.clone(), c.1, c.2))
            .collect::<Vec<_>>(),
        [(
            spoken.asset.clone(),
            ms(1000),
            whole_frames(spoken.file.duration)
        )]
    );

    // Once imported, the asset is placed again without a second import, on a free track.
    let third = voice::speak(session.project(), &request).unwrap();
    assert_eq!(third.asset, spoken.asset);
    assert_eq!(third.calls.len(), 1);
    apply(&mut session, &third.calls, Some(third.revision));
    let clips = audio_clips(session.project());
    assert_eq!(clips.len(), 2);
    assert_ne!(
        clips[0].3, clips[1].3,
        "the overlapping copy is on another track"
    );
    std::fs::remove_dir_all(&bundle).unwrap();
}

#[test]
fn caption_pages_are_dubbed_at_their_starts() {
    let Some(models) = installed("kokoro-82m") else {
        return;
    };
    let bundle = bundle("dub");
    let mut session = session(&[
        &[
            ("Thank", 0, 300),
            ("you", 300, 500),
            ("very", 500, 800),
            ("much.", 800, 1200),
        ],
        &[
            ("Good", 2000, 2100),
            ("morning,", 2100, 2300),
            ("everyone,", 2300, 2500),
            ("and", 2500, 2550),
            ("welcome", 2550, 2650),
            ("to", 2650, 2700),
            ("this", 2700, 2750),
            ("rather", 2750, 2780),
            ("long", 2780, 2790),
            ("page.", 2790, 2800),
        ],
    ]);
    let request = DubRequest {
        bundle: bundle.clone(),
        models: models.clone(),
        ..DubRequest::default()
    };
    let dubbed = voice::dub(session.project(), &request).unwrap();
    eprintln!("{:#?}", dubbed.pages);
    assert_eq!(dubbed.pages.len(), 2);
    let (first, second) = (&dubbed.pages[0], &dubbed.pages[1]);
    assert_eq!(first.start, ms(0));
    assert_eq!(first.room, ms(2000), "until the next page");
    assert_eq!(first.speed, 1.0);
    assert!(first.warning.is_none());
    assert_eq!(second.room, ms(800), "the last page's own length");
    assert_eq!(second.speed, 1.25, "spoken as fast as allowed");
    assert!(second.speech > second.room);
    assert!(second.warning.as_deref().unwrap().contains("runs on"));
    let commands: Vec<&str> = dubbed
        .calls
        .iter()
        .map(|c| c["command"].as_str().unwrap())
        .collect();
    assert_eq!(commands, ["media.add", "media.add", "clip.add", "clip.add"]);
    apply(&mut session, &dubbed.calls, Some(dubbed.revision));
    let clips = audio_clips(session.project());
    assert_eq!(
        clips.iter().map(|c| (c.0.clone(), c.1)).collect::<Vec<_>>(),
        [
            (first.asset.clone(), ms(0)),
            (second.asset.clone(), ms(2000))
        ]
    );
    assert_eq!(clips[1].2, whole_frames(second.speech), "never cut short");

    // Again: every file is reused; translations are missing, so nothing is spoken.
    let again = voice::dub(session.project(), &request).unwrap();
    assert!(again.pages.iter().all(|p| p.reused));
    assert_eq!(again.calls.len(), 2, "already imported: placing only");
    let translations = voice::dub(
        session.project(),
        &DubRequest {
            text: DubText::Translation,
            ..request
        },
    )
    .unwrap();
    assert!(translations.pages.is_empty() && translations.calls.is_empty());
    assert_eq!(translations.skipped.len(), 2);
    std::fs::remove_dir_all(&bundle).unwrap();
}
