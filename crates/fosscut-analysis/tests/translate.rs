//! Translation with OPUS-MT (ADR-0033 item 6, Validation): English to Hungarian and back
//! keeps the meaning of short sentences (checked for known words), batches give the same
//! translations as one text at a time, and the same texts always give the same results.
//! They need `onnxruntime` and the pair's model in `FOSSCUT_MODELS_DIR` and skip without
//! them; `FOSSCUT_REQUIRE_MODELS` turns a missing model into a failure. Timings print with
//! `--nocapture`.

#![cfg(feature = "onnx")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Instant;

use fosscut_analysis::translate::{self, Translator};
use fosscut_analysis::{models, onnx};

fn required() -> bool {
    std::env::var_os("FOSSCUT_REQUIRE_MODELS").is_some_and(|v| !v.is_empty() && v != "0")
}

/// The models folder when ONNX Runtime and the model for `pair` are installed.
fn installed(pair: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("FOSSCUT_MODELS_DIR").map(PathBuf::from);
    let model = translate::model_for(pair).unwrap();
    let ready = dir
        .as_ref()
        .is_some_and(|d| onnx::runtime_path(d).is_some() && models::is_installed(d, model));
    assert!(
        ready || !required(),
        "FOSSCUT_REQUIRE_MODELS is set but ONNX Runtime or {} is not installed in FOSSCUT_MODELS_DIR",
        model.name
    );
    ready.then(|| dir.unwrap())
}

fn strings(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|t| t.to_string()).collect()
}

/// Translates `texts`, printing the speed.
fn timed(translator: &mut Translator, texts: &[String]) -> Vec<String> {
    let started = Instant::now();
    let out = translator.translate(texts).unwrap();
    let took = started.elapsed().as_secs_f64();
    eprintln!(
        "{}: {} texts in {took:.2} s ({:.1} per second)",
        translator.pair(),
        texts.len(),
        texts.len() as f64 / took
    );
    for (text, translation) in texts.iter().zip(&out) {
        eprintln!("  {text:?} -> {translation:?}");
    }
    out
}

/// Whether `text` has every word of `words`, ignoring case.
fn has(text: &str, words: &[&str]) -> bool {
    let lower = text.to_lowercase();
    words.iter().all(|w| lower.contains(w))
}

#[test]
fn english_becomes_hungarian() {
    let Some(models) = installed("en>hu") else {
        return;
    };
    let mut translator = Translator::open(&models, "en>hu").unwrap();
    assert_eq!(translator.pair(), "en>hu");
    let texts = strings(&[
        "Thank you very much.",
        "The dog is sleeping.",
        "Good morning!",
        "",
        "I have two children.",
    ]);
    let out = timed(&mut translator, &texts);
    assert_eq!(out.len(), texts.len());
    assert!(has(&out[0], &["köszönöm"]), "{}", out[0]);
    assert!(has(&out[1], &["kutya"]), "{}", out[1]);
    assert!(has(&out[2], &["reggel"]), "{}", out[2]);
    assert_eq!(out[3], "", "an empty text stays empty");
    assert!(has(&out[4], &["két", "gyerek"]), "{}", out[4]);
}

#[test]
fn hungarian_becomes_english() {
    let Some(models) = installed("hu>en") else {
        return;
    };
    let mut translator = Translator::open(&models, "hu>en").unwrap();
    let texts = strings(&[
        "Köszönöm szépen.",
        "A kutya alszik.",
        "Jó reggelt!",
        "Két gyerekem van.",
        "  Ma   este\nmoziba megyünk.  ",
    ]);
    let out = timed(&mut translator, &texts);
    assert!(has(&out[0], &["thank"]), "{}", out[0]);
    assert!(has(&out[1], &["dog", "sleep"]), "{}", out[1]);
    assert!(has(&out[2], &["morning"]), "{}", out[2]);
    assert!(
        has(&out[3], &["two", "child"]) || has(&out[3], &["two", "kids"]),
        "{}",
        out[3]
    );
    assert!(
        has(&out[4], &["movie"]) || has(&out[4], &["cinema"]),
        "{}",
        out[4]
    );
}

#[test]
fn batches_translate_as_one_text_at_a_time_and_the_same_every_time() {
    let Some(models) = installed("hu>en") else {
        return;
    };
    let texts = strings(&[
        "Ez egy rövid mondat.",
        "A macska az asztalon ül, és nézi az esőt.",
        "Holnap reggel korán indulunk a hegyekbe.",
        "Igen.",
        "A videó végén mindenki tapsolt.",
        "Ki tudod kapcsolni a zenét?",
        "Nagyon szeretem ezt a dalt.",
        "A vonat tíz perc késéssel érkezett meg az állomásra.",
        "Szép napunk van.",
        "Mennyibe kerül ez a kabát?",
    ]);
    let mut translator = Translator::open(&models, "hu>en").unwrap();
    let together = timed(&mut translator, &texts);
    // Batches of eight are faster and repeatable, but may word some texts differently.
    let mut batched = Translator::open(&models, "hu>en").unwrap().with_batch(8);
    let eight = timed(&mut batched, &texts);
    assert_eq!(batched.translate(&texts).unwrap(), eight, "batches repeat");
    let started = Instant::now();
    let alone: Vec<String> = texts
        .iter()
        .map(|t| {
            translator
                .translate(std::slice::from_ref(t))
                .unwrap()
                .remove(0)
        })
        .collect();
    eprintln!(
        "one at a time: {} texts in {:.2} s",
        texts.len(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(together, alone);
    assert!(together.iter().all(|t| !t.is_empty()));
    assert_eq!(translator.translate(&texts).unwrap(), together, "again");
    let fresh = translate::translate(&models, "hu>en", &texts).unwrap();
    assert_eq!(fresh, together, "a new translator");
}

#[test]
fn long_texts_are_translated_in_segments() {
    let Some(models) = installed("en>hu") else {
        return;
    };
    let text = vec![
        "The editor opens the project. The agent cuts the silences out. \
        Then it adds captions to every page. The music fades in after ten seconds. \
        We export the video for the phone. The colours look warm and bright. \
        The speaker thanks the audience at the end. Everyone claps and smiles. \
        The camera moves slowly to the left. A dog runs across the garden. \
        The title appears in large letters. The film is ready to share."
            .to_string(),
    ];
    let mut translator = Translator::open(&models, "en>hu").unwrap();
    let started = Instant::now();
    let out = translator.translate(&text).unwrap();
    eprintln!(
        "en>hu: {} characters in {:.2} s -> {} characters",
        text[0].len(),
        started.elapsed().as_secs_f64(),
        out[0].len()
    );
    assert!(out[0].len() > text[0].len() / 2, "{}", out[0]);
    // Sentences come back whole; the model may join a few with commas.
    assert!(out[0].matches('.').count() >= 5, "{}", out[0]);
    assert!(
        has(&out[0], &["szerkesztő", "zene", "kamera", "kutya", "film"]),
        "{}",
        out[0]
    );
}

#[test]
fn pairs_that_are_missing_or_unknown_are_typed_errors() {
    let missing = std::env::temp_dir().join(format!("fosscut-no-mt-{}", std::process::id()));
    let error = Translator::open(&missing, "hu>en").unwrap_err();
    assert_eq!(error.code(), "fosscut.model.not_installed", "{error}");
    let error = Translator::open(&missing, "en>de").unwrap_err();
    assert_eq!(error.code(), "fosscut.translate.unsupported_pair");
    assert!(error.to_string().contains("en>hu"), "{error}");
}
