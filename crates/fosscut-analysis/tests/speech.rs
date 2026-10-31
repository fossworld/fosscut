//! Speech recognition on a recording with a reference word alignment (ADR-0020, Validation):
//! the words, their start times, the pauses between sentences, progress and cancellation.
//! It needs FFmpeg and `whisper-tiny.en` installed in `FOSSCUT_MODELS_DIR`; CI installs the
//! model and sets `FOSSCUT_REQUIRE_SPEECH`, which turns a missing model into a failure.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use fosscut_analysis::decode::{Media, analyze, speech_model};
use fosscut_analysis::models::{self, ModelError};
use fosscut_analysis::{AnalysisError, Analyzer, Options, Progress, Transcript};
use fosscut_time::{FLICKS_PER_SECOND, Flicks};
use serde_json::Value;

fn required(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty() && v != "0")
}

/// The models folder when FFmpeg and `whisper-tiny.en` are available.
fn models_with_tiny() -> Option<PathBuf> {
    let ffmpeg = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    let dir = std::env::var_os("FOSSCUT_MODELS_DIR").map(PathBuf::from);
    let tiny = models::manifest("whisper-tiny.en").unwrap();
    let installed = dir.as_ref().is_some_and(|d| models::is_installed(d, tiny));
    assert!(
        (ffmpeg && installed) || !required("FOSSCUT_REQUIRE_SPEECH"),
        "FOSSCUT_REQUIRE_SPEECH is set but ffmpeg or whisper-tiny.en in FOSSCUT_MODELS_DIR is missing"
    );
    (ffmpeg && installed).then(|| dir.unwrap())
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/speech/librispeech-5105-28233-0005")
}

fn media(path: &Path) -> Media<'_> {
    Media {
        path,
        // 11.39 s.
        duration: Flicks::new(1139 * (FLICKS_PER_SECOND / 100)),
        audio_channels: Some(1),
        video_rate: None,
    }
}

fn tiny() -> Options {
    let mut options = Options::default();
    options.transcript.model = Some("whisper-tiny.en".into());
    options
}

fn seconds(moment: &fosscut_analysis::Moment) -> f64 {
    moment.flicks.get() as f64 / FLICKS_PER_SECOND as f64
}

/// The reference words as (lowercase word, start, end) in seconds.
fn reference() -> Vec<(String, f64, f64)> {
    let text = std::fs::read_to_string(fixture().join("alignment.json")).unwrap();
    let alignment: Value = serde_json::from_str(&text).unwrap();
    alignment["words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["word"].as_str().unwrap().to_string(),
                w["start"].as_f64().unwrap(),
                w["end"].as_f64().unwrap(),
            )
        })
        .collect()
}

/// Start time errors of the reference words found in order in `transcript`, in seconds.
fn start_errors(transcript: &Transcript, reference: &[(String, f64, f64)]) -> Vec<f64> {
    let mut next = 0;
    let mut errors = Vec::new();
    for (word, start, _) in reference {
        let found = (next..transcript.words.len().min(next + 3))
            .find(|&i| fosscut_analysis::speech::bare(&transcript.words[i].text) == *word);
        if let Some(i) = found {
            errors.push((seconds(&transcript.words[i].start) - start).abs());
            next = i + 1;
        }
    }
    errors
}

#[test]
fn speech_is_transcribed_with_word_timings() {
    let Some(models) = models_with_tiny() else {
        return;
    };
    let audio = fixture().join("audio.flac");
    let mut seen: Vec<Progress> = Vec::new();
    let results = analyze(
        media(&audio),
        &[Analyzer::Transcript],
        &tiny(),
        &models,
        &mut |p| {
            seen.push(p);
            true
        },
    )
    .unwrap();
    let transcript = results.transcript.unwrap();
    assert!(results.loudness.is_none() && results.silences.is_none());
    assert_eq!(transcript.model, "whisper-tiny.en");
    assert_eq!(transcript.language, "en", "English-only models never guess");
    assert!(transcript.fillers.is_empty(), "read speech has no fillers");
    let text = transcript
        .segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("sandy shore"), "{text}");

    // ADR-0020 Validation: at least 90% of the words, a median start error of at most
    // 100 ms, and at least 90% of the words within 300 ms.
    let reference = reference();
    let mut errors = start_errors(&transcript, &reference);
    let total = reference.len() as f64;
    assert!(errors.len() as f64 >= 0.9 * total, "{} words", errors.len());
    errors.sort_by(f64::total_cmp);
    let median = errors[errors.len() / 2];
    assert!(median <= 0.1, "median start error {median:.3} s");
    let close = errors.iter().filter(|&&e| e <= 0.3).count() as f64;
    assert!(
        close >= 0.9 * total,
        "{close} of {total} words within 300 ms"
    );

    // The silence before the first word is not part of it.
    let first = seconds(&transcript.words[0].start);
    assert!((first - 0.42).abs() <= 0.1, "first word at {first}");
    // The two pauses of the reading (3.45-3.82 s and 6.94-7.63 s) are found, and little else.
    for (from, to) in [(3.45, 3.82), (6.94, 7.63)] {
        assert!(
            transcript
                .pauses
                .iter()
                .any(|p| seconds(&p.start) < to && seconds(&p.end) > from),
            "no pause found at {from}-{to} s: {:?}",
            transcript.pauses
        );
    }
    assert!(transcript.pauses.len() <= 4, "{:?}", transcript.pauses);
    assert_eq!(transcript.min_pause_ms, 300);
    for pair in transcript.words.windows(2) {
        let (word, next) = (&pair[0], &pair[1]);
        assert!(word.start.flicks <= word.end.flicks && word.end.flicks <= next.start.flicks);
    }

    // Progress rises to the end.
    assert!(seen.windows(2).all(|w| w[0].done <= w[1].done));
    let last = seen.last().unwrap();
    assert_eq!(last.done, last.total);
}

#[test]
fn recognition_can_be_cancelled() {
    let Some(models) = models_with_tiny() else {
        return;
    };
    let audio = fixture().join("audio.flac");
    let stopped = analyze(
        media(&audio),
        &[Analyzer::Transcript],
        &tiny(),
        &models,
        &mut |_| false,
    )
    .unwrap_err();
    assert!(matches!(stopped, AnalysisError::Cancelled), "{stopped:?}");
}

#[test]
fn the_speech_model_is_chosen_from_what_is_installed() {
    let dir = std::env::temp_dir().join(format!("fosscut-speech-models-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut options = Options::default();
    assert_eq!(
        speech_model(&options, &dir),
        Err(ModelError::NotInstalled {
            what: "a speech model".into()
        })
    );
    options.transcript.language = Some("de".into());
    assert_eq!(
        speech_model(&options, &dir),
        Err(ModelError::NotInstalled {
            what: "a multilingual speech model (for de)".into()
        })
    );
    options.transcript.model = Some("whisper-base.en".into());
    assert_eq!(
        speech_model(&options, &dir),
        Err(ModelError::NotInstalled {
            what: "whisper-base.en".into()
        })
    );
    options.transcript.model = Some("whisper-huge".into());
    assert!(matches!(
        speech_model(&options, &dir),
        Err(ModelError::Unknown { .. })
    ));

    // An installed model is chosen by name, or as the best for the language.
    let base = models::manifest("whisper-base.en").unwrap();
    std::fs::File::create(models::path(&dir, base))
        .unwrap()
        .set_len(base.size())
        .unwrap();
    options.transcript.model = Some("whisper-base.en".into());
    assert_eq!(speech_model(&options, &dir), Ok(base));
    options.transcript.model = None;
    options.transcript.language = None;
    assert_eq!(speech_model(&options, &dir), Ok(base));

    // Asking for a transcript with no model installed fails before decoding anything.
    let empty = dir.join("empty");
    let missing = analyze(
        media(Path::new("does-not-exist.flac")),
        &[Analyzer::Transcript],
        &Options::default(),
        &empty,
        &mut |_| true,
    )
    .unwrap_err();
    assert!(
        matches!(
            missing,
            AnalysisError::Model(ModelError::NotInstalled { .. })
        ),
        "{missing:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
