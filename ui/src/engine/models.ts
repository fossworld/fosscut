// Stand-ins for the local models (ADR-0033): the models as `models_list` lists them, and
// the voices of the voice model, for the test engine and the browser's sample project.

import type { ModelInfo, ModelListing, VoiceInfo } from "./types";

function model(name: string, task: string, size: number, license: string, attribution: string, languages = "none"): ModelInfo {
  return {
    name,
    task,
    size_bytes: size,
    license,
    attribution,
    languages,
    url: `https://example.invalid/models/${name}`,
    installed: false,
    files: [{ name: `${name}.onnx`, url: `https://example.invalid/models/${name}.onnx`, size_bytes: size, sha256: "0".repeat(64) }],
  };
}

/** The models of ADR-0033 with their sizes and licenses, as the engine lists them on macOS. */
export const STAND_IN_MODELS: ModelInfo[] = [
  model("onnxruntime", "runtime", 36_936_435, "MIT", "ONNX Runtime by Microsoft, MIT license."),
  model(
    "birefnet-lite",
    "matte",
    224_005_088,
    "MIT",
    "BiRefNet by Peng Zheng, Dehong Gao, Deng-Ping Fan, Li Liu, Jorma Laaksonen, Wanli Ouyang and Nicu Sebe, MIT license.",
  ),
  model(
    "mediapipe-pose-full",
    "pose",
    24_691_287,
    "Apache-2.0",
    "MediaPipe Pose Landmarker (BlazePose GHUM) by Google, converted to ONNX by FossCut, Apache-2.0.",
  ),
  model(
    "kokoro-82m",
    "voice",
    96_400_000,
    "Apache-2.0 AND BSD-2-Clause",
    "Kokoro-82M by hexgrad, Apache-2.0; ONNX export by onnx-community. Pronunciations from the CMU Pronouncing Dictionary, Copyright (C) 1993-2015 Carnegie Mellon University, BSD-2-Clause.",
    "en",
  ),
  model(
    "opus-mt-en-hu",
    "translate",
    120_031_000,
    "Apache-2.0",
    "OPUS-MT en-hu by Jörg Tiedemann and the Language Technology Research Group at the University of Helsinki, Apache-2.0; ONNX export by Xenova.",
    "en>hu",
  ),
  model(
    "opus-mt-hu-en",
    "translate",
    120_031_000,
    "Apache-2.0",
    "OPUS-MT hu-en by Jörg Tiedemann and the Language Technology Research Group at the University of Helsinki, Apache-2.0; ONNX export by Xenova.",
    "hu>en",
  ),
];

/** A listing with the models in `installed` installed (the runtime too when it is among them). */
export function standInListing(installed: string[]): ModelListing {
  return {
    dir: "/Users/demo/Library/Caches/fosscut/models",
    models: STAND_IN_MODELS.map((m) => ({ ...m, installed: installed.includes(m.name) })),
    runtime: installed.includes("onnxruntime") ? "/Users/demo/Library/Caches/fosscut/models/onnxruntime/libonnxruntime.dylib" : null,
  };
}

/** Some of the voice model's voices. */
export const STAND_IN_VOICES: VoiceInfo[] = [
  { name: "af_heart", accent: "american", gender: "female", installed: true },
  { name: "af_bella", accent: "american", gender: "female", installed: true },
  { name: "am_adam", accent: "american", gender: "male", installed: true },
  { name: "am_michael", accent: "american", gender: "male", installed: true },
  { name: "bf_emma", accent: "british", gender: "female", installed: true },
  { name: "bm_george", accent: "british", gender: "male", installed: true },
];
