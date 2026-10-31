//! Smoke tests for the ONNX models (ADR-0033): each installed model loads in a session on
//! the CPU, runs one inference on a synthetic input of the right shape, and returns outputs
//! of the expected shapes. They need the `onnxruntime` model and the model under test in
//! `FOSSCUT_MODELS_DIR` and skip without them; `FOSSCUT_REQUIRE_MODELS` turns a missing
//! runtime or model into a failure. Timings print with `--nocapture`.

#![cfg(feature = "onnx")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::Instant;

use fosscut_analysis::models::{self, Manifest};
use fosscut_analysis::onnx::ort::session::SessionInputValue;
use fosscut_analysis::onnx::ort::value::{Tensor, TensorElementType, ValueType};
use fosscut_analysis::onnx::{self, Device, Session, SessionOptions};

fn required() -> bool {
    std::env::var_os("FOSSCUT_REQUIRE_MODELS").is_some_and(|v| !v.is_empty() && v != "0")
}

/// The models folder and manifest when the runtime and model `name` are installed.
fn installed(name: &str) -> Option<(PathBuf, &'static Manifest)> {
    let dir = std::env::var_os("FOSSCUT_MODELS_DIR").map(PathBuf::from);
    let model = models::manifest(name).unwrap();
    let ready = dir
        .as_ref()
        .is_some_and(|d| onnx::runtime_path(d).is_some() && models::is_installed(d, model));
    assert!(
        ready || !required(),
        "FOSSCUT_REQUIRE_MODELS is set but ONNX Runtime or {name} is not installed in FOSSCUT_MODELS_DIR"
    );
    ready.then(|| (dir.unwrap(), model))
}

/// A CPU session for file `file` of `model`, timed.
fn open(dir: &Path, model: &Manifest, file: &str, device: Device) -> Session {
    let started = Instant::now();
    let session = onnx::session(
        &models::file_path(dir, model, file),
        &SessionOptions::new(dir).on(device),
    )
    .unwrap();
    eprintln!(
        "{}/{file} on {device:?}: session in {} ms",
        model.name,
        started.elapsed().as_millis()
    );
    session
}

/// The names of `session`'s inputs or outputs with their element types and shapes, as the
/// model declares them (-1 for a dynamic dimension).
fn declared(
    outlets: &[fosscut_analysis::onnx::ort::value::Outlet],
) -> Vec<(String, TensorElementType, Vec<i64>)> {
    outlets
        .iter()
        .map(|o| match o.dtype() {
            ValueType::Tensor { ty, shape, .. } => (o.name().to_string(), *ty, shape.to_vec()),
            other => panic!("{} is not a tensor: {other}", o.name()),
        })
        .collect()
}

fn floats(shape: &[usize], value: impl Fn(usize) -> f32) -> Tensor<f32> {
    let count = shape.iter().product();
    Tensor::from_array((shape.to_vec(), (0..count).map(value).collect::<Vec<f32>>())).unwrap()
}

fn integers(shape: &[usize], values: Vec<i64>) -> Tensor<i64> {
    Tensor::from_array((shape.to_vec(), values)).unwrap()
}

/// A float output's shape and values, all of them finite.
fn output(
    outputs: &fosscut_analysis::onnx::ort::session::SessionOutputs<'_>,
    name: &str,
) -> (Vec<i64>, Vec<f32>) {
    let (shape, values) = outputs[name].try_extract_tensor::<f32>().unwrap();
    assert!(
        values.iter().all(|v| v.is_finite()),
        "{name} has non-finite values"
    );
    (shape.to_vec(), values.to_vec())
}

#[test]
fn birefnet_lite_mattes_a_frame() {
    let Some((dir, model)) = installed("birefnet-lite") else {
        return;
    };
    let mut session = open(&dir, model, "model.onnx", Device::Cpu);
    use TensorElementType::Float32;
    assert_eq!(
        declared(session.inputs()),
        [("input_image".into(), Float32, vec![1, 3, 1024, 1024])]
    );
    assert_eq!(
        declared(session.outputs()),
        [("output_image".into(), Float32, vec![1, 1, 1024, 1024])]
    );
    // A light square on a dark background, normalized with the ImageNet mean and deviation.
    let (mean, deviation) = ([0.485, 0.456, 0.406], [0.229, 0.224, 0.225]);
    let image = floats(&[1, 3, 1024, 1024], |i| {
        let (channel, y, x) = (i / (1024 * 1024), (i / 1024) % 1024, i % 1024);
        let inside = (384..640).contains(&x) && (256..768).contains(&y);
        let value = if inside { 0.9 } else { 0.2 };
        (value - mean[channel]) / deviation[channel]
    });
    let started = Instant::now();
    let outputs = session
        .run(ort_inputs(vec![("input_image", image.into())]))
        .unwrap();
    eprintln!(
        "birefnet-lite on Cpu: 1024x1024 frame in {} ms",
        started.elapsed().as_millis()
    );
    let (shape, logits) = output(&outputs, "output_image");
    assert_eq!(shape, [1, 1, 1024, 1024]);
    let spread = logits.iter().fold(f32::MAX, |a, &b| a.min(b))
        ..logits.iter().fold(f32::MIN, |a, &b| a.max(b));
    assert!(spread.end > spread.start, "the matte is not flat");
}

#[test]
fn mediapipe_pose_detects_and_finds_landmarks() {
    let Some((dir, model)) = installed("mediapipe-pose-full") else {
        return;
    };
    use TensorElementType::Float32;
    let mut detector = open(&dir, model, "detector.onnx", Device::Cpu);
    assert_eq!(
        declared(detector.inputs()),
        [("input_1".into(), Float32, vec![1, 224, 224, 3])]
    );
    assert_eq!(
        declared(detector.outputs()),
        [
            ("Identity".into(), Float32, vec![1, 2254, 12]),
            ("Identity_1".into(), Float32, vec![1, 2254, 1]),
        ]
    );
    // Pixels scaled to -1..1, channels last.
    let frame = floats(&[1, 224, 224, 3], |i| ((i % 251) as f32 / 125.0) - 1.0);
    let started = Instant::now();
    let outputs = detector
        .run(ort_inputs(vec![("input_1", frame.into())]))
        .unwrap();
    eprintln!("pose detector on Cpu: {} ms", started.elapsed().as_millis());
    assert_eq!(output(&outputs, "Identity").0, [1, 2254, 12]);
    assert_eq!(output(&outputs, "Identity_1").0, [1, 2254, 1]);

    let mut landmarks = open(&dir, model, "landmarks.onnx", Device::Cpu);
    assert_eq!(
        declared(landmarks.inputs()),
        [("input_1".into(), Float32, vec![1, 256, 256, 3])]
    );
    let expected = [
        ("Identity", vec![1, 195]),
        ("Identity_1", vec![1, 1]),
        ("Identity_2", vec![1, 256, 256, 1]),
        ("Identity_3", vec![1, 64, 64, 39]),
        ("Identity_4", vec![1, 117]),
    ];
    assert_eq!(
        declared(landmarks.outputs()),
        expected
            .iter()
            .map(|(name, shape)| (name.to_string(), Float32, shape.clone()))
            .collect::<Vec<_>>()
    );
    // Pixels scaled to 0..1, channels last. The CPU gives the same values on every run.
    let crop = |i: usize| (i % 253) as f32 / 252.0;
    let mut runs = Vec::new();
    for _ in 0..2 {
        let started = Instant::now();
        let outputs = landmarks
            .run(ort_inputs(vec![(
                "input_1",
                floats(&[1, 256, 256, 3], crop).into(),
            )]))
            .unwrap();
        eprintln!(
            "pose landmarks on Cpu: {} ms",
            started.elapsed().as_millis()
        );
        for (name, shape) in &expected {
            assert_eq!(&output(&outputs, name).0, shape);
        }
        runs.push(output(&outputs, "Identity").1);
    }
    assert_eq!(runs[0], runs[1], "CPU sessions are deterministic");
}

#[test]
fn kokoro_speaks_phonemes() {
    let Some((dir, model)) = installed("kokoro-82m") else {
        return;
    };
    let mut session = open(&dir, model, "model.onnx", Device::Cpu);
    use TensorElementType::{Float32, Int64};
    assert_eq!(
        declared(session.inputs()),
        [
            ("input_ids".into(), Int64, vec![1, -1]),
            ("style".into(), Float32, vec![1, 256]),
            ("speed".into(), Float32, vec![1]),
        ]
    );
    assert_eq!(
        declared(session.outputs()),
        [("waveform".into(), Float32, vec![1, -1])]
    );
    // Phonemes of "hello world" mapped through the tokenizer's vocabulary, between the pad
    // token 0 at each end.
    let tokenizer: serde_json::Value = serde_json::from_slice(
        &std::fs::read(models::file_path(&dir, model, "tokenizer.json")).unwrap(),
    )
    .unwrap();
    let vocabulary = &tokenizer["model"]["vocab"];
    let mut ids = vec![0i64];
    for phoneme in "həlˈO wˈɜɹld".chars() {
        ids.push(vocabulary[phoneme.to_string()].as_i64().unwrap());
    }
    ids.push(0);
    // A voice file holds 510 style vectors of 256 floats; the one for n phonemes is row n.
    let voice = std::fs::read(models::file_path(&dir, model, "voices/af_heart.bin")).unwrap();
    assert_eq!(voice.len(), 510 * 256 * 4);
    let row = ids.len() - 2;
    let style: Vec<f32> = voice[row * 1024..(row + 1) * 1024]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    let started = Instant::now();
    let outputs = session
        .run(ort_inputs(vec![
            ("input_ids", integers(&[1, ids.len()], ids.clone()).into()),
            (
                "style",
                Tensor::from_array((vec![1usize, 256], style))
                    .unwrap()
                    .into(),
            ),
            (
                "speed",
                Tensor::from_array((vec![1usize], vec![1.0f32]))
                    .unwrap()
                    .into(),
            ),
        ]))
        .unwrap();
    let (shape, samples) = output(&outputs, "waveform");
    eprintln!(
        "kokoro-82m on Cpu: {} samples ({:.2} s at 24 kHz) in {} ms",
        samples.len(),
        samples.len() as f64 / 24_000.0,
        started.elapsed().as_millis()
    );
    assert_eq!(shape.len(), 2);
    assert_eq!(shape[0], 1);
    assert!(samples.len() > 12_000, "at least half a second of speech");
    assert!(
        samples.iter().any(|s| s.abs() > 0.01),
        "the voice is not silent"
    );
}

#[test]
fn opus_mt_translates_hungarian_to_english() {
    let Some((dir, model)) = installed("opus-mt-hu-en") else {
        return;
    };
    let text = translate(&dir, model, &["▁Köszönöm", "▁szépen", "."]);
    assert!(text.to_lowercase().contains("thank"), "{text}");
}

#[test]
fn opus_mt_translates_english_to_hungarian() {
    let Some((dir, model)) = installed("opus-mt-en-hu") else {
        return;
    };
    let text = translate(&dir, model, &["▁Thank", "▁you", "▁very", "▁much", "."]);
    // "Nagyon szépen köszönöm." in testing.
    assert!(text.to_lowercase().contains("köszönöm"), "{text}");
}

/// Translates SentencePiece `pieces` with OPUS-MT `model` by greedy decoding, checking the
/// encoder's and decoder's inputs and outputs on the way.
fn translate(dir: &Path, model: &Manifest, pieces: &[&str]) -> String {
    use TensorElementType::{Bool, Float32, Int64};
    let mut encoder = open(dir, model, "encoder.onnx", Device::Cpu);
    assert_eq!(
        declared(encoder.inputs()),
        [
            ("input_ids".into(), Int64, vec![-1, -1]),
            ("attention_mask".into(), Int64, vec![-1, -1]),
        ]
    );
    assert_eq!(
        declared(encoder.outputs()),
        [("last_hidden_state".into(), Float32, vec![-1, -1, 512])]
    );
    let mut decoder = open(dir, model, "decoder.onnx", Device::Cpu);
    let inputs = declared(decoder.inputs());
    assert_eq!(inputs.len(), 3 + 6 * 4 + 1);
    assert_eq!(inputs[1], ("input_ids".into(), Int64, vec![-1, -1]));
    assert_eq!(
        inputs[3],
        (
            "past_key_values.0.decoder.key".into(),
            Float32,
            vec![-1, 8, -1, 64]
        )
    );
    assert_eq!(inputs[27], ("use_cache_branch".into(), Bool, vec![1]));
    assert_eq!(
        declared(decoder.outputs())[0],
        ("logits".into(), Float32, vec![-1, -1, 62_522])
    );

    // The pieces looked up in vocab.json, then </s> (0).
    let vocabulary: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(
        &std::fs::read(models::file_path(dir, model, "vocab.json")).unwrap(),
    )
    .unwrap();
    let mut source: Vec<i64> = pieces
        .iter()
        .map(|piece| vocabulary[*piece].as_i64().unwrap())
        .collect();
    source.push(0);
    let n = source.len();
    let started = Instant::now();
    let encoded = encoder
        .run(ort_inputs(vec![
            ("input_ids", integers(&[1, n], source.clone()).into()),
            ("attention_mask", integers(&[1, n], vec![1; n]).into()),
        ]))
        .unwrap();
    let (shape, hidden) = output(&encoded, "last_hidden_state");
    assert_eq!(shape, [1, n as i64, 512]);
    drop(encoded);

    // Greedy decoding from the start token (62521): the first step has an empty cache and
    // use_cache_branch false; later steps feed back the present keys and values.
    let names: Vec<String> = declared(decoder.inputs())
        .into_iter()
        .map(|i| i.0)
        .collect();
    let mut past: Vec<(Vec<usize>, Vec<f32>)> = names[3..27]
        .iter()
        .map(|_| (vec![1, 8, 0, 64], Vec::new()))
        .collect();
    let mut tokens = vec![62_521i64];
    for step in 0..12 {
        let mut feed: Vec<(&str, SessionInputValue<'_>)> = vec![
            (
                "encoder_attention_mask",
                integers(&[1, n], vec![1; n]).into(),
            ),
            (
                "input_ids",
                integers(&[1, 1], vec![*tokens.last().unwrap()]).into(),
            ),
            (
                "encoder_hidden_states",
                Tensor::from_array((vec![1usize, n, 512], hidden.clone()))
                    .unwrap()
                    .into(),
            ),
        ];
        for (name, (shape, values)) in names[3..27].iter().zip(&past) {
            feed.push((
                name,
                Tensor::from_array((shape.clone(), values.clone()))
                    .unwrap()
                    .into(),
            ));
        }
        feed.push((
            "use_cache_branch",
            Tensor::from_array((vec![1usize], vec![step > 0]))
                .unwrap()
                .into(),
        ));
        let outputs = decoder.run(ort_inputs(feed)).unwrap();
        let (shape, logits) = output(&outputs, "logits");
        assert_eq!(shape, [1, 1, 62_522]);
        // The pad token is never chosen (bad_words_ids in generation_config.json).
        let next = logits[..62_521]
            .iter()
            .enumerate()
            .fold(
                (0, f32::MIN),
                |best, (i, &l)| if l > best.1 { (i, l) } else { best },
            )
            .0 as i64;
        assert_eq!(
            output(&outputs, "present.0.decoder.key").0,
            [1, 8, step as i64 + 1, 64]
        );
        // The cross-attention keys and values come from the first step only; the cached
        // branch returns empty placeholders for them, so they are kept.
        if step == 0 {
            assert_eq!(
                output(&outputs, "present.0.encoder.key").0,
                [1, 8, n as i64, 64]
            );
        }
        for (name, slot) in names[3..27].iter().zip(past.iter_mut()) {
            if step == 0 || name.contains(".decoder.") {
                let (shape, values) = output(&outputs, &name.replace("past_key_values", "present"));
                *slot = (shape.iter().map(|&d| d as usize).collect(), values);
            }
        }
        tokens.push(next);
        if next == 0 {
            break;
        }
    }
    let by_id: std::collections::HashMap<i64, &str> = vocabulary
        .iter()
        .map(|(piece, id)| (id.as_i64().unwrap(), piece.as_str()))
        .collect();
    let text: String = tokens[1..]
        .iter()
        .filter(|&&t| t != 0)
        .map(|t| by_id[t])
        .collect::<String>()
        .replace('▁', " ");
    let text = text.trim().to_string();
    eprintln!(
        "{} on Cpu: {:?} -> \"{text}\" in {} ms",
        model.name,
        pieces.concat().replace('▁', " ").trim(),
        started.elapsed().as_millis()
    );
    text
}

#[test]
fn the_accelerator_falls_back_to_the_cpu() {
    // CoreML cannot compile the pose landmark model, so the accelerator session is a CPU
    // one; it runs and returns the same shapes. On Windows, Microsoft's release has no
    // DirectML and the CPU runs it too.
    let Some((dir, model)) = installed("mediapipe-pose-full") else {
        return;
    };
    let mut landmarks = open(&dir, model, "landmarks.onnx", Device::Accelerator);
    let started = Instant::now();
    let outputs = landmarks
        .run(ort_inputs(vec![(
            "input_1",
            floats(&[1, 256, 256, 3], |i| (i % 7) as f32 / 7.0).into(),
        )]))
        .unwrap();
    eprintln!(
        "pose landmarks on Accelerator: {} ms",
        started.elapsed().as_millis()
    );
    assert_eq!(output(&outputs, "Identity").0, [1, 195]);
    assert_eq!(output(&outputs, "Identity_2").0, [1, 256, 256, 1]);
}

#[test]
fn a_damaged_model_fails_to_load_with_a_typed_error() {
    let Some((dir, _)) = installed("mediapipe-pose-full") else {
        return;
    };
    let broken = std::env::temp_dir().join(format!("fosscut-broken-{}.onnx", std::process::id()));
    std::fs::write(&broken, b"not a model").unwrap();
    for device in [Device::Cpu, Device::Auto, Device::Accelerator] {
        let error = onnx::session(&broken, &SessionOptions::new(&dir).on(device)).unwrap_err();
        assert_eq!(error.code(), "fosscut.model.load_failed", "{error}");
    }
    let missing = onnx::session(&dir.join("missing.onnx"), &SessionOptions::new(&dir)).unwrap_err();
    assert!(matches!(missing, onnx::OnnxError::LoadFailed { .. }));
    let _ = std::fs::remove_file(&broken);
    let runtime = onnx::runtime(&dir).unwrap();
    assert_eq!(Some(runtime.library), onnx::runtime_path(&dir));
    assert!(
        runtime.build.contains("ORT Build Info"),
        "{}",
        runtime.build
    );
}

/// Named inputs for `Session::run`.
fn ort_inputs<'v>(
    inputs: Vec<(&str, SessionInputValue<'v>)>,
) -> Vec<(Cow<'static, str>, SessionInputValue<'v>)> {
    inputs
        .into_iter()
        .map(|(name, value)| (Cow::Owned(name.to_string()), value))
        .collect()
}
