//! Text to speech with Kokoro-82M (ADR-0033 item 5, Validation): speech is recognised again
//! by whisper as the same words, the same request gives the same samples, long text is cut
//! within the model's input, and words no dictionary has are still spoken. They need the
//! `onnxruntime` and `kokoro-82m` models in `FOSSCUT_MODELS_DIR` and skip without them;
//! `FOSSCUT_REQUIRE_MODELS` turns a missing model into a failure. The recognition check
//! also needs FFmpeg and `whisper-tiny.en` (`FOSSCUT_REQUIRE_SPEECH`). Timings print with
//! `--nocapture`.

#![cfg(feature = "onnx")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use fosscut_analysis::decode::{Media, analyze};
use fosscut_analysis::models;
use fosscut_analysis::voice::{
    self, Accent, MAX_PHONEMES, RATE, SENTENCE_PAUSE_MS, Speech, Synthesizer, VoiceError,
};
use fosscut_analysis::{Analyzer, Options, onnx};

fn required(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty() && v != "0")
}

/// The models folder when ONNX Runtime and the voice model are installed.
fn models_with_voices() -> Option<PathBuf> {
    let dir = std::env::var_os("FOSSCUT_MODELS_DIR").map(PathBuf::from);
    let kokoro = models::manifest(voice::MODEL).unwrap();
    let ready = dir
        .as_ref()
        .is_some_and(|d| onnx::runtime_path(d).is_some() && models::is_installed(d, kokoro));
    assert!(
        ready || !required("FOSSCUT_REQUIRE_MODELS"),
        "FOSSCUT_REQUIRE_MODELS is set but ONNX Runtime or kokoro-82m is not installed in FOSSCUT_MODELS_DIR"
    );
    ready.then(|| dir.unwrap())
}

/// The characters the voice model reads, from its tokenizer.
fn vocabulary(models: &Path) -> HashSet<char> {
    let kokoro = models::manifest(voice::MODEL).unwrap();
    let tokenizer: serde_json::Value = serde_json::from_slice(
        &std::fs::read(models::file_path(models, kokoro, "tokenizer.json")).unwrap(),
    )
    .unwrap();
    tokenizer["model"]["vocab"]
        .as_object()
        .unwrap()
        .keys()
        .filter_map(|k| k.chars().next())
        .collect()
}

/// Speaks `text`, printing how long it took against how long the speech lasts.
fn speak(synthesizer: &mut Synthesizer, text: &str, voice: &str, speed: f32) -> Speech {
    let started = Instant::now();
    let speech = synthesizer.speak(text, voice, speed).unwrap();
    let took = started.elapsed().as_secs_f64();
    eprintln!(
        "{voice} at {speed}: {:.2} s of speech in {took:.2} s (real-time factor {:.3}) for {:?}",
        speech.seconds(),
        took / speech.seconds(),
        text.chars().take(48).collect::<String>()
    );
    speech
}

#[test]
fn a_missing_model_or_a_bad_request_is_a_typed_error() {
    let missing = std::env::temp_dir().join(format!("fosscut-no-voices-{}", std::process::id()));
    let error = voice::synthesize(&missing, "Hello.", "af_heart", 1.0).unwrap_err();
    assert_eq!(error.code(), "fosscut.model.not_installed", "{error}");
    assert!(voice::voices(&missing).iter().all(|v| !v.installed));

    let Some(models) = models_with_voices() else {
        return;
    };
    assert_eq!(voice::voices(&models).len(), 28);
    assert!(voice::voices(&models).iter().all(|v| v.installed));
    let started = Instant::now();
    let mut synthesizer = Synthesizer::open(&models).unwrap();
    eprintln!(
        "voice model and pronouncing dictionary loaded in {} ms",
        started.elapsed().as_millis()
    );
    let started = Instant::now();
    let dictionary = voice::Dictionary::open(&models).unwrap();
    eprintln!(
        "pronouncing dictionary ({} words) again in {} ms",
        dictionary.len(),
        started.elapsed().as_millis()
    );
    for (text, voice, speed, code) in [
        ("Hello.", "hu_anna", 1.0, "fosscut.voice.unknown_voice"),
        (
            "Hello.",
            "af_heart",
            4.0,
            "fosscut.request.invalid_arguments",
        ),
        (
            " ... !? ",
            "af_heart",
            1.0,
            "fosscut.request.invalid_arguments",
        ),
        ("", "af_heart", 1.0, "fosscut.request.invalid_arguments"),
        (
            "Привет, мир.",
            "af_heart",
            1.0,
            "fosscut.voice.language_unavailable",
        ),
    ] {
        let error = synthesizer.speak(text, voice, speed).unwrap_err();
        assert_eq!(error.code(), code, "{text:?}: {error}");
    }
    assert_eq!(
        synthesizer.speak("", "af_heart", 1.0).unwrap_err(),
        VoiceError::EmptyText
    );
}

#[test]
fn pronunciation_uses_only_phonemes_the_model_reads() {
    let Some(models) = models_with_voices() else {
        return;
    };
    let known = vocabulary(&models);
    let dictionary = voice::Dictionary::open(&models).unwrap();
    assert!(dictionary.len() > 120_000, "{} words", dictionary.len());
    let text = "Dr. Smith paid $12.50 for 3 e-books on Oct. 31st, 2026 at 3:45 pm! \
        They record a record; the schedule is full. Zorblax, qzxwv and FossCut's \
        Kubernetes cluster met at the moonbase. Köszönöm szépen. 😀 #hashtag @name";
    for accent in [Accent::American, Accent::British] {
        let phrases = voice::phrases(&dictionary, text, accent).unwrap();
        assert!(phrases.len() >= 5, "{phrases:?}");
        for phrase in &phrases {
            for c in phrase.phonemes.chars() {
                assert!(
                    known.contains(&c),
                    "{accent:?}: {c:?} in {:?} is not in the tokenizer",
                    phrase.phonemes
                );
            }
        }
        let all: String = phrases.iter().map(|p| p.phonemes.as_str()).collect();
        eprintln!("{accent:?}: {all}");
        // Numbers, money and abbreviations are read out; unknown words get sounds.
        assert!(
            all.contains("twˈɛlv dˈɑləɹz") || all.contains("twˈɛlv dˈɒləz"),
            "{all}"
        );
        assert!(all.contains("dˈɑktəɹ") || all.contains("dˈɒktə"), "{all}");
        // Invented words get sounds; "qzxwv" is spelled out.
        for word in ["Zorblax", "Kubernetes", "FossCut's"] {
            let sounds = voice::phrases(&dictionary, word, accent).unwrap();
            assert!(
                !sounds.is_empty() && !sounds[0].phonemes.is_empty(),
                "{word}"
            );
        }
        assert!(all.contains("kjˌu"), "{all}");
        assert!(!all.contains('❓'));
    }
    // Spaces between paragraphs, sentences and clauses become pauses of their own.
    let phrases = voice::phrases(&dictionary, "One. Two!\nThree", Accent::American).unwrap();
    let pauses: Vec<u32> = phrases.iter().map(|p| p.pause_ms).collect();
    assert_eq!(
        pauses,
        [SENTENCE_PAUSE_MS, voice::PARAGRAPH_PAUSE_MS, 0],
        "{phrases:?}"
    );
}

#[test]
fn speech_is_recognised_as_the_same_words() {
    let Some(models) = models_with_voices() else {
        return;
    };
    let ffmpeg = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    let tiny = models::manifest("whisper-tiny.en").unwrap();
    let whisper = ffmpeg && models::is_installed(&models, tiny);
    assert!(
        whisper || !required("FOSSCUT_REQUIRE_SPEECH"),
        "FOSSCUT_REQUIRE_SPEECH is set but ffmpeg or whisper-tiny.en is missing"
    );
    let mut synthesizer = Synthesizer::open(&models).unwrap();
    let fox = "The quick brown fox jumps over the lazy dog.";
    let speech = speak(&mut synthesizer, fox, "af_heart", 1.0);
    assert_eq!(speech.rate, RATE);
    assert!(
        (1.5..6.0).contains(&speech.seconds()),
        "{} s",
        speech.seconds()
    );
    let peak = speech.samples.iter().fold(0.0f32, |a, s| a.max(s.abs()));
    assert!((0.05..=0.99).contains(&peak), "peak {peak}");
    // Numbers read in words and a word no dictionary has.
    let counts = "FossCut 2 edits 15 clips in 3 minutes.";
    let numbers = speak(&mut synthesizer, counts, "af_heart", 1.0);
    if !whisper {
        return;
    }
    let heard = hear(&models, &speech);
    eprintln!("whisper-tiny.en heard: {:?}", heard.join(" "));
    let said: Vec<&str> = fox.split_whitespace().collect();
    let found = said.iter().filter(|w| heard.contains(&plain(w))).count();
    assert!(
        found * 10 >= said.len() * 8,
        "{found} of {} words heard: {heard:?}",
        said.len()
    );
    let heard = hear(&models, &numbers);
    eprintln!("whisper-tiny.en heard: {:?}", heard.join(" "));
    let expected: [&[&str]; 7] = [
        &["2", "two", "to", "too"],
        &["edits"],
        &["15", "fifteen"],
        &["clips"],
        &["in"],
        &["3", "three"],
        &["minutes"],
    ];
    let found = expected
        .iter()
        .filter(|forms| forms.iter().any(|f| heard.iter().any(|h| h == f)))
        .count();
    assert!(found >= 6, "{found} of 7 words heard: {heard:?}");
}

/// A word as compared: lowercase letters and digits.
fn plain(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The words whisper-tiny.en recognises in `speech`.
fn hear(models: &Path, speech: &Speech) -> Vec<String> {
    let file = std::env::temp_dir().join(format!(
        "fosscut-voice-{}-{}.wav",
        std::process::id(),
        speech.samples.len()
    ));
    std::fs::write(&file, wav(speech)).unwrap();
    let mut options = Options::default();
    options.transcript.model = Some("whisper-tiny.en".into());
    let results = analyze(
        Media {
            path: &file,
            duration: speech.duration(),
            audio_channels: Some(1),
            video_rate: None,
        },
        &[Analyzer::Transcript],
        &options,
        models,
        &mut |_| true,
    )
    .unwrap();
    let _ = std::fs::remove_file(&file);
    results
        .transcript
        .unwrap()
        .words
        .iter()
        .map(|w| plain(&w.text))
        .filter(|w| !w.is_empty())
        .collect()
}

#[test]
fn the_same_request_gives_the_same_samples() {
    let Some(models) = models_with_voices() else {
        return;
    };
    let text = "Determinism matters: the same words, the same voice.";
    let mut first = Synthesizer::open(&models).unwrap();
    let a = speak(&mut first, text, "am_michael", 1.0);
    let b = speak(&mut first, text, "am_michael", 1.0);
    assert_eq!(a, b, "the same synthesizer");
    let c = voice::synthesize(&models, text, "am_michael", 1.0).unwrap();
    assert_eq!(a, c, "a new synthesizer");
    let faster = speak(&mut first, text, "am_michael", 1.5);
    assert!(
        faster.seconds() < a.seconds() * 0.85,
        "{} s at 1.5 against {} s",
        faster.seconds(),
        a.seconds()
    );
    let british = speak(&mut first, text, "bf_emma", 1.0);
    assert_ne!(british, a);
    assert!(british.seconds() > 1.0);
}

#[test]
fn long_text_is_cut_within_the_model_input_and_joined_with_pauses() {
    let Some(models) = models_with_voices() else {
        return;
    };
    // One sentence longer than the model reads at once, then a short one. Kept just past
    // one input: speaking it is the slow part, and CI's runners are slower than a laptop.
    let clause = "the editor cuts the long recording into shorter pieces, ";
    let text = format!("{}and then it stops. Done.", clause.repeat(11));
    let dictionary = voice::Dictionary::open(&models).unwrap();
    let phrases = voice::phrases(&dictionary, &text, Accent::American).unwrap();
    assert!(phrases.len() >= 3, "{}", phrases.len());
    assert!(
        phrases
            .iter()
            .all(|p| p.phonemes.chars().count() <= MAX_PHONEMES)
    );
    let total: usize = phrases.iter().map(|p| p.phonemes.chars().count()).sum();
    assert!(total > MAX_PHONEMES, "{total} phonemes");
    let mut synthesizer = Synthesizer::open(&models).unwrap();
    let speech = speak(&mut synthesizer, &text, "af_heart", 1.0);
    assert!(speech.seconds() > 10.0, "{} s", speech.seconds());
    // The sentence pause before "Done." is exact silence.
    let pause = (SENTENCE_PAUSE_MS * RATE / 1000) as usize;
    let longest_silence = speech
        .samples
        .split(|s| *s != 0.0)
        .map(<[f32]>::len)
        .max()
        .unwrap();
    assert!(longest_silence >= pause, "{longest_silence} silent samples");
}

#[test]
fn words_no_dictionary_has_are_spoken() {
    let Some(models) = models_with_voices() else {
        return;
    };
    let mut synthesizer = Synthesizer::open(&models).unwrap();
    for text in [
        "Zorblax!",
        "qzxwv",
        "FossCut's Kubernetes",
        "Köszönöm szépen.",
        "42",
    ] {
        let speech = speak(&mut synthesizer, text, "af_heart", 1.0);
        assert!(speech.seconds() > 0.2, "{text}: {} s", speech.seconds());
        assert!(
            speech.samples.iter().any(|s| s.abs() > 0.01),
            "{text} is silent"
        );
    }
}

/// 16-bit PCM WAV of `speech`.
fn wav(speech: &Speech) -> Vec<u8> {
    let data = (speech.samples.len() * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&speech.rate.to_le_bytes());
    out.extend_from_slice(&(speech.rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for sample in &speech.samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}
