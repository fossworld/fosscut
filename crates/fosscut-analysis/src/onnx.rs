//! ONNX Runtime for the local vision, voice and translation models (ADR-0033 item 1).
//!
//! FossCut does not link ONNX Runtime. [`session`] opens Microsoft's library at run time
//! through `ort`'s dynamic loading, from the first of [`runtime_candidates`] that exists,
//! with ONNX Runtime's telemetry switched off. A missing or unusable library is the typed
//! [`OnnxError::RuntimeMissing`] or [`OnnxError::RuntimeUnusable`], never a crash, and a
//! model that cannot be loaded is [`OnnxError::LoadFailed`].
//!
//! Where a session runs is its [`Device`]. [`Device::Accelerator`] registers CoreML on macOS
//! and DirectML on Windows where the library has them, with the CPU for whatever those
//! cannot run. [`Device::Auto`], the default, is the CPU today: with ONNX Runtime 1.31.1 none
//! of FossCut's models ran correctly and faster on CoreML or WebGPU (the vision models fail
//! to compile, the voice and translation models compile and then fail while running), and
//! Microsoft's Windows release has no DirectML. [`Device::Cpu`] forces the CPU with
//! deterministic kernels, so tests get the same results on every run.
//!
//! The library is loaded once per process. Later sessions reuse it, even when
//! `FOSSCUT_ORT_DYLIB` or the models folder changes.
//!
//! The library, the error type and [`runtime_path`] exist in every build; [`session`] and
//! [`runtime`] need the `onnx` feature, which is on by default.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::models;

#[cfg(feature = "onnx")]
pub use ort;
#[cfg(feature = "onnx")]
pub use ort::session::Session;

/// The model that installs the runtime: `fosscut models install onnxruntime`.
pub const RUNTIME_MODEL: &str = "onnxruntime";

/// The environment variable that names an ONNX Runtime library to use instead.
pub const LIBRARY_VARIABLE: &str = "FOSSCUT_ORT_DYLIB";

/// The oldest ONNX Runtime release FossCut can use (the `api-22` feature of `ort`).
pub const MINIMUM_VERSION: &str = "1.22";

/// Why a model could not run.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum OnnxError {
    /// No ONNX Runtime library was found.
    #[error("ONNX Runtime is not installed (looked for {})", tried.join(", "))]
    RuntimeMissing {
        /// Every place that was looked at, in order.
        tried: Vec<String>,
    },
    /// A library was found but cannot be used: older than [`MINIMUM_VERSION`], damaged, or
    /// built for another system.
    #[error("the ONNX Runtime library at {library} cannot be used: {message}")]
    RuntimeUnusable {
        /// The library.
        library: String,
        /// What ONNX Runtime or the loader reported.
        message: String,
    },
    /// The model could not be loaded into a session.
    #[error("cannot load {model}: {message}")]
    LoadFailed {
        /// The model file.
        model: String,
        /// What ONNX Runtime reported.
        message: String,
    },
}

impl OnnxError {
    /// The stable error code front ends report (ADR-0006).
    pub fn code(&self) -> &'static str {
        match self {
            OnnxError::RuntimeMissing { .. } | OnnxError::RuntimeUnusable { .. } => {
                "fosscut.model.runtime_missing"
            }
            OnnxError::LoadFailed { .. } => "fosscut.model.load_failed",
        }
    }
}

/// Where a session runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Device {
    /// The best choice known to work for FossCut's models: the CPU on every system for now
    /// (see the module documentation), on all its cores.
    #[default]
    Auto,
    /// The CPU only, with deterministic kernels: the same results on every run, for tests.
    Cpu,
    /// The system's accelerator where the library has it, CoreML on macOS (on the CPU and
    /// GPU: compiling for the Neural Engine did not finish in testing) and DirectML on
    /// Windows, with the CPU for the nodes it does not take. A model it cannot compile gets
    /// a CPU session instead, but some models compile and then fail while running, so use it
    /// only for a model checked on it.
    Accelerator,
}

/// How to make a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionOptions {
    /// The models folder, where the installed `onnxruntime` model is looked for.
    pub models: PathBuf,
    /// Where it runs.
    pub device: Device,
    /// Threads for each operator; ONNX Runtime chooses when `None`.
    pub threads: Option<usize>,
}

impl SessionOptions {
    /// Sessions on [`Device::Auto`], with the runtime looked for in `models`.
    pub fn new(models: impl Into<PathBuf>) -> SessionOptions {
        SessionOptions {
            models: models.into(),
            device: Device::Auto,
            threads: None,
        }
    }

    /// The same options on `device`.
    pub fn on(self, device: Device) -> SessionOptions {
        SessionOptions { device, ..self }
    }
}

/// Every place the ONNX Runtime library is looked for, in order: `FOSSCUT_ORT_DYLIB`, the
/// executable's folder, then the installed `onnxruntime` model in `models`.
pub fn runtime_candidates(models: &Path) -> Vec<PathBuf> {
    candidates(
        std::env::var_os(LIBRARY_VARIABLE),
        std::env::current_exe().ok(),
        models,
    )
}

/// [`runtime_candidates`] from the variable's value and the executable's path, as absolute
/// paths.
fn candidates(
    variable: Option<OsString>,
    executable: Option<PathBuf>,
    models: &Path,
) -> Vec<PathBuf> {
    let absolute = |path: PathBuf| std::path::absolute(&path).unwrap_or(path);
    let mut found = Vec::new();
    if let Some(variable) = variable.filter(|v| !v.is_empty()) {
        found.push(absolute(PathBuf::from(variable)));
    }
    if let Some(folder) = executable.as_deref().and_then(Path::parent) {
        found.push(absolute(folder.join(models::RUNTIME_LIBRARY)));
    }
    if let Ok(runtime) = models::manifest(RUNTIME_MODEL) {
        found.push(absolute(models::file_path(
            models,
            runtime,
            models::RUNTIME_LIBRARY,
        )));
    }
    found
}

/// The ONNX Runtime library to load: the first of [`runtime_candidates`] that exists.
pub fn runtime_path(models: &Path) -> Option<PathBuf> {
    runtime_candidates(models).into_iter().find(|p| p.is_file())
}

/// The loaded runtime.
#[cfg(feature = "onnx")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    /// The library in use.
    pub library: PathBuf,
    /// ONNX Runtime's build description, with its version and commit.
    pub build: String,
}

/// Loads the runtime if it is not loaded yet, and says which it is.
#[cfg(feature = "onnx")]
pub fn runtime(models: &Path) -> Result<Runtime, OnnxError> {
    let (library, _) = loaded::environment(models)?;
    Ok(Runtime {
        library,
        build: ort::info().to_string(),
    })
}

/// The process's ONNX Runtime environment, loading the runtime first if needed, for
/// sessions with options [`session`] does not offer.
#[cfg(feature = "onnx")]
pub fn environment(models: &Path) -> Result<ort::environment::Environment, OnnxError> {
    loaded::environment(models).map(|(_, environment)| environment)
}

/// A session for `model_file`, loading the runtime first if needed. On
/// [`Device::Accelerator`], a model the accelerator cannot compile gets a CPU session
/// instead; a failure on the CPU is [`OnnxError::LoadFailed`].
#[cfg(feature = "onnx")]
pub fn session(model_file: &Path, options: &SessionOptions) -> Result<Session, OnnxError> {
    let environment = environment(&options.models)?;
    let failed = |message: String| OnnxError::LoadFailed {
        model: model_file.display().to_string(),
        message,
    };
    if !model_file.is_file() {
        return Err(failed("there is no such file".into()));
    }
    let build = |device: Device, deterministic: bool| -> Result<Session, String> {
        use ort::session::builder::AutoDevicePolicy;
        let mut builder = Session::builder(&environment)
            .map_err(|e| e.to_string())?
            // Only the providers below: ONNX Runtime's own choice could add CoreML to CPU
            // runs.
            .with_auto_device(AutoDevicePolicy::PreferCPU)
            .map_err(|e| e.to_string())?
            .with_execution_providers(providers(device))
            .map_err(|e| e.to_string())?;
        if deterministic {
            builder = builder
                .with_deterministic_compute(true)
                .map_err(|e| e.to_string())?;
        }
        if let Some(threads) = options.threads {
            builder = builder
                .with_intra_threads(threads)
                .map_err(|e| e.to_string())?;
        }
        builder
            .commit_from_file(model_file)
            .map_err(|e| e.to_string())
    };
    match options.device {
        Device::Cpu => build(Device::Cpu, true).map_err(failed),
        Device::Auto => build(Device::Auto, false).map_err(failed),
        Device::Accelerator => build(Device::Accelerator, false)
            .or_else(|_| build(Device::Cpu, false))
            .map_err(failed),
    }
}

/// The execution providers for `device`, most preferred first; the CPU always comes last.
#[cfg(feature = "onnx")]
fn providers(device: Device) -> Vec<ort::ep::ExecutionProviderDispatch> {
    let mut providers = match device {
        Device::Accelerator => accelerators(),
        Device::Auto | Device::Cpu => Vec::new(),
    };
    providers.push(ort::ep::CPU::default().build());
    providers
}

/// CoreML, as an ML Program on the CPU and GPU.
#[cfg(all(feature = "onnx", target_os = "macos"))]
fn accelerators() -> Vec<ort::ep::ExecutionProviderDispatch> {
    use ort::ep::coreml::{ComputeUnits, ModelFormat};
    vec![
        ort::ep::CoreML::default()
            .with_model_format(ModelFormat::MLProgram)
            .with_compute_units(ComputeUnits::CPUAndGPU)
            .build(),
    ]
}

/// DirectML. Microsoft's Windows release does not include it; registering it then fails
/// quietly and the CPU runs everything.
#[cfg(all(feature = "onnx", windows))]
fn accelerators() -> Vec<ort::ep::ExecutionProviderDispatch> {
    vec![ort::ep::DirectML::default().build()]
}

/// No accelerator on other systems.
#[cfg(all(feature = "onnx", not(any(target_os = "macos", windows))))]
fn accelerators() -> Vec<ort::ep::ExecutionProviderDispatch> {
    Vec::new()
}

/// The process's one ONNX Runtime environment.
#[cfg(feature = "onnx")]
mod loaded {
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, PoisonError};

    use ort::environment::Environment;

    use super::{OnnxError, runtime_candidates};

    /// The library and environment, once loaded. A failed attempt is not kept, so
    /// installing the runtime later works without a restart.
    static LOADED: Mutex<Option<(PathBuf, Environment)>> = Mutex::new(None);

    /// The loaded library and its environment, loading it from the first candidate that
    /// exists if this is the first use.
    pub(super) fn environment(models: &Path) -> Result<(PathBuf, Environment), OnnxError> {
        let mut loaded = LOADED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((library, environment)) = loaded.as_ref() {
            return Ok((library.clone(), environment.clone()));
        }
        let tried = runtime_candidates(models);
        let library = tried.iter().find(|p| p.is_file()).cloned().ok_or_else(|| {
            OnnxError::RuntimeMissing {
                tried: tried.iter().map(|p| p.display().to_string()).collect(),
            }
        })?;
        let unusable = |message: String| OnnxError::RuntimeUnusable {
            library: library.display().to_string(),
            message,
        };
        let environment = ort::init_from(&library)
            .map_err(|e| unusable(e.to_string()))?
            .with_name("fosscut")
            // Microsoft's builds send usage events unless told not to (ADR-0004: no
            // telemetry).
            .with_telemetry(false)
            .build()
            .map_err(|e| unusable(e.to_string()))?;
        // ONNX Runtime's own messages stay quiet unless ORT_LOG asks for them; failures
        // come back as errors.
        if std::env::var_os("ORT_LOG").is_none() {
            environment.set_log_level(ort::logging::LogLevel::Fatal);
        }
        *loaded = Some((library.clone(), environment.clone()));
        Ok((library, environment))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_is_looked_for_in_order() {
        let base = std::env::temp_dir().join(format!("fosscut-onnx-{}", std::process::id()));
        let models = base.join("models");
        let executable = base.join("bin").join("fosscut");
        let found = candidates(
            Some(base.join("custom").join("lib.bin").into_os_string()),
            Some(executable.clone()),
            &models,
        );
        assert_eq!(
            found,
            [
                base.join("custom").join("lib.bin"),
                base.join("bin").join(models::RUNTIME_LIBRARY),
                models.join("onnxruntime").join(models::RUNTIME_LIBRARY),
            ]
        );
        // An empty variable is ignored; relative paths become absolute.
        let found = candidates(Some(OsString::new()), None, Path::new("models"));
        assert_eq!(found.len(), 1);
        assert!(found[0].is_absolute());
        assert!(
            found[0].ends_with(
                Path::new("models")
                    .join("onnxruntime")
                    .join(models::RUNTIME_LIBRARY)
            )
        );
        assert_eq!(
            candidates(Some("lib.bin".into()), None, &models)[0],
            std::path::absolute("lib.bin").unwrap()
        );
    }

    #[test]
    fn the_installed_runtime_is_found_when_nothing_else_is() {
        let models =
            std::env::temp_dir().join(format!("fosscut-onnx-models-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&models);
        let elsewhere = runtime_candidates(&models)[..runtime_candidates(&models).len() - 1]
            .iter()
            .any(|p| p.is_file());
        if !elsewhere {
            assert_eq!(runtime_path(&models), None);
        }
        let library = models.join("onnxruntime").join(models::RUNTIME_LIBRARY);
        std::fs::create_dir_all(library.parent().unwrap()).unwrap();
        std::fs::write(&library, b"not really a library").unwrap();
        if !elsewhere {
            assert_eq!(runtime_path(&models), Some(library));
        }
        let _ = std::fs::remove_dir_all(&models);
    }

    #[test]
    fn errors_have_codes_and_say_what_was_tried() {
        let missing = OnnxError::RuntimeMissing {
            tried: vec![
                "/a/libonnxruntime.dylib".into(),
                "/b/libonnxruntime.dylib".into(),
            ],
        };
        assert_eq!(missing.code(), "fosscut.model.runtime_missing");
        assert_eq!(
            missing.to_string(),
            "ONNX Runtime is not installed (looked for /a/libonnxruntime.dylib, /b/libonnxruntime.dylib)"
        );
        let unusable = OnnxError::RuntimeUnusable {
            library: "/a/lib".into(),
            message: "too old".into(),
        };
        assert_eq!(unusable.code(), "fosscut.model.runtime_missing");
        let load = OnnxError::LoadFailed {
            model: "m.onnx".into(),
            message: "bad".into(),
        };
        assert_eq!(load.code(), "fosscut.model.load_failed");
        assert_eq!(load.to_string(), "cannot load m.onnx: bad");
        let options = SessionOptions::new("models").on(Device::Cpu);
        assert_eq!(
            (options.device, options.threads, options.models),
            (Device::Cpu, None, PathBuf::from("models"))
        );
        assert_eq!(Device::default(), Device::Auto);
    }

    #[cfg(feature = "onnx")]
    #[test]
    fn a_missing_runtime_is_an_error_not_a_crash() {
        let models = std::env::temp_dir().join(format!("fosscut-onnx-none-{}", std::process::id()));
        if runtime_path(&models).is_some() {
            return; // A runtime is installed elsewhere on this machine.
        }
        let error = session(Path::new("model.onnx"), &SessionOptions::new(&models)).unwrap_err();
        let OnnxError::RuntimeMissing { tried } = &error else {
            panic!("expected a missing runtime, got {error}");
        };
        assert_eq!(
            tried.last().map(PathBuf::from),
            Some(models.join("onnxruntime").join(models::RUNTIME_LIBRARY))
        );
        assert!(matches!(
            runtime(&models),
            Err(OnnxError::RuntimeMissing { .. })
        ));
    }
}
