//! Rendering a composition of the user's own Remotion project (ADR-0039).
//!
//! Remotion is source-available under its own licence, which requires a paid company
//! licence above a small company size, so FossCut never bundles, installs or requires it.
//! When a user has a Remotion project, FossCut can run the Remotion installed in that
//! project, after the user confirms, with `npx --no-install remotion render` into the
//! bundle's `renders/` folder as ProRes 4444 with alpha, then import the file as video.

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::web::WebError;
use crate::{EngineError, Progress};

/// What the user is told before Remotion runs: its licence terms in one sentence.
pub const LICENCE_NOTE: &str = "Remotion is source-available under its own licence, which is free for individuals and small companies but requires a paid company licence above that size (remotion.dev/license); FossCut runs the copy installed in your project and never installs it.";

/// The bundle folder renders are written into.
pub const RENDERS_DIR: &str = "renders";

/// The environment variable naming the `npx` to run; `npx` on the path otherwise.
pub const NPX_VARIABLE: &str = "FOSSCUT_NPX";

/// Lines of Remotion's output kept for errors.
const LOG_LINES: usize = 30;

/// A Remotion render to run.
#[derive(Clone, Debug, PartialEq)]
pub struct RemotionRequest {
    /// The Remotion project: a folder whose `package.json` depends on `remotion`.
    pub project: PathBuf,
    /// The composition id, as `npx remotion compositions` lists it.
    pub composition: String,
    /// Input props for the composition, passed as a JSON file.
    pub props: Option<Value>,
    /// The entry point relative to the project; found among Remotion's usual places when
    /// `None`.
    pub entry: Option<String>,
    /// The `.fosscut` bundle to render into.
    pub bundle: PathBuf,
    /// True once the user agreed to run Remotion under its licence.
    pub confirmed: bool,
    /// The `npx` to run; `FOSSCUT_NPX`, or `npx` on the path, when `None`.
    pub npx: Option<PathBuf>,
}

/// A checked render, ready to run.
#[derive(Clone, Debug, PartialEq)]
pub struct RemotionPlan {
    request: RemotionRequest,
    entry: String,
    /// The file the render is written to.
    pub output: PathBuf,
    /// The command line, as shown to the user.
    pub command: Vec<String>,
}

/// A finished render.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemotionReport {
    /// The rendered file in the bundle's `renders/` folder.
    pub output: String,
    /// The composition rendered.
    pub composition: String,
    /// The command that ran.
    pub command: Vec<String>,
    /// The asset it was imported as, when it was imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
}

/// The entry points Remotion looks for, in its order.
const ENTRIES: &[&str] = &[
    "src/index.ts",
    "src/index.tsx",
    "src/index.js",
    "src/index.jsx",
    "remotion/index.ts",
    "remotion/index.tsx",
    "remotion/index.js",
    "app/remotion/index.ts",
    "app/remotion/index.tsx",
    "src/remotion/index.ts",
    "src/remotion/index.tsx",
    "src/remotion/index.js",
];

/// True when `folder` holds a `package.json` that depends on Remotion.
pub fn is_project(folder: &Path) -> bool {
    dependencies(folder).is_ok()
}

fn dependencies(folder: &Path) -> Result<(), WebError> {
    let not = |message: String| WebError::NotRemotion {
        path: folder.display().to_string(),
        message,
    };
    let text = std::fs::read_to_string(folder.join("package.json"))
        .map_err(|_| not("it has no package.json".into()))?;
    let package: Value = serde_json::from_str(&text)
        .map_err(|e| not(format!("its package.json is not JSON: {e}")))?;
    let depends = ["dependencies", "devDependencies"].iter().any(|list| {
        package[list]
            .as_object()
            .is_some_and(|deps| deps.contains_key("remotion") || deps.contains_key("@remotion/cli"))
    });
    if depends {
        Ok(())
    } else {
        Err(not("its package.json does not depend on remotion".into()))
    }
}

/// The `npx` to run.
fn npx() -> PathBuf {
    match std::env::var_os(NPX_VARIABLE).filter(|v| !v.is_empty()) {
        Some(path) => path.into(),
        None if cfg!(windows) => "npx.cmd".into(),
        None => "npx".into(),
    }
}

/// `program` itself when it is a path to a file, otherwise the first file of that name in
/// the folders of `PATH`.
fn find_program(program: &Path) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return program.is_file().then(|| program.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// The first file in `renders/` of `bundle` named after `composition` that does not exist
/// yet: `<composition>.mov`, then `<composition>-2.mov` and so on.
pub fn default_output(bundle: &Path, composition: &str) -> PathBuf {
    let safe: String = composition
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let dir = bundle.join(RENDERS_DIR);
    (1..)
        .map(|n| {
            if n == 1 {
                dir.join(format!("{safe}.mov"))
            } else {
                dir.join(format!("{safe}-{n}.mov"))
            }
        })
        .find(|path| !path.exists())
        .unwrap_or_else(|| dir.join(format!("{safe}.mov")))
}

/// Checks `request` before anything runs: the project depends on Remotion and has it
/// installed, the entry point exists, and the user confirmed. Without confirmation it fails
/// with `fosscut.web.confirmation_required`, which carries [`LICENCE_NOTE`] and the command
/// that would run.
pub fn plan(request: &RemotionRequest) -> Result<RemotionPlan, EngineError> {
    let project = &request.project;
    if !project.is_dir() {
        return Err(WebError::NotRemotion {
            path: project.display().to_string(),
            message: "it is not a folder".into(),
        }
        .into());
    }
    dependencies(project)?;
    if request.composition.trim().is_empty() || request.composition.starts_with('-') {
        return Err(EngineError::InvalidArguments {
            message: format!("{:?} is not a composition id", request.composition),
        });
    }
    let entry = match &request.entry {
        Some(entry) => {
            if !project.join(entry).is_file() {
                return Err(WebError::NotRemotion {
                    path: project.display().to_string(),
                    message: format!("its entry point {entry} does not exist"),
                }
                .into());
            }
            entry.clone()
        }
        None => ENTRIES
            .iter()
            .find(|e| project.join(e).is_file())
            .map(|e| e.to_string())
            .ok_or_else(|| WebError::NotRemotion {
                path: project.display().to_string(),
                message: format!(
                    "no entry point was found ({}); give one",
                    ENTRIES[..4].join(", ")
                ),
            })?,
    };
    let installed = ["node_modules/@remotion/cli", "node_modules/.bin/remotion"]
        .iter()
        .any(|p| project.join(p).exists());
    if !installed {
        return Err(WebError::RemotionUnavailable {
            message: format!(
                "Remotion is not installed in {} (no node_modules/@remotion/cli)",
                project.display()
            ),
        }
        .into());
    }
    let npx = request.npx.clone().unwrap_or_else(npx);
    let npx = find_program(&npx).ok_or_else(|| WebError::RemotionUnavailable {
        message: format!("npx (Node.js) was not found on the path; {NPX_VARIABLE} can name it"),
    })?;
    // Remotion runs in the project's folder, so the output is named absolutely.
    let bundle = std::path::absolute(&request.bundle).unwrap_or_else(|_| request.bundle.clone());
    let output = default_output(&bundle, &request.composition);
    let mut command = vec![
        npx.display().to_string(),
        "--no-install".into(),
        "remotion".into(),
        "render".into(),
        entry.clone(),
        request.composition.clone(),
        output.display().to_string(),
        "--codec=prores".into(),
        "--prores-profile=4444".into(),
        "--pixel-format=yuva444p10le".into(),
        "--image-format=png".into(),
    ];
    if request.props.is_some() {
        command.push(format!("--props={}", props_path(&output).display()));
    }
    if !request.confirmed {
        return Err(WebError::RemotionConfirmation {
            licence: LICENCE_NOTE.into(),
            command,
        }
        .into());
    }
    Ok(RemotionPlan {
        request: request.clone(),
        entry,
        output,
        command,
    })
}

fn props_path(output: &Path) -> PathBuf {
    let name = output
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    output.with_file_name(format!(".{name}.props.json"))
}

/// Runs a planned render, reporting frames done of the total as Remotion prints them; the
/// render stops when `cancel` is set or `progress` answers `false`, leaving no file. The
/// output is written under a hidden name and moved into place once Remotion succeeds.
pub fn render(
    plan: &RemotionPlan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<RemotionReport, EngineError> {
    let output = &plan.output;
    let dir = output.parent().unwrap_or(Path::new("."));
    let failed = |message: String| {
        EngineError::Web(WebError::RemotionFailed {
            message,
            log: Vec::new(),
        })
    };
    std::fs::create_dir_all(dir)
        .map_err(|e| failed(format!("cannot create {}: {e}", dir.display())))?;
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let partial = dir.join(format!(".{name}.partial.mov"));
    let props = props_path(output);
    let _ = std::fs::remove_file(&partial);
    if let Some(value) = &plan.request.props {
        std::fs::write(&props, value.to_string())
            .map_err(|e| failed(format!("cannot write the props: {e}")))?;
    }
    let mut args: Vec<String> = plan.command[1..].to_vec();
    // The command as shown names the final file; Remotion writes the hidden one.
    if let Some(slot) = args
        .iter_mut()
        .find(|a| Path::new(a.as_str()) == output.as_path())
    {
        *slot = partial.display().to_string();
    }
    let mut command = if cfg!(unix) {
        // A small shell leads a process group of its own with Remotion in it, and ends
        // the whole group (npx, Node, the browser and FFmpeg it starts) once its standard
        // input closes: when the render is cancelled, or FossCut exits or is killed. A
        // background job's standard input is /dev/null unless redirected, so the pipe is
        // passed to `cat` as descriptor 3.
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("exec 3<&0; \"$@\" </dev/null 3<&- & child=$!; (cat <&3; kill -TERM 0) >/dev/null 2>&1 & exec 3<&-; wait \"$child\"")
            .arg("fosscut-remotion")
            .arg(&plan.command[0])
            .args(&args)
            .stdin(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command
    } else {
        let mut command = Command::new(&plan.command[0]);
        command.args(&args).stdin(Stdio::null());
        command
    };
    command
        .current_dir(&plan.request.project)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let result = run(command, cancel, progress);
    let _ = std::fs::remove_file(&props);
    match result {
        Ok(()) if partial.is_file() => {
            std::fs::rename(&partial, output)
                .map_err(|e| failed(format!("cannot move the render into place: {e}")))?;
            Ok(RemotionReport {
                output: output.display().to_string(),
                composition: plan.request.composition.clone(),
                command: plan.command.clone(),
                asset: None,
            })
        }
        Ok(()) => Err(failed(format!(
            "Remotion finished without writing {}",
            partial.display()
        ))),
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            Err(error)
        }
    }
}

/// Runs `command`, following its output for progress, until it exits or is cancelled.
fn run(
    mut command: Command,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<(), EngineError> {
    let mut child = command.spawn().map_err(|e| {
        EngineError::Web(WebError::RemotionUnavailable {
            message: format!("cannot run npx (Node.js): {e}"),
        })
    })?;
    let watchdog = child.stdin.take();
    let log = Arc::new(Mutex::new(Log::default()));
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|stream| {
        let log = Arc::clone(&log);
        std::thread::spawn(move || follow(stream, &log))
    })
    .collect();
    let status = loop {
        let latest = lock(&log).progress;
        let go_on = progress(latest);
        if !go_on || cancel.load(Ordering::Relaxed) {
            drop(watchdog);
            stop(&mut child);
            for reader in readers {
                let _ = reader.join();
            }
            return Err(EngineError::JobCancelled {
                kind: "remotion".into(),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                drop(watchdog);
                stop(&mut child);
                return Err(EngineError::Web(WebError::RemotionFailed {
                    message: format!("lost Remotion: {e}"),
                    log: Vec::new(),
                }));
            }
        }
    };
    // Ends what is left of the group, then the output pipes close.
    drop(watchdog);
    for reader in readers {
        let _ = reader.join();
    }
    let log = lock(&log);
    let _ = progress(log.progress);
    if status.success() {
        Ok(())
    } else {
        Err(EngineError::Web(WebError::RemotionFailed {
            message: format!("Remotion exited with {status}"),
            log: log.lines.iter().cloned().collect(),
        }))
    }
}

/// What Remotion printed: its last lines and the latest `done/total` count.
#[derive(Default)]
struct Log {
    lines: std::collections::VecDeque<String>,
    progress: Progress,
}

fn lock(log: &Mutex<Log>) -> std::sync::MutexGuard<'_, Log> {
    log.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Reads `stream` line by line (progress bars end lines with carriage returns).
fn follow(stream: Box<dyn Read + Send>, log: &Mutex<Log>) {
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let mut byte = [0u8; 1];
        let mut ended = false;
        loop {
            match reader.read(&mut byte) {
                Ok(0) | Err(_) => {
                    ended = true;
                    break;
                }
                Ok(_) if byte[0] == b'\n' || byte[0] == b'\r' => break,
                Ok(_) => buffer.push(byte[0]),
            }
        }
        let line = strip_ansi(&String::from_utf8_lossy(&buffer));
        let line = line.trim();
        if !line.is_empty() {
            let mut log = lock(log);
            if let Some(counted) = parse_progress(line) {
                log.progress = counted;
            }
            if log.lines.len() >= LOG_LINES {
                log.lines.pop_front();
            }
            log.lines.push_back(line.to_string());
        }
        if ended {
            return;
        }
    }
}

/// The last `done/total` pair in a line Remotion printed, such as `Rendered 45/180`.
fn parse_progress(line: &str) -> Option<Progress> {
    line.split(|c: char| !(c.is_ascii_digit() || c == '/'))
        .filter_map(|word| {
            let (done, total) = word.split_once('/')?;
            let (done, total) = (done.parse::<u64>().ok()?, total.parse::<u64>().ok()?);
            (total > 0 && done <= total).then_some(Progress { done, total })
        })
        .next_back()
}

/// `text` without terminal colour and cursor codes.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Stops `child` and everything it started.
fn stop(child: &mut Child) {
    #[cfg(unix)]
    if let Some(group) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::TERM);
        for _ in 0..50 {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_comes_from_counts_in_the_output() {
        assert_eq!(
            parse_progress("Rendered 45/180, time remaining: 3s"),
            Some(Progress {
                done: 45,
                total: 180
            })
        );
        assert_eq!(
            parse_progress("Bundling 100% | Rendering 3/90 | Encoding 1/90"),
            Some(Progress { done: 1, total: 90 })
        );
        assert_eq!(parse_progress("Composition: Main 1080x1920"), None);
        assert_eq!(parse_progress("date 2026/11/01"), None);
        assert_eq!(
            strip_ansi("\u{1b}[32mRendered\u{1b}[0m 1/2"),
            "Rendered 1/2"
        );
    }

    #[test]
    fn outputs_are_named_after_the_composition() {
        let dir = std::env::temp_dir().join(format!("fosscut-remotion-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = default_output(&dir, "Promo Ad/1");
        assert_eq!(first, dir.join("renders").join("Promo-Ad-1.mov"));
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::write(&first, "").unwrap();
        assert_eq!(
            default_output(&dir, "Promo Ad/1"),
            dir.join("renders").join("Promo-Ad-1-2.mov")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
