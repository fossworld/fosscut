//! Repository automation (ADR-0003). Run as `cargo xtask <task>`:
//!
//! - `schemas [--check]`: write the JSON Schemas in `schemas/`, or fail if they are stale;
//! - `fixtures`: generate test media in `fixtures/generated/` with FFmpeg.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use fosscut_core::Session;
use fosscut_model::{Id, IdKind, MemoryStore, Project, TimelineFormat};
use fosscut_time::Rate;
use serde_json::Value;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("schemas") => schemas(args.iter().any(|a| a == "--check")),
        Some("fixtures") => fixtures(),
        _ => Err("usage: cargo xtask <schemas [--check] | fixtures>".to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn to_bytes(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// The generated schema files, keyed by file name.
fn generate() -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut files = Vec::new();
    for (stem, schema) in fosscut_model::document_schemas() {
        let value = serde_json::to_value(schema).map_err(|e| e.to_string())?;
        files.push((format!("{stem}.schema.json"), to_bytes(&value)?));
    }
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let project = Project::new(Id::new(IdKind::Project, 0), "schemas", format);
    let session = Session::create(MemoryStore::new(), project).map_err(|e| e.to_string())?;
    let api = session
        .query("capabilities", serde_json::json!({ "detail": "full" }))
        .map_err(|e| e.to_string())?;
    files.push(("api.json".to_string(), to_bytes(&api)?));
    Ok(files)
}

fn schemas(check: bool) -> Result<(), String> {
    let dir = root().join("schemas");
    let files = generate()?;
    if check {
        let stale: Vec<&str> = files
            .iter()
            .filter(|(name, bytes)| std::fs::read(dir.join(name)).ok().as_ref() != Some(bytes))
            .map(|(name, _)| name.as_str())
            .collect();
        if stale.is_empty() {
            println!("schemas are up to date ({} files)", files.len());
            return Ok(());
        }
        return Err(format!(
            "stale schemas: {}; run `cargo xtask schemas` and commit the result",
            stale.join(", ")
        ));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, bytes) in &files {
        std::fs::write(dir.join(name), bytes).map_err(|e| format!("{name}: {e}"))?;
        println!("wrote schemas/{name}");
    }
    Ok(())
}

fn fixtures() -> Result<(), String> {
    let dir = root().join("fixtures").join("generated");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let output = dir.join("test-pattern.mp4");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30:duration=8",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=8",
        ])
        .args(["-c:v", "mpeg4", "-q:v", "5", "-c:a", "aac", "-shortest"])
        .arg(&output)
        .status()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    if !status.success() {
        return Err("ffmpeg failed".to_string());
    }
    println!("wrote {}", output.display());
    Ok(())
}
