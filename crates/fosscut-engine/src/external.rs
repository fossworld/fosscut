//! Running programs the user installed themselves (ADR-0039 item 6, ADR-0043): the
//! user's own Remotion and the user's own agent. FossCut never installs or bundles them; it
//! finds them, shows the exact command first, and runs it only once the user agreed.
//!
//! - **Finding** ([`find_program`]): a path to a file, or a name looked up in the folders
//!   of `PATH` and then in the program's usual install places, since apps started from the
//!   dock have a short `PATH`. An environment variable may name the program instead
//!   ([`overridden`]).
//! - **Starting** ([`command`]): on Unix a small shell leads a process group of its own
//!   with the program in it, and ends the whole group (the program and everything it
//!   started) once its standard input closes: when the run is cancelled, or FossCut exits or
//!   is killed.
//! - **Following** ([`run`]): standard output and error are read line by line (lines end at
//!   line feeds or carriage returns, for progress bars), without terminal colour codes; a
//!   handler sees every line and chooses what the log keeps, and the last [`LOG_LINES`]
//!   kept lines come back with the outcome.
//! - **Stopping** ([`stop`]): the process group is asked to end, then killed.

use std::collections::VecDeque;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Lines of a program's output kept for errors.
pub const LOG_LINES: usize = 30;

/// How often [`run`] checks on the program and calls its `tick`.
const TICK: Duration = Duration::from_millis(100);

/// The output stream a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// Standard output.
    Out,
    /// Standard error.
    Err,
}

/// Sees every line a program prints and answers what the log keeps of it, if anything.
pub type Lines = Box<dyn FnMut(Stream, &str) -> Option<String> + Send>;

/// A program that ran to its end.
#[derive(Debug)]
pub struct Finished {
    /// How it exited.
    pub status: ExitStatus,
    /// The last lines the handler kept, oldest first.
    pub log: Vec<String>,
}

/// Why [`run`] did not see the program to its end.
#[derive(Debug)]
pub enum RunError {
    /// The program could not be started.
    Spawn(std::io::Error),
    /// The run was cancelled and the program's process group stopped.
    Cancelled,
    /// The program could no longer be waited for; it was stopped.
    Lost(std::io::Error),
}

/// The value of the environment variable `variable`, as a program, when it is set and not
/// empty.
pub fn overridden(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// `program` itself when it is a path to a file, otherwise the first file of that name in
/// the folders of `PATH`, then in `places`.
pub fn find_program(program: &Path, places: &[PathBuf]) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return program.is_file().then(|| program.to_path_buf());
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(places.iter().cloned())
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// A command that runs `program` with `args` so that [`run`] and [`stop`] can end it with
/// everything it starts. `name` labels the shell that leads the group on Unix. Standard
/// output and error are piped; the caller sets the folder and environment.
pub fn command(program: &Path, args: &[String], name: &str) -> Command {
    let mut command = if cfg!(unix) {
        // A small shell leads a process group of its own with the program in it, and ends
        // the whole group once its standard input closes: when the run is cancelled, or
        // FossCut exits or is killed. A background job's standard input is /dev/null unless
        // redirected, so the pipe is passed to `cat` as descriptor 3.
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("exec 3<&0; \"$@\" </dev/null 3<&- & child=$!; (cat <&3; kill -TERM 0) >/dev/null 2>&1 & exec 3<&-; wait \"$child\"")
            .arg(name)
            .arg(program)
            .args(args)
            .stdin(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command
    } else {
        let mut command = Command::new(program);
        command.args(args).stdin(Stdio::null());
        command
    };
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command
}

/// What the readers share: the handler and the kept lines.
struct Shared {
    lines: Lines,
    log: VecDeque<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs `command` (made by [`command`]) until it exits or is cancelled. Every line it
/// prints goes to `lines`. About every 100 ms `tick` is called, and the program is stopped
/// with its process group when `tick` answers `false` or `cancel` is set; `tick` is called
/// once more after the program ended and its output was read, and its answer is then
/// ignored.
pub fn run(
    mut command: Command,
    cancel: &AtomicBool,
    lines: Lines,
    tick: &mut dyn FnMut() -> bool,
) -> Result<Finished, RunError> {
    let mut child = command.spawn().map_err(RunError::Spawn)?;
    let watchdog = child.stdin.take();
    let shared = Arc::new(Mutex::new(Shared {
        lines,
        log: VecDeque::new(),
    }));
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| (Stream::Out, Box::new(s) as Box<dyn Read + Send>)),
        child
            .stderr
            .take()
            .map(|s| (Stream::Err, Box::new(s) as Box<dyn Read + Send>)),
    ]
    .into_iter()
    .flatten()
    .map(|(which, stream)| {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || follow(which, stream, &shared))
    })
    .collect();
    let status = loop {
        let go_on = tick();
        if !go_on || cancel.load(Ordering::Relaxed) {
            drop(watchdog);
            stop(&mut child);
            for reader in readers {
                let _ = reader.join();
            }
            return Err(RunError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(TICK),
            Err(e) => {
                drop(watchdog);
                stop(&mut child);
                return Err(RunError::Lost(e));
            }
        }
    };
    // Ends what is left of the group, then the output pipes close.
    drop(watchdog);
    for reader in readers {
        let _ = reader.join();
    }
    let _ = tick();
    let log = lock(&shared).log.iter().cloned().collect();
    Ok(Finished { status, log })
}

/// Reads `stream` line by line (progress bars end lines with carriage returns).
fn follow(which: Stream, stream: Box<dyn Read + Send>, shared: &Mutex<Shared>) {
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
            let mut shared = lock(shared);
            if let Some(kept) = (shared.lines)(which, line) {
                if shared.log.len() >= LOG_LINES {
                    shared.log.pop_front();
                }
                shared.log.push_back(kept);
            }
        }
        if ended {
            return;
        }
    }
}

/// `text` without terminal colour and cursor codes.
pub fn strip_ansi(text: &str) -> String {
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
pub fn stop(child: &mut Child) {
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
    fn programs_are_files_given_by_path_or_found_on_the_path_and_in_places() {
        let dir = std::env::temp_dir().join(format!("fosscut-external-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("fosscut-test-tool");
        std::fs::write(&tool, "").unwrap();
        assert_eq!(find_program(&tool, &[]), Some(tool.clone()));
        assert_eq!(find_program(&dir.join("missing"), &[]), None);
        assert_eq!(
            find_program(Path::new("fosscut-test-tool"), std::slice::from_ref(&dir)),
            Some(tool.clone())
        );
        assert_eq!(find_program(Path::new("fosscut-test-tool"), &[]), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn handlers_choose_what_the_log_keeps_and_ticks_end_with_the_program() {
        let script = "printf 'one\\n\\033[32mtwo\\033[0m\\rthree\\n'; echo oops >&2; exit 3";
        let command = command(
            Path::new("/bin/sh"),
            &["-c".into(), script.into()],
            "fosscut-test",
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let saw = Arc::clone(&seen);
        let mut ticks = 0;
        let finished = run(
            command,
            &AtomicBool::new(false),
            Box::new(move |which, line| {
                lock(&saw).push((which, line.to_string()));
                (line != "two").then(|| line.to_uppercase())
            }),
            &mut || {
                ticks += 1;
                true
            },
        )
        .unwrap();
        assert_eq!(finished.status.code(), Some(3));
        let mut log = finished.log.clone();
        log.sort();
        assert_eq!(log, ["ONE", "OOPS", "THREE"]);
        assert!(lock(&seen).contains(&(Stream::Err, "oops".to_string())));
        assert!(lock(&seen).contains(&(Stream::Out, "two".to_string())));
        assert!(ticks >= 2, "a tick while running and one after");
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_cannot_start_or_is_cancelled_says_so() {
        let missing = run(
            Command::new("/no/such/program"),
            &AtomicBool::new(false),
            Box::new(|_, _| None),
            &mut || true,
        );
        assert!(matches!(missing, Err(RunError::Spawn(_))));
        let command = command(
            Path::new("/bin/sh"),
            &["-c".into(), "sleep 30".into()],
            "fosscut-test",
        );
        let started = std::time::Instant::now();
        let cancelled = run(
            command,
            &AtomicBool::new(true),
            Box::new(|_, _| None),
            &mut || true,
        );
        assert!(matches!(cancelled, Err(RunError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
