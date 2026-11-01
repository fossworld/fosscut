//! Models for analyzers and effects that need one (ADR-0019 item 5, ADR-0020 item 2,
//! ADR-0033 item 2). Manifests are compiled in: each model is a name, a task, a license with
//! its attribution, and files with their address, size and SHA-256. A model installs only
//! when asked, downloads with the system `curl` (or is copied from local files), and every
//! file is verified before it is used.
//!
//! The speech models of ADR-0020 are single files in the models folder itself. Every other
//! model is a folder named after it, which appears only once all of its files are verified.
//! The `onnxruntime` model picks Microsoft's archive for the running system and unpacks its
//! library with the system `tar`, which ships with macOS, Linux and Windows 10 and later.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// An operating system and processor, named as `std::env::consts` names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Platform {
    /// `macos`, `linux` or `windows`.
    pub os: &'static str,
    /// `aarch64` or `x86_64`.
    pub arch: &'static str,
}

impl Platform {
    /// The system this build runs on.
    pub const CURRENT: Platform = Platform {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    };
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.os, self.arch)
    }
}

/// A file kept from an archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    /// Its path in the archive, exactly as the archive stores it.
    pub path: &'static str,
    /// Its name in the model's folder.
    pub name: &'static str,
    /// Exact size in bytes.
    pub size: u64,
    /// SHA-256, lowercase hex.
    pub sha256: &'static str,
}

/// One file of a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelFile {
    /// Where it downloads from.
    pub address: &'static str,
    /// Its name in the model's folder, with `/` for a subfolder (`voices/af_heart.bin`); for
    /// an archive, the name it is downloaded as.
    pub name: &'static str,
    /// Exact size in bytes.
    pub size: u64,
    /// SHA-256, lowercase hex.
    pub sha256: &'static str,
    /// The only system it is for; `None` when it is for every system.
    pub platform: Option<Platform>,
    /// For an archive (`.tgz` or `.zip`), the members to keep; the archive itself is removed
    /// once they are unpacked and verified. Empty for a plain file.
    pub unpack: &'static [Member],
}

/// Where a model's files are installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Its one file sits in the models folder itself, as the speech models of ADR-0020 do.
    File,
    /// Its files sit in a folder named after the model.
    Folder,
}

/// A model FossCut can install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// Stable name, for example `whisper-base.en`.
    pub name: &'static str,
    /// What it does: `speech`, `matte`, `pose`, `voice`, `translate`, `runtime` for the
    /// ONNX Runtime library the vision and voice models run on, or `browser` for the headless
    /// browser web compositions render in (ADR-0039).
    pub task: &'static str,
    /// SPDX license of the weights (or of the library).
    pub license: &'static str,
    /// The credit to show wherever the model is listed and in an export's rights notes;
    /// licenses such as CC-BY-4.0 require it.
    pub attribution: Option<&'static str>,
    /// `en` for English-only models, `multilingual`, a translation direction such as `hu>en`,
    /// or `none` for models that do not read language.
    pub languages: &'static str,
    /// Where its files are installed.
    pub layout: Layout,
    /// Its files, for every system or for one each.
    pub files: &'static [ModelFile],
    /// What a user or agent should know before installing it, if anything.
    pub note: Option<&'static str>,
}

impl Manifest {
    /// Its files for `platform`: those for every system and those for that one.
    pub fn files_for(&self, platform: Platform) -> Vec<&'static ModelFile> {
        self.files
            .iter()
            .filter(|f| f.platform.is_none_or(|p| p == platform))
            .collect()
    }

    /// Its files for this system.
    pub fn files(&self) -> Vec<&'static ModelFile> {
        self.files_for(Platform::CURRENT)
    }

    /// Download size in bytes on this system.
    pub fn size(&self) -> u64 {
        self.files().iter().map(|f| f.size).sum()
    }
}

/// A plain file in a model's folder.
const fn file(
    address: &'static str,
    name: &'static str,
    size: u64,
    sha256: &'static str,
) -> ModelFile {
    ModelFile {
        address,
        name,
        size,
        sha256,
        platform: None,
        unpack: &[],
    }
}

/// A Microsoft ONNX Runtime archive for one system, keeping its library, license and notices.
const fn runtime(
    address: &'static str,
    name: &'static str,
    size: u64,
    sha256: &'static str,
    platform: Platform,
    unpack: &'static [Member],
) -> ModelFile {
    ModelFile {
        address,
        name,
        size,
        sha256,
        platform: Some(platform),
        unpack,
    }
}

/// The ONNX Runtime library's file name on this system, in the `onnxruntime` model's folder
/// and beside an executable.
pub const RUNTIME_LIBRARY: &str = if cfg!(target_os = "macos") {
    "libonnxruntime.dylib"
} else if cfg!(windows) {
    "onnxruntime.dll"
} else {
    "libonnxruntime.so"
};

macro_rules! kokoro {
    ($path:literal, $name:literal, $size:literal, $sha256:literal) => {
        file(
            concat!(
                "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/1939ad2a8e416c0acfeecc08a694d14ef25f2231/",
                $path
            ),
            $name,
            $size,
            $sha256,
        )
    };
    ($voice:literal, $sha256:literal) => {
        file(
            concat!(
                "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/1939ad2a8e416c0acfeecc08a694d14ef25f2231/voices/",
                $voice,
                ".bin"
            ),
            concat!("voices/", $voice, ".bin"),
            522_240,
            $sha256,
        )
    };
}

macro_rules! opus_en_hu {
    ($path:literal, $name:literal, $size:literal, $sha256:literal) => {
        file(
            concat!(
                "https://huggingface.co/Xenova/opus-mt-en-hu/resolve/1b7df958c1bd8f094eff1ce76d536468923ed7e3/",
                $path
            ),
            $name,
            $size,
            $sha256,
        )
    };
}

macro_rules! opus_hu_en {
    ($path:literal, $name:literal, $size:literal, $sha256:literal) => {
        file(
            concat!(
                "https://huggingface.co/Xenova/opus-mt-hu-en/resolve/c9e8a7617fb4cb4fca3785c5bb22cfe457cde00b/",
                $path
            ),
            $name,
            $size,
            $sha256,
        )
    };
}

const MACOS_ARM: Platform = Platform {
    os: "macos",
    arch: "aarch64",
};
const MACOS_INTEL: Platform = Platform {
    os: "macos",
    arch: "x86_64",
};
const LINUX_X64: Platform = Platform {
    os: "linux",
    arch: "x86_64",
};
const WINDOWS_X64: Platform = Platform {
    os: "windows",
    arch: "x86_64",
};

/// Every model; the speech models first, most capable first.
pub const MODELS: &[Manifest] = &[
    Manifest {
        name: "whisper-large-v3-turbo",
        task: "speech",
        license: "MIT",
        attribution: None,
        languages: "multilingual",
        layout: Layout::File,
        files: &[file(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
            "ggml-large-v3-turbo.bin",
            1_624_555_275,
            "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        )],
        note: None,
    },
    Manifest {
        name: "whisper-small",
        task: "speech",
        license: "MIT",
        attribution: None,
        languages: "multilingual",
        layout: Layout::File,
        files: &[file(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
            "ggml-small.bin",
            487_601_967,
            "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        )],
        note: None,
    },
    Manifest {
        name: "whisper-base.en",
        task: "speech",
        license: "MIT",
        attribution: None,
        languages: "en",
        layout: Layout::File,
        files: &[file(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
            "ggml-base.en.bin",
            147_964_211,
            "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        )],
        note: None,
    },
    Manifest {
        name: "whisper-tiny.en",
        task: "speech",
        license: "MIT",
        attribution: None,
        languages: "en",
        layout: Layout::File,
        files: &[file(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin",
            "ggml-tiny.en.bin",
            77_704_715,
            "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        )],
        note: None,
    },
    // Microsoft's ONNX Runtime 1.31.1 release, and 1.23.2 for Intel Macs, the last release
    // with a build for them. The archives are verified, then only the library, the license
    // and the third-party notices are kept.
    Manifest {
        name: "onnxruntime",
        task: "runtime",
        license: "MIT",
        attribution: Some("ONNX Runtime by Microsoft, MIT license."),
        languages: "none",
        layout: Layout::Folder,
        files: &[
            runtime(
                "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-osx-arm64-1.31.1.tgz",
                "onnxruntime-osx-arm64-1.31.1.tgz",
                36_936_435,
                "2e1344a1926ee50eb25b38153cb6992c3567515aef0e677a1a520a1798896d6f",
                MACOS_ARM,
                &[
                    Member {
                        path: "onnxruntime-osx-arm64-1.31.1/lib/libonnxruntime.1.31.1.dylib",
                        name: "libonnxruntime.dylib",
                        size: 44_797_256,
                        sha256: "17500d6b1e37e70f43a2c57d94233a1bc29fbcb297be0a5f8c7c13184f3307ba",
                    },
                    Member {
                        path: "onnxruntime-osx-arm64-1.31.1/LICENSE",
                        name: "LICENSE",
                        size: 1_073,
                        sha256: "2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c",
                    },
                    Member {
                        path: "onnxruntime-osx-arm64-1.31.1/ThirdPartyNotices.txt",
                        name: "ThirdPartyNotices.txt",
                        size: 338_088,
                        sha256: "143764b952fdb1a7c69ce653bfba74a7744d6a8a573bfb73e235fba356c83de3",
                    },
                ],
            ),
            runtime(
                "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-osx-x86_64-1.23.2.tgz",
                "onnxruntime-osx-x86_64-1.23.2.tgz",
                11_676_322,
                "d10359e16347b57d9959f7e80a225a5b4a66ed7d7e007274a15cae86836485a6",
                MACOS_INTEL,
                &[
                    Member {
                        path: "./onnxruntime-osx-x86_64-1.23.2/lib/libonnxruntime.1.23.2.dylib",
                        name: "libonnxruntime.dylib",
                        size: 39_742_608,
                        sha256: "8c9c78de65ea3786f987c0d980e9c1b13a3a5fbc6b3e2965ba05b450e6e4c054",
                    },
                    Member {
                        path: "./onnxruntime-osx-x86_64-1.23.2/LICENSE",
                        name: "LICENSE",
                        size: 1_073,
                        sha256: "2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c",
                    },
                    Member {
                        path: "./onnxruntime-osx-x86_64-1.23.2/ThirdPartyNotices.txt",
                        name: "ThirdPartyNotices.txt",
                        size: 326_866,
                        sha256: "e9e90971a8e75a9a8ac0c6412e29c1202d079998389915aa485f46c816c3b4cc",
                    },
                ],
            ),
            runtime(
                "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-linux-x64-1.31.1.tgz",
                "onnxruntime-linux-x64-1.31.1.tgz",
                11_413_655,
                "c849e36a1ef3e22d4ff217b9d1ce4a3cf5b9467b975610913ae959ad5b16bdc9",
                LINUX_X64,
                &[
                    Member {
                        path: "onnxruntime-linux-x64-1.31.1/lib/libonnxruntime.so.1.31.1",
                        name: "libonnxruntime.so",
                        size: 29_345_384,
                        sha256: "a96f132e5eb5ca04e23a7d2fc1078fb95aa591e8d71d0f237702ece3a1e0f4ea",
                    },
                    Member {
                        path: "onnxruntime-linux-x64-1.31.1/LICENSE",
                        name: "LICENSE",
                        size: 1_073,
                        sha256: "2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c",
                    },
                    Member {
                        path: "onnxruntime-linux-x64-1.31.1/ThirdPartyNotices.txt",
                        name: "ThirdPartyNotices.txt",
                        size: 338_088,
                        sha256: "143764b952fdb1a7c69ce653bfba74a7744d6a8a573bfb73e235fba356c83de3",
                    },
                ],
            ),
            runtime(
                "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-win-x64-1.31.1.zip",
                "onnxruntime-win-x64-1.31.1.zip",
                82_929_944,
                "5869e26bb25314375f2d5ef1877e3ea06b1543d6c710e6e4a1ec2f7177ab4b2c",
                WINDOWS_X64,
                &[
                    Member {
                        path: "onnxruntime-win-x64-1.31.1/lib/onnxruntime.dll",
                        name: "onnxruntime.dll",
                        size: 17_253_728,
                        sha256: "cb2fa518b18356617f6f9f460c012501da2819ec7e5c1d80bd8f5de20492db0c",
                    },
                    Member {
                        path: "onnxruntime-win-x64-1.31.1/LICENSE",
                        name: "LICENSE",
                        size: 1_094,
                        sha256: "c250d6278f0b47a6439fb7592b08b58a55eb9f535aa49a1db63211c3f982b674",
                    },
                    Member {
                        path: "onnxruntime-win-x64-1.31.1/ThirdPartyNotices.txt",
                        name: "ThirdPartyNotices.txt",
                        size: 344_457,
                        sha256: "c53a76501ef60db6f865f20599f220761201ac4057683acefdab37d861b86622",
                    },
                ],
            ),
        ],
        note: Some(
            "The library the matte, pose, voice and translation models run on; install it before them.",
        ),
    },
    // The general BiRefNet_lite (Swin-T backbone) from the authors' v1 release; the same
    // bytes are published as onnx-community/BiRefNet_lite-ONNX onnx/model.onnx (fp32).
    Manifest {
        name: "birefnet-lite",
        task: "matte",
        license: "MIT",
        attribution: Some(
            "BiRefNet by Peng Zheng, Dehong Gao, Deng-Ping Fan, Li Liu, Jorma Laaksonen, Wanli Ouyang and Nicu Sebe, MIT license.",
        ),
        languages: "none",
        layout: Layout::Folder,
        files: &[file(
            "https://github.com/ZhengPeng7/BiRefNet/releases/download/v1/BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx",
            "model.onnx",
            224_005_088,
            "5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333",
        )],
        note: None,
    },
    // Google's Pose Landmarker full (`pose_landmarker_full.task`, float16, version 1,
    // SHA-256 5134a3aad27a58b93da0088d431f366da362b44e3ccfbe3462b3827a839011b1), converted to
    // ONNX by FossCut with the scripts in scripts/models/ and published as assets of the
    // `models-v1` release; scripts/models/README.md records the steps and the checks.
    Manifest {
        name: "mediapipe-pose-full",
        task: "pose",
        license: "Apache-2.0",
        attribution: Some(
            "MediaPipe Pose Landmarker (BlazePose GHUM) by Google, converted to ONNX by FossCut, Apache-2.0.",
        ),
        languages: "none",
        layout: Layout::Folder,
        files: &[
            file(
                "https://github.com/fossworld/fosscut/releases/download/models-v1/mediapipe-pose-full-detector.onnx",
                "detector.onnx",
                11_925_808,
                "d36c4aecbfea4ed0a9a5cfb1905569437f7e4b777f5921d60e5dc5d4f154e323",
            ),
            file(
                "https://github.com/fossworld/fosscut/releases/download/models-v1/mediapipe-pose-full-landmarks.onnx",
                "landmarks.onnx",
                12_765_479,
                "f29febba853796d52f94948632f138c53c617deb09f8539d31b1a4a14b023260",
            ),
        ],
        note: Some(
            "Converted from Google's TFLite release by FossCut; scripts/models/README.md in the FossCut repository shows how to repeat the conversion and check it.",
        ),
    },
    // Kokoro-82M v1.0, 8-bit quantized, with its American (a) and British (b) English voices,
    // and the CMU Pronouncing Dictionary (BSD-2-Clause) that English text is pronounced with,
    // from a pinned commit of Carnegie Mellon's repository, with its license.
    Manifest {
        name: "kokoro-82m",
        task: "voice",
        license: "Apache-2.0 AND BSD-2-Clause",
        attribution: Some(
            "Kokoro-82M by hexgrad, Apache-2.0; ONNX export by onnx-community. Pronunciations from the CMU Pronouncing Dictionary, Copyright (C) 1993-2015 Carnegie Mellon University, BSD-2-Clause.",
        ),
        languages: "en",
        layout: Layout::Folder,
        files: &[
            kokoro!(
                "onnx/model_quantized.onnx",
                "model.onnx",
                92_361_116,
                "fbae9257e1e05ffc727e951ef9b9c98418e6d79f1c9b6b13bd59f5c9028a1478"
            ),
            kokoro!(
                "tokenizer.json",
                "tokenizer.json",
                3_497,
                "77a02c8e164413299b4b4c403b14f8e0e1c1b727db4d46a09d6327b861060a34"
            ),
            file(
                "https://raw.githubusercontent.com/cmusphinx/cmudict/74790861f652b15e4ac49015a90074ad62a27690/cmudict.dict",
                "cmudict/cmudict.dict",
                3_618_488,
                "81917843c7f44ce2b094ac63873c2c7a4cf802040792c455ba3ca406891c3d22",
            ),
            file(
                "https://raw.githubusercontent.com/cmusphinx/cmudict/74790861f652b15e4ac49015a90074ad62a27690/LICENSE",
                "cmudict/LICENSE",
                1_754,
                "bd4ce8e44170a5f9f481310ca85c51de3c4f851a65e679b40e603b143bd3542a",
            ),
            kokoro!(
                "config.json",
                "config.json",
                44,
                "df34b4f930b23447cd4dc410fabfb42eb3f24e803e6c3f97d618fb359380a36f"
            ),
            kokoro!(
                "af_alloy",
                "c4a6b876047fd7fb472edf4ebd63cfac7c3b958a7cae7c106e8f038ca6308c45"
            ),
            kokoro!(
                "af_aoede",
                "4a004c33430762e2461eedb2013fad808ef4ab3121f5300f554476caf58d8361"
            ),
            kokoro!(
                "af_bella",
                "f69d836209b78eb8c66e75e3cda491e26ea838a3674257e9d4e5703cbaf55c8b"
            ),
            kokoro!(
                "af_heart",
                "d583ccff3cdca2f7fae535cb998ac07e9fcb90f09737b9a41fa2734ec44a8f0b"
            ),
            kokoro!(
                "af_jessica",
                "a240a5e3c15b43563d6e923bdca8ef5613a23471d9b77653694012435df23bd8"
            ),
            kokoro!(
                "af_kore",
                "9be5221b6a941c04b561959b8ff0b06e809444dcc4ab7e75a7b23606f691819e"
            ),
            kokoro!(
                "af_nicole",
                "cd2191ab31b914ed7b318416b0e4440fdf392ddad9106a060819aa600a64f59a"
            ),
            kokoro!(
                "af_nova",
                "18778272caa0d0eebaea251c35fd635f038434f9eee5e691d02a174bd328414f"
            ),
            kokoro!(
                "af_river",
                "00a2bcf82b1d86e8f19902ede58c65ccf6c0e43b44b7d74fad54e5d8933c9c30"
            ),
            kokoro!(
                "af_sarah",
                "4409fbc125afabacc615d94db5398d847006a737b0247d6892b7a9a0007a2f0a"
            ),
            kokoro!(
                "af_sky",
                "4435255c9744f3f31659e0d714ab7689bf65d9e77ec1cce060f083912614f0b9"
            ),
            kokoro!(
                "am_adam",
                "162b035ed91cfc48b6046982184c645f72edcdd1b82843347f605d7bf7b15716"
            ),
            kokoro!(
                "am_echo",
                "3968b92c3c4cd1c4416dbded36c13eaa388a90d5788d02a13e4d781f5f8cf3c3"
            ),
            kokoro!(
                "am_eric",
                "e8b5be17edd1e3636901ce7598baafe2dc8dd8ff707a0c23bf9e461add7e2832"
            ),
            kokoro!(
                "am_fenrir",
                "c27989f741f7ee34d273a39d8a595cc0837d35f5ced9a29b7cc162614616df43"
            ),
            kokoro!(
                "am_liam",
                "52403be32fd047c6a44517cb0bcd6b134f2a18baa73e70ef41651e0eab921ade"
            ),
            kokoro!(
                "am_michael",
                "1d1f21dd8da39c30705cd4c75d039d265e9bc4a2a93ed09bc9e1b1225eb95ba1"
            ),
            kokoro!(
                "am_onyx",
                "da5d135b424164916d75a68ffb4c2abce3d7d5ccc82dd1ee6cf447ce286145e6"
            ),
            kokoro!(
                "am_puck",
                "fcf73c989033e9233e0b98713eca600c8c74dcc1614b37009d5450ff4a2274a0"
            ),
            kokoro!(
                "am_santa",
                "61150cf726ab6c5ed7a99f90a304f91f5a72c00c592e89ec94e5df11c319227a"
            ),
            kokoro!(
                "bf_alice",
                "08afa6ba24da61ea5e8efa139e5aadc938d83f0a6da5a900adaf763ac1da5573"
            ),
            kokoro!(
                "bf_emma",
                "669fe0647f9dd04fcab92f1439a40eeb4c8b4ab1f82e4996fe3d918ce4a63b73"
            ),
            kokoro!(
                "bf_isabella",
                "3754352c4aaa46d17f27654ab7518d65b62ad6163a0f55a5f4330c2da2c4e94f"
            ),
            kokoro!(
                "bf_lily",
                "5e0ee32ebe64a467124976b14e69590746f1c4ce41a12b587a50c862edfea335"
            ),
            kokoro!(
                "bm_daniel",
                "6b3194bbceffb746733cbc22c8f593dd44e401a71d53895a2dca891bc595a1e8"
            ),
            kokoro!(
                "bm_fable",
                "f889083196807b4adb15e9204252165f503b8d33d3982e681c52443c49d798f1"
            ),
            kokoro!(
                "bm_george",
                "c4b235a4c1f2cd3b939fed08b899ce9385638b763f7b73a59616c4fc9bd6c9bc"
            ),
            kokoro!(
                "bm_lewis",
                "b8f671cef828c30e66fdf0b0756a76bba58f6bb3398cbbf27058642acbcedb97"
            ),
        ],
        note: None,
    },
    // OPUS-MT Hungarian to English (Helsinki-NLP/opus-mt-hu-en), exported to ONNX by Xenova
    // with the same tokenizer files and vocabulary, 8-bit quantized.
    Manifest {
        name: "opus-mt-hu-en",
        task: "translate",
        license: "Apache-2.0",
        attribution: Some(
            "OPUS-MT hu-en by Jörg Tiedemann and the Language Technology Research Group at the University of Helsinki, Apache-2.0; ONNX export by Xenova.",
        ),
        languages: "hu>en",
        layout: Layout::Folder,
        files: &[
            opus_hu_en!(
                "onnx/encoder_model_quantized.onnx",
                "encoder.onnx",
                51_630_494,
                "f0dce998a12ce35d4880d5f6ac153807be88bb50691244e512e9605b2cabd6fe"
            ),
            opus_hu_en!(
                "onnx/decoder_model_merged_quantized.onnx",
                "decoder.onnx",
                58_933_640,
                "c8d9338328feecf6b21ca74a075a8fa6484c45b2fece2b96d2cf65c4cc7c31dd"
            ),
            opus_hu_en!(
                "source.spm",
                "source.spm",
                849_914,
                "e53f2bee88be1541f17929195532b2c77e5bbd6dfb69aeb0589d8fe457a2b6b0"
            ),
            opus_hu_en!(
                "target.spm",
                "target.spm",
                792_289,
                "13cc9fb7021c19589da645e49efe801ac51e9563db22da262156903ddb98cea6"
            ),
            opus_hu_en!(
                "vocab.json",
                "vocab.json",
                1_690_844,
                "a298c9bcefb63d18f091ef18d674a6b78a424b910aa6c0df1698dfbb3fb79252"
            ),
            opus_hu_en!(
                "tokenizer.json",
                "tokenizer.json",
                6_133_233,
                "05467c7d69c3755d66e02b34d58d9643d0b2e7c28d3e761ba6036d7c855326c5"
            ),
            opus_hu_en!(
                "config.json",
                "config.json",
                1_376,
                "5a93f022de4c82b858d12df9dfafeff11142c2861cfe5009c420060f9fdf7eb4"
            ),
            opus_hu_en!(
                "generation_config.json",
                "generation_config.json",
                293,
                "ef672fbdfe695658b92448bcb8cad696ca550d04f285b6dc6699598280fa1cb6"
            ),
        ],
        note: None,
    },
    // OPUS-MT English to Hungarian (Helsinki-NLP/opus-mt-en-hu), exported to ONNX by Xenova
    // with the same tokenizer files and vocabulary, 8-bit quantized. The newer
    // opus-mt-tc-big-en-hu has no ONNX export.
    Manifest {
        name: "opus-mt-en-hu",
        task: "translate",
        license: "Apache-2.0",
        attribution: Some(
            "OPUS-MT en-hu by Jörg Tiedemann and the Language Technology Research Group at the University of Helsinki, Apache-2.0; ONNX export by Xenova.",
        ),
        languages: "en>hu",
        layout: Layout::Folder,
        files: &[
            opus_en_hu!(
                "onnx/encoder_model_quantized.onnx",
                "encoder.onnx",
                51_630_494,
                "f78d4af0a91fc534b0d0267e85fc7164d326a7b8fb18f46472e2f70d443230af"
            ),
            opus_en_hu!(
                "onnx/decoder_model_merged_quantized.onnx",
                "decoder.onnx",
                58_933_640,
                "b9d9f644444b8cf89aa24e8c4347d247537a25d8d850919843082ce7f44ea9ee"
            ),
            opus_en_hu!(
                "source.spm",
                "source.spm",
                792_289,
                "13cc9fb7021c19589da645e49efe801ac51e9563db22da262156903ddb98cea6"
            ),
            opus_en_hu!(
                "target.spm",
                "target.spm",
                849_914,
                "e53f2bee88be1541f17929195532b2c77e5bbd6dfb69aeb0589d8fe457a2b6b0"
            ),
            opus_en_hu!(
                "vocab.json",
                "vocab.json",
                1_690_844,
                "a298c9bcefb63d18f091ef18d674a6b78a424b910aa6c0df1698dfbb3fb79252"
            ),
            opus_en_hu!(
                "tokenizer.json",
                "tokenizer.json",
                6_133_233,
                "05467c7d69c3755d66e02b34d58d9643d0b2e7c28d3e761ba6036d7c855326c5"
            ),
            opus_en_hu!(
                "config.json",
                "config.json",
                1_376,
                "83a2546555e427ab63c2afb161d5c4ff2e2ca4bc108c186750f7d2b7f3a4488b"
            ),
            opus_en_hu!(
                "generation_config.json",
                "generation_config.json",
                293,
                "22afa2568f868878a08a98968d15c4e5ee4fcf9a7c0bdb64186417e07faacb15"
            ),
        ],
        note: None,
    },
    // Chrome for Testing's headless shell 156.0.8078.80 for macOS (ADR-0039), the browser
    // web compositions render in when no Chrome, Chromium, Edge or Brave is installed. The
    // archives are verified, then every file is kept.
    Manifest {
        name: "chrome-headless-shell",
        task: "browser",
        license: "LicenseRef-Chrome-for-Testing",
        attribution: Some(
            "Chrome for Testing by Google LLC, built from the Chromium project (BSD-3-Clause) and other open source software listed in LICENSE.headless_shell.",
        ),
        languages: "none",
        layout: Layout::Folder,
        files: &[
            runtime(
                "https://storage.googleapis.com/chrome-for-testing-public/156.0.8078.80/mac-arm64/chrome-headless-shell-mac-arm64.zip",
                "chrome-headless-shell-mac-arm64.zip",
                103_982_159,
                "ea615baa2e9caf0d02d1898a6562b5f8fa9c35239707dae0f592178df5b986ac",
                MACOS_ARM,
                &[
                    Member {
                        path: "chrome-headless-shell-mac-arm64/ABOUT",
                        name: "ABOUT",
                        size: 248,
                        sha256: "34d078ce3003087a8374e7c6156fda374769b8047d6ddaf419d66414aa48edfb",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/LICENSE.headless_shell",
                        name: "LICENSE.headless_shell",
                        size: 2_151_339,
                        sha256: "a2196c2e406ebe900cd6403a033935ecc641a971b7f2a57bd8c24afa9574e492",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/chrome-headless-shell",
                        name: "chrome-headless-shell",
                        size: 177_094_416,
                        sha256: "febae66e849b516d551a7fd0434af84f7c7e688e93e43dbc1fb951bdc87ec869",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/headless_command_resources.pak",
                        name: "headless_command_resources.pak",
                        size: 3_109,
                        sha256: "4eecdc01a7158a78f68b6b20f1452586aa04d2ed2e1b56084c55d51ec3f31707",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/headless_lib_data.pak",
                        name: "headless_lib_data.pak",
                        size: 730_182,
                        sha256: "5085d8de3bfbab6fe8e8780d1675bc6d6a68830145d01bff6436b06b49f0e7a1",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/headless_lib_strings.pak",
                        name: "headless_lib_strings.pak",
                        size: 5_021_260,
                        sha256: "3f6372558b21217a82115582e5c413861724b7035bdfdf32104868f913b517ec",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/icudtl.dat",
                        name: "icudtl.dat",
                        size: 10_824_576,
                        sha256: "e1ed8f674bb8449731b79cae956042da59f9ecbd37c8d3f86586a22b4836e5b2",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/libEGL.dylib",
                        name: "libEGL.dylib",
                        size: 736,
                        sha256: "8b7e5b3ebed9cd5fc1cde7ff1458fd5eb150d496d7474e13f963d43546c08aeb",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/libGLESv2.dylib",
                        name: "libGLESv2.dylib",
                        size: 736,
                        sha256: "7765bf768e52ced295a67cb362e3ca9714e64208e6ecea4e67e5b4b85e094934",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/libvk_swiftshader.dylib",
                        name: "libvk_swiftshader.dylib",
                        size: 16_326_576,
                        sha256: "94d3ebe226450420923fd681511ff87b2f887c1fdc21f9e8ef8e93a2e48f73dd",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/libvulkan.dylib",
                        name: "libvulkan.dylib",
                        size: 499_488,
                        sha256: "eee9118541fe306c76ee84c95c69e781f57417a198cf14b2cfb581f34b2350f4",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/resources/accessibility/reading_mode_gdocs_helper/content.js",
                        name: "resources/accessibility/reading_mode_gdocs_helper/content.js",
                        size: 439,
                        sha256: "6c0aeaaf9a71ad68aa45764c111e70bb23aa0fbf406644a375b6b709cbbe4b6e",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/resources/accessibility/reading_mode_gdocs_helper/gdocs_script.js",
                        name: "resources/accessibility/reading_mode_gdocs_helper/gdocs_script.js",
                        size: 1_425,
                        sha256: "d291adabcfcb1a4a4a2ffacdd9d04fdb473ab0d6903556bef7446be9c3646d86",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/resources/accessibility/reading_mode_gdocs_helper_manifest.json",
                        name: "resources/accessibility/reading_mode_gdocs_helper_manifest.json",
                        size: 2_375,
                        sha256: "313b31982a5c4af013d11cde4883f27c11a63af76333dc0a3539dddf0d866094",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/v8_context_snapshot.arm64.bin",
                        name: "v8_context_snapshot.arm64.bin",
                        size: 753_112,
                        sha256: "413651d04ca0dab47bd125fe5d1f73ab43614575883ddf2a2a3d47cd660748da",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-arm64/vk_swiftshader_icd.json",
                        name: "vk_swiftshader_icd.json",
                        size: 110,
                        sha256: "d717d915e31e7c27948b80b36ab34e2d897888114c5c7d0af835f93eb53e58f5",
                    },
                ],
            ),
            runtime(
                "https://storage.googleapis.com/chrome-for-testing-public/156.0.8078.80/mac-x64/chrome-headless-shell-mac-x64.zip",
                "chrome-headless-shell-mac-x64.zip",
                109_493_298,
                "535d417d9034cbd11e7dacd1a71aa3d6e1b525e6d5dcd2671eb2532f01481228",
                MACOS_INTEL,
                &[
                    Member {
                        path: "chrome-headless-shell-mac-x64/ABOUT",
                        name: "ABOUT",
                        size: 248,
                        sha256: "34d078ce3003087a8374e7c6156fda374769b8047d6ddaf419d66414aa48edfb",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/LICENSE.headless_shell",
                        name: "LICENSE.headless_shell",
                        size: 2_151_339,
                        sha256: "a2196c2e406ebe900cd6403a033935ecc641a971b7f2a57bd8c24afa9574e492",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/chrome-headless-shell",
                        name: "chrome-headless-shell",
                        size: 193_103_440,
                        sha256: "ca5be1d3423afd143d241a86bab0a103c37ffef3dd2bb60dd14627e0a702e138",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/headless_command_resources.pak",
                        name: "headless_command_resources.pak",
                        size: 3_109,
                        sha256: "4eecdc01a7158a78f68b6b20f1452586aa04d2ed2e1b56084c55d51ec3f31707",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/headless_lib_data.pak",
                        name: "headless_lib_data.pak",
                        size: 730_182,
                        sha256: "5085d8de3bfbab6fe8e8780d1675bc6d6a68830145d01bff6436b06b49f0e7a1",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/headless_lib_strings.pak",
                        name: "headless_lib_strings.pak",
                        size: 5_021_260,
                        sha256: "3f6372558b21217a82115582e5c413861724b7035bdfdf32104868f913b517ec",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/icudtl.dat",
                        name: "icudtl.dat",
                        size: 10_824_576,
                        sha256: "e1ed8f674bb8449731b79cae956042da59f9ecbd37c8d3f86586a22b4836e5b2",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/libEGL.dylib",
                        name: "libEGL.dylib",
                        size: 552,
                        sha256: "71a00bc0c12b510e63364720f1cda4735863113017e5051ee3a952dfcc475c69",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/libGLESv2.dylib",
                        name: "libGLESv2.dylib",
                        size: 560,
                        sha256: "3a26faf71edbbbac22668b20c79ec787196084023e002be987fa5420179008de",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/libvk_swiftshader.dylib",
                        name: "libvk_swiftshader.dylib",
                        size: 4_420_128,
                        sha256: "299dc3613f44acce28d9e74cabc16a0fdd799783ae2f5adae15b7fd3fc3362e7",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/libvulkan.dylib",
                        name: "libvulkan.dylib",
                        size: 479_152,
                        sha256: "a2c904f5df8684d362c2896905b9c617cf5556810b7931ba26c4c93201c6acf3",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/resources/accessibility/reading_mode_gdocs_helper/content.js",
                        name: "resources/accessibility/reading_mode_gdocs_helper/content.js",
                        size: 439,
                        sha256: "6c0aeaaf9a71ad68aa45764c111e70bb23aa0fbf406644a375b6b709cbbe4b6e",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/resources/accessibility/reading_mode_gdocs_helper/gdocs_script.js",
                        name: "resources/accessibility/reading_mode_gdocs_helper/gdocs_script.js",
                        size: 1_425,
                        sha256: "d291adabcfcb1a4a4a2ffacdd9d04fdb473ab0d6903556bef7446be9c3646d86",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/resources/accessibility/reading_mode_gdocs_helper_manifest.json",
                        name: "resources/accessibility/reading_mode_gdocs_helper_manifest.json",
                        size: 2_375,
                        sha256: "313b31982a5c4af013d11cde4883f27c11a63af76333dc0a3539dddf0d866094",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/v8_context_snapshot.x86_64.bin",
                        name: "v8_context_snapshot.x86_64.bin",
                        size: 753_112,
                        sha256: "5e448d35ca437b8248e7eb866b002003e09f2e48d73cd678caf11bb10c1b5d89",
                    },
                    Member {
                        path: "chrome-headless-shell-mac-x64/vk_swiftshader_icd.json",
                        name: "vk_swiftshader_icd.json",
                        size: 110,
                        sha256: "d717d915e31e7c27948b80b36ab34e2d897888114c5c7d0af835f93eb53e58f5",
                    },
                ],
            ),
        ],
        note: Some(
            "Google's headless Chrome for rendering web compositions, distributed by Google under the Google Chrome Terms of Service; install it only when no Chrome, Chromium, Edge or Brave is installed.",
        ),
    },
];

/// A model's file as front ends list it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileInfo {
    /// Its name in the model's folder, or the archive's name.
    pub name: &'static str,
    /// Where it downloads from.
    pub url: &'static str,
    /// Download size in bytes.
    pub size_bytes: u64,
    /// SHA-256, lowercase hex.
    pub sha256: &'static str,
}

/// A model as front ends list it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelInfo {
    /// Stable name.
    pub name: &'static str,
    /// What it does.
    pub task: &'static str,
    /// Download size in bytes on this system.
    pub size_bytes: u64,
    /// SPDX license.
    pub license: &'static str,
    /// The credit to show with it, if its license asks for one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<&'static str>,
    /// `en`, `multilingual`, a direction such as `hu>en`, or `none`.
    pub languages: &'static str,
    /// Where its first file downloads from on this system; empty when it has no build for
    /// this system.
    pub url: String,
    /// Whether it is installed in the models folder.
    pub installed: bool,
    /// The files it downloads on this system.
    pub files: Vec<FileInfo>,
    /// What to know before installing it, if anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
}

/// Why a model could not be found, chosen or installed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelError {
    /// No model has this name.
    #[error("there is no model {name}")]
    Unknown {
        /// The requested name.
        name: String,
    },
    /// The model, or any model for the task and language, is not installed.
    #[error("{what} is not installed")]
    NotInstalled {
        /// The model or kind of model.
        what: String,
    },
    /// The file does not match the manifest.
    #[error("{name} does not match its checksum: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// The model, and for a model of several files, the file (`kokoro-82m/model.onnx`).
        name: String,
        /// The manifest's SHA-256 (or size).
        expected: String,
        /// What the file has.
        actual: String,
    },
    /// The download, copy or unpacking failed.
    #[error("cannot install {name}: {message}")]
    DownloadFailed {
        /// The model.
        name: String,
        /// What went wrong.
        message: String,
    },
    /// The model has no files for this system.
    #[error("{name} has no build for {platform}")]
    Unsupported {
        /// The model.
        name: String,
        /// The system, for example `linux aarch64`.
        platform: String,
    },
}

/// The manifest called `name`.
pub fn manifest(name: &str) -> Result<&'static Manifest, ModelError> {
    MODELS
        .iter()
        .find(|m| m.name == name)
        .ok_or_else(|| ModelError::Unknown { name: name.into() })
}

/// Where `model` lives in `dir`: its file for a single-file speech model, otherwise its
/// folder.
pub fn path(dir: &Path, model: &Manifest) -> PathBuf {
    match (model.layout, model.files.first()) {
        (Layout::File, Some(file)) => dir.join(file.name),
        _ => dir.join(model.name),
    }
}

/// Where the installed file `name` of `model` is in `dir`, for example
/// `file_path(dir, kokoro, "voices/af_heart.bin")`.
pub fn file_path(dir: &Path, model: &Manifest, name: &str) -> PathBuf {
    match model.layout {
        Layout::File => dir.join(name),
        Layout::Folder => dir.join(model.name).join(name),
    }
}

/// The download address of `model`'s first file on this system; empty when it has no build
/// for this system.
pub fn url(model: &Manifest) -> String {
    model
        .files()
        .first()
        .map(|f| f.address)
        .unwrap_or_default()
        .to_string()
}

/// The files installing `file` leaves: itself, or the members unpacked from it.
fn kept(file: &ModelFile) -> Vec<(&'static str, u64)> {
    if file.unpack.is_empty() {
        vec![(file.name, file.size)]
    } else {
        file.unpack.iter().map(|m| (m.name, m.size)).collect()
    }
}

/// Whether `model` is installed in `dir`: every file it keeps on this system is present with
/// the manifest's size. Checksums are verified once, when it is installed.
pub fn is_installed(dir: &Path, model: &Manifest) -> bool {
    let files = model.files();
    !files.is_empty()
        && files.iter().flat_map(|f| kept(f)).all(|(name, size)| {
            std::fs::metadata(file_path(dir, model, name)).is_ok_and(|m| m.len() == size)
        })
}

/// `model` as front ends list it, with whether it is installed in `dir`.
pub fn info(dir: &Path, model: &Manifest) -> ModelInfo {
    ModelInfo {
        name: model.name,
        task: model.task,
        size_bytes: model.size(),
        license: model.license,
        attribution: model.attribution,
        languages: model.languages,
        url: url(model),
        installed: is_installed(dir, model),
        files: model
            .files()
            .into_iter()
            .map(|f| FileInfo {
                name: f.name,
                url: f.address,
                size_bytes: f.size,
                sha256: f.sha256,
            })
            .collect(),
        note: model.note,
    }
}

/// Every model with whether it is installed in `dir`.
pub fn list(dir: &Path) -> Vec<ModelInfo> {
    MODELS.iter().map(|m| info(dir, m)).collect()
}

/// The most capable installed speech model for `language` (any language when `None`);
/// English-only models qualify only for English.
pub fn best_speech_model(dir: &Path, language: Option<&str>) -> Option<&'static Manifest> {
    MODELS.iter().find(|m| {
        m.task == "speech"
            && (m.languages == "multilingual"
                || matches!(language, Some("en"))
                || language.is_none())
            && is_installed(dir, m)
    })
}

/// Where to install from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The manifest's addresses, fetched with `curl`. Another address may be given (`file://`
    /// works too): the file's own address for a model of one file, or the address of a
    /// folder that holds the files by name for a model of several.
    Download(Option<String>),
    /// Files already on disk: the file itself for a model of one file, or a folder that
    /// holds the files by name (`voices/af_heart.bin`, or the archive for `onnxruntime`).
    File(PathBuf),
}

/// Whether `model` can be installed on this system, before anything is asked or fetched:
/// it needs files for this system.
pub fn installable(model: &Manifest) -> Result<(), ModelError> {
    if model.files().is_empty() {
        return Err(ModelError::Unsupported {
            name: model.name.into(),
            platform: Platform::CURRENT.to_string(),
        });
    }
    Ok(())
}

/// Installs `model` into `dir` from `source`. Each file is written to a `.partial` file,
/// checked against the manifest's size and SHA-256, and archives are unpacked and their
/// members checked too. A model of several files is assembled in `<name>.partial` and moved
/// into place only when every file is verified, replacing an older copy; nothing partial is
/// left when anything fails. Returns the installed path, as [`path`] gives it.
pub fn install(dir: &Path, model: &Manifest, source: &Source) -> Result<PathBuf, ModelError> {
    let failed = |message: String| ModelError::DownloadFailed {
        name: model.name.into(),
        message,
    };
    installable(model)?;
    let files = model.files();
    std::fs::create_dir_all(dir)
        .map_err(|e| failed(format!("cannot create {}: {e}", dir.display())))?;
    match model.layout {
        Layout::File => {
            let file = files[0];
            let target = dir.join(file.name);
            let partial = dir.join(format!("{}.partial", file.name));
            let _ = std::fs::remove_file(&partial);
            let result = fetch(model, file, files.len(), source, &partial)
                .and_then(|()| verify(&partial, model.name, file.size, file.sha256));
            if let Err(error) = result {
                let _ = std::fs::remove_file(&partial);
                return Err(error);
            }
            std::fs::rename(&partial, &target)
                .map_err(|e| failed(format!("cannot move into place: {e}")))?;
            Ok(target)
        }
        Layout::Folder => {
            let target = dir.join(model.name);
            let staging = dir.join(format!("{}.partial", model.name));
            let _ = std::fs::remove_dir_all(&staging);
            let result = assemble(&staging, model, &files, source).and_then(|()| {
                if target.exists() {
                    std::fs::remove_dir_all(&target)
                        .map_err(|e| failed(format!("cannot replace {}: {e}", target.display())))?;
                }
                std::fs::rename(&staging, &target)
                    .map_err(|e| failed(format!("cannot move into place: {e}")))
            });
            if let Err(error) = result {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(error);
            }
            Ok(target)
        }
    }
}

/// Fetches, verifies and unpacks every file of `model` into `staging`.
fn assemble(
    staging: &Path,
    model: &Manifest,
    files: &[&ModelFile],
    source: &Source,
) -> Result<(), ModelError> {
    let failed = |message: String| ModelError::DownloadFailed {
        name: model.name.into(),
        message,
    };
    for file in files {
        let target = staging.join(file.name);
        let partial = staging.join(format!("{}.partial", file.name));
        if let Some(parent) = partial.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| failed(format!("cannot create {}: {e}", parent.display())))?;
        }
        fetch(model, file, files.len(), source, &partial)?;
        let name = format!("{}/{}", model.name, file.name);
        verify(&partial, &name, file.size, file.sha256)?;
        if file.unpack.is_empty() {
            std::fs::rename(&partial, &target)
                .map_err(|e| failed(format!("cannot move {}: {e}", file.name)))?;
        } else {
            unpack(model, &partial, staging, file.unpack)?;
            std::fs::remove_file(&partial)
                .map_err(|e| failed(format!("cannot remove {}: {e}", file.name)))?;
        }
    }
    Ok(())
}

/// Writes `file` of `model` (one of `count` files) from `source` to `out`.
fn fetch(
    model: &Manifest,
    file: &ModelFile,
    count: usize,
    source: &Source,
    out: &Path,
) -> Result<(), ModelError> {
    let failed = |message: String| ModelError::DownloadFailed {
        name: model.name.into(),
        message,
    };
    match source {
        Source::Download(address) => {
            let address = match address {
                Some(address) if count == 1 => address.clone(),
                Some(folder) => format!("{}/{}", folder.trim_end_matches('/'), file.name),
                None => file.address.to_string(),
            };
            let output = Command::new("curl")
                .args([
                    "--fail",
                    "--location",
                    "--silent",
                    "--show-error",
                    "--output",
                ])
                .arg(out)
                .arg(&address)
                .output()
                .map_err(|e| failed(format!("cannot run curl: {e}")))?;
            if !output.status.success() {
                let _ = std::fs::remove_file(out);
                return Err(failed(
                    String::from_utf8_lossy(&output.stderr).trim().to_string(),
                ));
            }
        }
        Source::File(path) => {
            let from = if path.is_dir() {
                path.join(file.name)
            } else if count == 1 {
                path.clone()
            } else {
                return Err(failed(format!(
                    "it has {count} files; pass the folder that holds them"
                )));
            };
            std::fs::copy(&from, out)
                .map_err(|e| failed(format!("cannot copy {}: {e}", from.display())))?;
        }
    }
    Ok(())
}

/// The `tar` to unpack archives with. On Windows that is the system's own (bsdtar), which
/// reads zip archives; another `tar` earlier on the path, such as Git's, may not.
fn tar() -> PathBuf {
    if cfg!(windows)
        && let Some(root) = std::env::var_os("SystemRoot")
    {
        let system = PathBuf::from(root).join("System32").join("tar.exe");
        if system.is_file() {
            return system;
        }
    }
    PathBuf::from("tar")
}

/// Unpacks `members` of the verified `archive` into `into`, checking each.
fn unpack(
    model: &Manifest,
    archive: &Path,
    into: &Path,
    members: &[Member],
) -> Result<(), ModelError> {
    let failed = |message: String| ModelError::DownloadFailed {
        name: model.name.into(),
        message,
    };
    let work = into.join(".unpack");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)
        .map_err(|e| failed(format!("cannot create {}: {e}", work.display())))?;
    let output = Command::new(tar())
        .arg("-x")
        .arg("-f")
        .arg(archive)
        .arg("-C")
        .arg(&work)
        .args(members.iter().map(|m| m.path))
        .output()
        .map_err(|e| failed(format!("cannot run tar: {e}")))?;
    if !output.status.success() {
        return Err(failed(format!(
            "cannot unpack {}: {}",
            archive.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    for member in members {
        let from = work.join(member.path);
        verify(
            &from,
            &format!("{}/{}", model.name, member.name),
            member.size,
            member.sha256,
        )?;
        let to = into.join(member.name);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| failed(format!("cannot create {}: {e}", parent.display())))?;
        }
        std::fs::rename(&from, &to)
            .map_err(|e| failed(format!("cannot move {}: {e}", member.name)))?;
    }
    std::fs::remove_dir_all(&work)
        .map_err(|e| failed(format!("cannot remove {}: {e}", work.display())))
}

/// Checks `file` against a manifest's size and SHA-256; `name` names it in errors.
fn verify(file: &Path, name: &str, size: u64, sha256: &str) -> Result<(), ModelError> {
    let mismatch = |expected: String, actual: String| ModelError::ChecksumMismatch {
        name: name.into(),
        expected,
        actual,
    };
    let actual_size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    if actual_size != size {
        return Err(mismatch(
            format!("{size} bytes"),
            format!("{actual_size} bytes"),
        ));
    }
    let unreadable = |e: std::io::Error| ModelError::DownloadFailed {
        name: name.into(),
        message: e.to_string(),
    };
    let mut reader = std::fs::File::open(file).map_err(unreadable)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = reader.read(&mut buffer).map_err(unreadable)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != sha256 {
        return Err(mismatch(sha256.into(), actual));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fosscut-models-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A file of `size` bytes that takes no disk space.
    fn sparse(path: &Path, size: u64) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::File::create(path).unwrap().set_len(size).unwrap();
    }

    /// SHA-256 of `bytes`, lowercase hex.
    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// A small single-file model whose file is the five bytes `hello`.
    const HELLO: Manifest = Manifest {
        name: "hello",
        task: "speech",
        license: "MIT",
        attribution: None,
        languages: "en",
        layout: Layout::File,
        files: &[file(
            "https://example.invalid/hello.bin",
            "hello.bin",
            5,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        )],
        note: None,
    };

    /// A model of two files in a folder, one of them in a subfolder: `hello` and `world`.
    const PAIR: Manifest = Manifest {
        name: "pair",
        task: "voice",
        license: "Apache-2.0",
        attribution: Some("Pair by the FossCut tests."),
        languages: "en",
        layout: Layout::Folder,
        files: &[
            file(
                "https://example.invalid/model.bin",
                "model.bin",
                5,
                "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
            ),
            file(
                "https://example.invalid/voices/a.bin",
                "voices/a.bin",
                5,
                "486ea46224d1bb4fb680f34f7c9ad96a8f24ec88be73ea8e5a6c65260e9cb8a7",
            ),
        ],
        note: None,
    };

    /// `path` as a `file://` address for curl.
    fn file_url(path: &Path) -> String {
        let path = path.display().to_string().replace('\\', "/");
        if path.starts_with('/') {
            format!("file://{path}")
        } else {
            format!("file:///{path}")
        }
    }

    fn is_hex(text: &str) -> bool {
        text.len() == 64 && text.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
    }

    fn is_relative(name: &str) -> bool {
        !name.is_empty()
            && !name.starts_with('/')
            && !name.contains('\\')
            && !name.contains(':')
            && name.split('/').all(|part| !part.is_empty() && part != "..")
    }

    #[test]
    fn manifests_are_complete() {
        let mut names: Vec<&str> = MODELS.iter().map(|m| m.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MODELS.len());
        let tasks = [
            "speech",
            "matte",
            "pose",
            "voice",
            "translate",
            "runtime",
            "browser",
        ];
        let licenses = [
            "MIT",
            "Apache-2.0",
            "CC-BY-4.0",
            // Kokoro's weights with the CMU Pronouncing Dictionary.
            "Apache-2.0 AND BSD-2-Clause",
            // Google's distribution of Chromium's headless shell (ADR-0039).
            "LicenseRef-Chrome-for-Testing",
        ];
        for model in MODELS {
            assert_eq!(manifest(model.name), Ok(model));
            assert!(tasks.contains(&model.task), "{}", model.name);
            assert!(licenses.contains(&model.license), "{}", model.name);
            assert!(!model.files.is_empty(), "{}", model.name);
            if model.license.starts_with("CC-BY") {
                assert!(
                    model.attribution.is_some(),
                    "{} needs attribution",
                    model.name
                );
            }
            for text in [model.attribution, model.note].into_iter().flatten() {
                assert!(text.ends_with('.') && !text.contains('\u{2014}'), "{text}");
            }
            let mut file_names = Vec::new();
            for file in model.files {
                assert!(is_hex(file.sha256), "{} {}", model.name, file.name);
                assert!(file.size > 0 && is_relative(file.name), "{}", file.name);
                assert!(file.address.starts_with("https://"), "{}", file.address);
                for member in file.unpack {
                    assert!(is_hex(member.sha256) && member.size > 0);
                    assert!(is_relative(member.name) && !member.path.is_empty());
                }
                for (name, _) in kept(file) {
                    file_names.push((file.platform.map(|p| p.to_string()), name));
                }
            }
            let count = file_names.len();
            file_names.sort_unstable();
            file_names.dedup();
            assert_eq!(file_names.len(), count, "{} repeats a file", model.name);
            match model.layout {
                Layout::File => {
                    assert_eq!(model.files.len(), 1);
                    assert_eq!(
                        path(Path::new("m"), model),
                        Path::new("m").join(model.files[0].name)
                    );
                }
                Layout::Folder => {
                    assert_eq!(path(Path::new("m"), model), Path::new("m").join(model.name))
                }
            }
        }
        // The speech models keep their addresses and fields from ADR-0020.
        for model in MODELS.iter().filter(|m| m.task == "speech") {
            assert_eq!(model.layout, Layout::File);
            assert!(model.size() > 50_000_000 && model.license == "MIT");
            assert!(
                url(model)
                    .starts_with("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/")
                    && url(model).ends_with(model.files[0].name)
            );
            assert_eq!(model.languages == "en", model.name.ends_with(".en"));
        }
        let tiny = manifest("whisper-tiny.en").unwrap();
        assert_eq!(
            (tiny.files[0].name, tiny.size()),
            ("ggml-tiny.en.bin", 77_704_715)
        );
        assert_eq!(
            MODELS[..4].iter().map(|m| m.name).collect::<Vec<_>>(),
            [
                "whisper-large-v3-turbo",
                "whisper-small",
                "whisper-base.en",
                "whisper-tiny.en"
            ],
            "the speech models stay first, in their order"
        );
        assert_eq!(
            manifest("whisper-huge"),
            Err(ModelError::Unknown {
                name: "whisper-huge".into()
            })
        );
    }

    #[test]
    fn model_files_and_sizes_are_recorded() {
        let size = |name: &str| manifest(name).unwrap().size();
        assert_eq!(size("birefnet-lite"), 224_005_088);
        assert_eq!(size("mediapipe-pose-full"), 11_925_808 + 12_765_479);
        assert_eq!(size("opus-mt-hu-en"), 120_032_083);
        assert_eq!(size("opus-mt-en-hu"), 120_032_083);
        let kokoro = manifest("kokoro-82m").unwrap();
        assert_eq!(kokoro.files.len(), 33);
        assert_eq!(
            kokoro.size(),
            92_361_116 + 3_497 + 44 + 3_618_488 + 1_754 + 28 * 522_240
        );
        let voices: Vec<&str> = kokoro
            .files
            .iter()
            .filter_map(|f| f.name.strip_prefix("voices/"))
            .collect();
        assert!(
            voices
                .iter()
                .all(|v| v.starts_with(['a', 'b']) && v.ends_with(".bin"))
        );
        assert!(voices.contains(&"af_heart.bin") && voices.contains(&"bm_george.bin"));
        assert_eq!(
            file_path(Path::new("m"), kokoro, "voices/af_heart.bin"),
            Path::new("m")
                .join("kokoro-82m")
                .join("voices/af_heart.bin")
        );
        let pose = manifest("mediapipe-pose-full").unwrap();
        let release = "https://github.com/fossworld/fosscut/releases/download/models-v1/";
        assert_eq!(
            pose.files.iter().map(|f| f.address).collect::<Vec<_>>(),
            [
                format!("{release}mediapipe-pose-full-detector.onnx"),
                format!("{release}mediapipe-pose-full-landmarks.onnx"),
            ]
        );
        assert!(pose.note.unwrap().contains("scripts/models/README.md"));
        assert!(
            pose.attribution
                .unwrap()
                .contains("converted to ONNX by FossCut")
        );
        for (name, languages) in [("opus-mt-hu-en", "hu>en"), ("opus-mt-en-hu", "en>hu")] {
            let opus = manifest(name).unwrap();
            assert_eq!(opus.languages, languages);
            assert_eq!(opus.files.len(), 8);
            let repository = format!("https://huggingface.co/Xenova/{name}/resolve/");
            assert!(
                opus.files
                    .iter()
                    .all(|f| f.address.starts_with(&repository))
            );
        }
    }

    #[test]
    fn the_runtime_has_one_archive_per_system() {
        let runtime = manifest("onnxruntime").unwrap();
        let systems = [
            (
                MACOS_ARM,
                "onnxruntime-osx-arm64-1.31.1.tgz",
                "libonnxruntime.dylib",
            ),
            (
                MACOS_INTEL,
                "onnxruntime-osx-x86_64-1.23.2.tgz",
                "libonnxruntime.dylib",
            ),
            (
                LINUX_X64,
                "onnxruntime-linux-x64-1.31.1.tgz",
                "libonnxruntime.so",
            ),
            (
                WINDOWS_X64,
                "onnxruntime-win-x64-1.31.1.zip",
                "onnxruntime.dll",
            ),
        ];
        for (platform, archive, library) in systems {
            let files = runtime.files_for(platform);
            assert_eq!(files.len(), 1, "{platform}");
            assert_eq!(files[0].name, archive);
            assert!(files[0].address.ends_with(archive));
            let names: Vec<&str> = files[0].unpack.iter().map(|m| m.name).collect();
            assert_eq!(names, [library, "LICENSE", "ThirdPartyNotices.txt"]);
        }
        assert!(
            runtime
                .files_for(Platform {
                    os: "linux",
                    arch: "aarch64"
                })
                .is_empty()
        );
        assert_eq!(
            runtime.files_for(MACOS_ARM)[0].size + runtime.files_for(WINDOWS_X64)[0].size,
            36_936_435 + 82_929_944
        );
        // Models of plain files have the same files on every system.
        let kokoro = manifest("kokoro-82m").unwrap();
        assert_eq!(kokoro.files_for(LINUX_X64), kokoro.files_for(WINDOWS_X64));
        let here = runtime.files();
        if systems.iter().any(|(p, ..)| *p == Platform::CURRENT) {
            assert_eq!(here.len(), 1);
            assert!(here[0].unpack.iter().any(|m| m.name == RUNTIME_LIBRARY));
        } else {
            let dir = folder("unsupported");
            assert_eq!(
                install(&dir, runtime, &Source::Download(None)),
                Err(ModelError::Unsupported {
                    name: "onnxruntime".into(),
                    platform: Platform::CURRENT.to_string(),
                })
            );
        }
        assert_eq!(MACOS_ARM.to_string(), "macos aarch64");
    }

    #[test]
    fn installed_models_are_found_by_size() {
        let dir = folder("installed");
        assert!(list(&dir).iter().all(|m| !m.installed));
        assert_eq!(best_speech_model(&dir, None), None);

        let tiny = manifest("whisper-tiny.en").unwrap();
        sparse(&path(&dir, tiny), tiny.size() - 1);
        assert!(!is_installed(&dir, tiny), "a partial file is not installed");
        sparse(&path(&dir, tiny), tiny.size());
        assert!(is_installed(&dir, tiny));
        let listed = info(&dir, tiny);
        assert!(listed.installed && listed.size_bytes == tiny.size());
        assert_eq!(listed.url, url(tiny));
        assert_eq!(listed.files.len(), 1);
        assert_eq!(listed.files[0].url, url(tiny));
        assert_eq!(best_speech_model(&dir, None), Some(tiny));
        assert_eq!(best_speech_model(&dir, Some("en")), Some(tiny));
        assert_eq!(
            best_speech_model(&dir, Some("de")),
            None,
            "English-only models are only for English"
        );

        let small = manifest("whisper-small").unwrap();
        sparse(&path(&dir, small), small.size());
        assert_eq!(best_speech_model(&dir, Some("de")), Some(small));
        assert_eq!(
            best_speech_model(&dir, None),
            Some(small),
            "the most capable installed model"
        );
        assert_eq!(list(&dir).iter().filter(|m| m.installed).count(), 2);

        // A model of several files is installed only when every one of them is there.
        let kokoro = manifest("kokoro-82m").unwrap();
        for file in &kokoro.files[..kokoro.files.len() - 1] {
            sparse(&file_path(&dir, kokoro, file.name), file.size);
        }
        assert!(!is_installed(&dir, kokoro), "a voice is missing");
        let last = kokoro.files[kokoro.files.len() - 1];
        sparse(&file_path(&dir, kokoro, last.name), last.size);
        assert!(is_installed(&dir, kokoro));

        // The runtime is installed when the members unpacked from its archive are there.
        let runtime = manifest("onnxruntime").unwrap();
        if let Some(archive) = runtime.files().first() {
            for member in archive.unpack {
                sparse(&file_path(&dir, runtime, member.name), member.size);
            }
            assert!(is_installed(&dir, runtime));
            assert!(!dir.join("onnxruntime").join(archive.name).exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn models_list_with_their_files_and_credits() {
        let dir = folder("listed");
        let listed = list(&dir);
        assert_eq!(listed.len(), MODELS.len());
        let tiny = serde_json::to_value(&listed[3]).unwrap();
        assert_eq!(tiny["name"], "whisper-tiny.en");
        assert!(tiny.get("attribution").is_none() && tiny.get("note").is_none());
        assert_eq!(tiny["files"][0]["name"], "ggml-tiny.en.bin");
        let kokoro = serde_json::to_value(info(&dir, manifest("kokoro-82m").unwrap())).unwrap();
        assert_eq!(kokoro["task"], "voice");
        assert_eq!(kokoro["files"].as_array().unwrap().len(), 33);
        assert!(kokoro["attribution"].as_str().unwrap().contains("hexgrad"));
        assert!(
            kokoro["attribution"]
                .as_str()
                .unwrap()
                .contains("Carnegie Mellon")
        );
        let pose =
            serde_json::to_value(info(&dir, manifest("mediapipe-pose-full").unwrap())).unwrap();
        assert_eq!(pose["url"], pose["files"][0]["url"]);
        assert!(
            pose["url"]
                .as_str()
                .unwrap()
                .contains("/releases/download/models-v1/")
        );
        assert!(
            pose["note"]
                .as_str()
                .unwrap()
                .contains("scripts/models/README.md")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installs_are_verified_before_use() {
        let dir = folder("install");
        let source = dir.join("download.bin");
        std::fs::write(&source, b"hello").unwrap();
        let models = dir.join("models");

        // From a local file, and downloaded (here from a file address).
        let installed = install(&models, &HELLO, &Source::File(source.clone())).unwrap();
        assert_eq!(installed, models.join("hello.bin"));
        assert_eq!(std::fs::read(&installed).unwrap(), b"hello");
        std::fs::remove_file(&installed).unwrap();
        install(&models, &HELLO, &Source::Download(Some(file_url(&source)))).unwrap();
        assert!(is_installed(&models, &HELLO));
        std::fs::remove_file(&installed).unwrap();

        // The right size with the wrong contents, then the wrong size.
        std::fs::write(&source, b"HELLO").unwrap();
        let ModelError::ChecksumMismatch {
            expected, actual, ..
        } = install(&models, &HELLO, &Source::File(source.clone())).unwrap_err()
        else {
            panic!("expected a checksum mismatch");
        };
        assert_eq!(
            (expected.as_str(), actual.len()),
            (HELLO.files[0].sha256, 64)
        );
        std::fs::write(&source, b"hi").unwrap();
        assert_eq!(
            install(&models, &HELLO, &Source::File(source.clone())),
            Err(ModelError::ChecksumMismatch {
                name: "hello".into(),
                expected: "5 bytes".into(),
                actual: "2 bytes".into(),
            })
        );
        assert!(!installed.exists(), "a failed install leaves nothing");
        assert!(!models.join("hello.bin.partial").exists());

        // Sources that cannot be read.
        let missing = dir.join("missing.bin");
        assert!(matches!(
            install(&models, &HELLO, &Source::File(missing.clone())),
            Err(ModelError::DownloadFailed { .. })
        ));
        assert!(matches!(
            install(&models, &HELLO, &Source::Download(Some(file_url(&missing)))),
            Err(ModelError::DownloadFailed { .. })
        ));
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"a file where the folder should be").unwrap();
        assert!(matches!(
            install(&blocked, &HELLO, &Source::File(source)),
            Err(ModelError::DownloadFailed { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn models_of_several_files_install_whole_or_not_at_all() {
        let dir = folder("pair");
        let files = dir.join("files");
        std::fs::create_dir_all(files.join("voices")).unwrap();
        std::fs::write(files.join("model.bin"), b"hello").unwrap();
        std::fs::write(files.join("voices/a.bin"), b"world").unwrap();
        assert_eq!(sha(b"world"), PAIR.files[1].sha256);
        let models = dir.join("models");

        // From a folder that holds the files by name.
        let installed = install(&models, &PAIR, &Source::File(files.clone())).unwrap();
        assert_eq!(installed, models.join("pair"));
        assert!(is_installed(&models, &PAIR));
        assert_eq!(
            std::fs::read(installed.join("voices/a.bin")).unwrap(),
            b"world"
        );
        assert!(!models.join("pair.partial").exists());

        // A corrupted voice is rejected; the folder being assembled goes, nothing partial
        // stays, and the copy already installed is kept.
        std::fs::write(files.join("voices/a.bin"), b"WORLD").unwrap();
        let error = install(&models, &PAIR, &Source::File(files.clone())).unwrap_err();
        assert_eq!(
            error,
            ModelError::ChecksumMismatch {
                name: "pair/voices/a.bin".into(),
                expected: PAIR.files[1].sha256.into(),
                actual: sha(b"WORLD"),
            }
        );
        assert!(!models.join("pair.partial").exists());
        assert!(
            is_installed(&models, &PAIR),
            "the earlier install is untouched"
        );

        // Without an earlier copy a failure leaves no folder at all.
        std::fs::remove_dir_all(&installed).unwrap();
        assert!(install(&models, &PAIR, &Source::File(files.clone())).is_err());
        assert!(!installed.exists() && !models.join("pair.partial").exists());
        assert!(!is_installed(&models, &PAIR));

        // Downloaded from a folder address, file by file; a missing file fails the install.
        std::fs::write(files.join("voices/a.bin"), b"world").unwrap();
        install(&models, &PAIR, &Source::Download(Some(file_url(&files)))).unwrap();
        assert!(is_installed(&models, &PAIR));
        std::fs::remove_file(files.join("model.bin")).unwrap();
        assert!(matches!(
            install(
                &models,
                &PAIR,
                &Source::Download(Some(format!("{}/", file_url(&files))))
            ),
            Err(ModelError::DownloadFailed { .. })
        ));
        assert!(is_installed(&models, &PAIR) && !models.join("pair.partial").exists());

        // Several files cannot come from one file.
        assert!(matches!(
            install(&models, &PAIR, &Source::File(dir.join("files/voices/a.bin"))),
            Err(ModelError::DownloadFailed { message, .. }) if message.contains("folder")
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archives_are_verified_then_unpacked() {
        let dir = folder("archive");
        let content = dir.join("content");
        std::fs::create_dir_all(content.join("pkg/lib")).unwrap();
        std::fs::write(content.join("pkg/lib/library.bin"), b"library").unwrap();
        std::fs::write(content.join("pkg/LICENSE"), b"license").unwrap();
        let archive = dir.join("files/pkg.tgz");
        std::fs::create_dir_all(archive.parent().unwrap()).unwrap();
        let packed = Command::new(tar())
            .arg("-c")
            .arg("-z")
            .arg("-f")
            .arg(&archive)
            .arg("-C")
            .arg(&content)
            .arg("pkg")
            .status()
            .unwrap();
        assert!(packed.success());
        let bytes = std::fs::read(&archive).unwrap();
        let archive_sha = Box::leak(sha(&bytes).into_boxed_str());
        let library_sha = Box::leak(sha(b"library").into_boxed_str());
        let license_sha = Box::leak(sha(b"license").into_boxed_str());
        let members: &'static [Member] = Box::leak(Box::new([
            Member {
                path: "pkg/lib/library.bin",
                name: "library.bin",
                size: 7,
                sha256: library_sha,
            },
            Member {
                path: "pkg/LICENSE",
                name: "LICENSE",
                size: 7,
                sha256: license_sha,
            },
        ]));
        let files: &'static [ModelFile] = Box::leak(Box::new([ModelFile {
            address: "https://example.invalid/pkg.tgz",
            name: "pkg.tgz",
            size: bytes.len() as u64,
            sha256: archive_sha,
            platform: Some(Platform::CURRENT),
            unpack: members,
        }]));
        let model = Manifest {
            name: "packed",
            task: "runtime",
            license: "MIT",
            attribution: None,
            languages: "none",
            layout: Layout::Folder,
            files,
            note: None,
        };
        let models = dir.join("models");
        let installed = install(&models, &model, &Source::File(archive.clone())).unwrap();
        assert!(is_installed(&models, &model));
        assert_eq!(
            std::fs::read(installed.join("library.bin")).unwrap(),
            b"library"
        );
        assert!(!installed.join("pkg.tgz").exists() && !installed.join(".unpack").exists());

        // An archive whose member does not match is rejected after unpacking.
        let wrong: &'static [Member] = Box::leak(Box::new([Member {
            sha256: archive_sha,
            ..members[0]
        }]));
        let files: &'static [ModelFile] = Box::leak(Box::new([ModelFile {
            unpack: wrong,
            ..files[0]
        }]));
        let bad = Manifest {
            name: "badly-packed",
            files,
            ..model
        };
        assert!(matches!(
            install(&models, &bad, &Source::File(archive.clone())),
            Err(ModelError::ChecksumMismatch { name, .. }) if name == "badly-packed/library.bin"
        ));
        assert!(
            !models.join("badly-packed").exists() && !models.join("badly-packed.partial").exists()
        );

        // A member the archive lacks fails the unpacking.
        let absent: &'static [Member] = Box::leak(Box::new([Member {
            path: "pkg/missing.bin",
            ..members[0]
        }]));
        let files: &'static [ModelFile] = Box::leak(Box::new([ModelFile {
            unpack: absent,
            ..files[0]
        }]));
        let missing = Manifest {
            name: "missing-member",
            files,
            ..model
        };
        assert!(matches!(
            install(&models, &missing, &Source::File(archive)),
            Err(ModelError::DownloadFailed { message, .. }) if message.contains("cannot unpack")
        ));
        assert!(!models.join("missing-member.partial").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
