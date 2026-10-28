//! Repository automation (ADR-0003). Run as `cargo xtask <task>`:
//!
//! - `schemas [--check]`: write the JSON Schemas in `schemas/`, or fail if they are stale;
//! - `fixtures`: generate test media in `fixtures/generated/` with FFmpeg;
//! - `coverage`: run the tests with coverage instrumentation and fail below the minimums;
//! - `goldens [--update]`: run the golden-frame render tests, or rewrite their PNGs.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use fosscut_core::Session;
use fosscut_model::{Id, IdKind, MemoryStore, Project, TimelineFormat};
use fosscut_time::Rate;
use serde_json::Value;

/// Minimum line coverage in percent for the whole workspace (ADR-0016). Raise it as coverage
/// grows; never lower it to make a change pass.
const MIN_LINES_TOTAL: f64 = 90.0;

/// Minimum line coverage in percent for each crate in `crates/`, so that a new or neglected
/// crate cannot hide behind the workspace total. A crate that cannot reach it (for example
/// GPU code without a GPU in CI) needs an explicit, reviewed exception here.
const MIN_LINES_PER_CRATE: f64 = 85.0;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("schemas") => schemas(args.iter().any(|a| a == "--check")),
        Some("fixtures") => fixtures(),
        Some("coverage") => coverage(),
        Some("goldens") => goldens(args.iter().any(|a| a == "--update")),
        _ => Err(
            "usage: cargo xtask <schemas [--check] | fixtures | coverage | goldens [--update]>"
                .to_string(),
        ),
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

/// Test media generated with FFmpeg's `lavfi` sources (ADR-0016, item 7): name, then the
/// FFmpeg arguments between `-y -v error` and the output path.
const FIXTURES: &[(&str, &[&str])] = &[
    (
        // The round 1 audit fixture: 8 s, 720p30 with a 440 Hz tone.
        "test-pattern.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30:duration=8",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=8",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
            "-c:a",
            "aac",
            "-shortest",
        ],
    ),
    (
        // The ADR-0008 spike source: 10 s of 4K30 H.264, tagged BT.709 limited range
        // with `setparams` so the tags do not trigger a pixel conversion.
        "test-pattern-4k.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=3840x2160:rate=30:duration=10,format=yuv420p,\
             setparams=range=tv:colorspace=bt709:color_primaries=bt709:color_trc=bt709",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "20",
            "-g",
            "30",
        ],
    ),
];

fn fixtures() -> Result<(), String> {
    let dir = root().join("fixtures").join("generated");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, args) in FIXTURES {
        let output = dir.join(name);
        let status = Command::new("ffmpeg")
            .args(["-y", "-v", "error"])
            .args(*args)
            .arg(&output)
            .status()
            .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
        if !status.success() {
            return Err(format!("ffmpeg failed to write {name}"));
        }
        println!("wrote {}", output.display());
    }
    Ok(())
}

/// A `cargo` command run from the workspace root with the same cargo that runs xtask.
fn cargo(args: &[&str]) -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(root()).args(args);
    command
}

fn run(mut command: Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|e| format!("cannot run {command:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command:?} failed"))
    }
}

/// Covered and total source lines.
#[derive(Default)]
struct Lines {
    covered: usize,
    count: usize,
}

impl Lines {
    fn percent(&self) -> f64 {
        if self.count == 0 {
            100.0
        } else {
            self.covered as f64 * 100.0 / self.count as f64
        }
    }
}

/// Per-file line coverage from an LCOV report. Uses the per-line `DA` records rather than the
/// summary counts, which count a line once per binary that contains it (a binary's code is
/// also compiled into its unit test harness).
fn lcov_lines(report: &str) -> BTreeMap<PathBuf, Lines> {
    let mut files: BTreeMap<PathBuf, BTreeMap<u32, bool>> = BTreeMap::new();
    let mut current: Option<PathBuf> = None;
    for line in report.lines() {
        if let Some(path) = line.strip_prefix("SF:") {
            current = Some(PathBuf::from(path));
        } else if let (Some(file), Some(record)) = (&current, line.strip_prefix("DA:")) {
            let mut fields = record.split(',');
            let number = fields.next().and_then(|n| n.parse::<u32>().ok());
            let hits = fields.next().and_then(|n| n.parse::<u64>().ok());
            if let (Some(number), Some(hits)) = (number, hits) {
                *files
                    .entry(file.clone())
                    .or_default()
                    .entry(number)
                    .or_default() |= hits > 0;
            }
        }
    }
    files
        .into_iter()
        .map(|(path, lines)| {
            let covered = lines.values().filter(|hit| **hit).count();
            let count = lines.len();
            (path, Lines { covered, count })
        })
        .collect()
}

fn coverage() -> Result<(), String> {
    let help = "coverage needs cargo-llvm-cov, cargo-nextest and the llvm-tools component: \
                cargo install cargo-llvm-cov cargo-nextest && rustup component add llvm-tools";
    run(cargo(&[
        "llvm-cov",
        "nextest",
        "--workspace",
        "--exclude",
        "xtask",
        "--all-features",
        "--locked",
        "--no-report",
    ]))
    .map_err(|e| format!("{e}\n{help}"))?;

    let dir = root().join("target").join("coverage");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let lcov = dir.join("lcov.info");
    let mut report = cargo(&["llvm-cov", "report", "--lcov", "--output-path"]);
    report.arg(&lcov);
    run(report)?;
    let files = lcov_lines(&std::fs::read_to_string(&lcov).map_err(|e| e.to_string())?);

    let crates_dir = root().join("crates");
    let mut total = Lines::default();
    let mut per_crate: BTreeMap<String, Lines> = BTreeMap::new();
    for (path, lines) in &files {
        total.covered += lines.covered;
        total.count += lines.count;
        if let Some(name) = path
            .strip_prefix(&crates_dir)
            .ok()
            .and_then(|p| p.components().next())
        {
            let entry = per_crate
                .entry(name.as_os_str().to_string_lossy().into_owned())
                .or_default();
            entry.covered += lines.covered;
            entry.count += lines.count;
        }
    }

    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (name, lines) in &per_crate {
        let ok = lines.percent() >= MIN_LINES_PER_CRATE;
        rows.push((name.as_str(), lines, MIN_LINES_PER_CRATE, ok));
        if !ok {
            failures.push(name.as_str());
        }
    }
    let ok = total.percent() >= MIN_LINES_TOTAL;
    rows.push(("workspace", &total, MIN_LINES_TOTAL, ok));
    if !ok {
        failures.push("workspace");
    }

    let mut table = String::from(
        "| Scope | Lines covered | Line coverage | Minimum | Result |\n|---|---|---|---|---|\n",
    );
    for (name, lines, min, ok) in &rows {
        table.push_str(&format!(
            "| {name} | {} / {} | {:.2}% | {min:.0}% | {} |\n",
            lines.covered,
            lines.count,
            lines.percent(),
            if *ok { "pass" } else { "FAIL" }
        ));
    }
    let mut weakest: Vec<(&PathBuf, &Lines)> = files.iter().collect();
    weakest.sort_by_key(|(_, lines)| std::cmp::Reverse(lines.count - lines.covered));
    let mut gaps = String::from("Most uncovered lines:\n");
    for (path, lines) in weakest.iter().take(5) {
        let shown = path.strip_prefix(root()).unwrap_or(path);
        gaps.push_str(&format!(
            "  {:>4} lines  {}\n",
            lines.count - lines.covered,
            shown.display()
        ));
    }
    println!(
        "\nLine coverage (minimums in xtask/src/main.rs, report in target/coverage/lcov.info)\n\n{table}\n{gaps}"
    );
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        writeln!(file, "## Line coverage\n\n{table}").map_err(|e| e.to_string())?;
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "line coverage is below the minimum for: {}. Add tests for the uncovered code \
             (run `cargo llvm-cov nextest --workspace --exclude xtask --open` to see it); \
             do not lower the minimums",
            failures.join(", ")
        ))
    }
}

/// Runs the golden-frame tests; with `update`, rewrites the PNGs for review instead
/// (the `golden-frame-tests` skill).
fn goldens(update: bool) -> Result<(), String> {
    let mut command = cargo(&["test", "-p", "fosscut-render", "--test", "goldens"]);
    if update {
        command.env("FOSSCUT_UPDATE_GOLDENS", "1");
    }
    run(command)?;
    if update {
        println!(
            "goldens rewritten in crates/fosscut-render/tests/goldens/; open every changed \
             PNG and describe the visual change in the pull request"
        );
    }
    Ok(())
}
