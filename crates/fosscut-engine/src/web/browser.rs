//! Finding and starting a headless browser for web compositions (ADR-0039).
//!
//! FossCut never ships a browser. It uses the one named by `FOSSCUT_BROWSER`, then the
//! `chrome-headless-shell` model when the user installed it (`fosscut models install
//! chrome-headless-shell`), then Google Chrome, Chromium, Microsoft Edge or Brave where
//! they are usually installed. Each browser runs with a fresh profile in a temporary
//! folder, its DevTools endpoint on the loopback interface only, and no network: host
//! names do not resolve, and the page's own requests are answered from its folder.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::WebError;
use super::cdp::Cdp;

/// The environment variable naming the browser to use.
pub const BROWSER_VARIABLE: &str = "FOSSCUT_BROWSER";

/// The model that installs Chrome for Testing's headless shell.
pub const HEADLESS_SHELL: &str = "chrome-headless-shell";

/// How long a browser may take to open its DevTools endpoint.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// A browser found on this machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Found {
    /// Its executable.
    pub path: PathBuf,
    /// Where it was found: `variable`, `model` or `installed`.
    pub source: &'static str,
    /// True for Chrome for Testing's headless shell, which needs no `--headless` switch.
    pub shell: bool,
}

/// The browser to use, or `None` when none is found. See the module documentation for
/// the order.
pub fn find() -> Option<Found> {
    find_in(
        std::env::var_os(BROWSER_VARIABLE),
        &crate::models_dir(),
        &installed_candidates(),
    )
}

/// [`find`] with the variable's value, the models folder and the usual places given.
pub(crate) fn find_in(
    variable: Option<std::ffi::OsString>,
    models: &Path,
    candidates: &[PathBuf],
) -> Option<Found> {
    if let Some(path) = variable.filter(|v| !v.is_empty()).map(PathBuf::from) {
        let shell = is_shell(&path);
        return Some(Found {
            path,
            source: "variable",
            shell,
        });
    }
    let shell = shell_path(models);
    if shell.is_file() {
        return Some(Found {
            path: shell,
            source: "model",
            shell: true,
        });
    }
    candidates.iter().find(|p| p.is_file()).map(|path| Found {
        path: path.clone(),
        source: "installed",
        shell: is_shell(path),
    })
}

fn is_shell(path: &Path) -> bool {
    path.file_stem()
        .is_some_and(|name| name.to_string_lossy().starts_with(HEADLESS_SHELL))
}

/// Where the installed headless shell's executable is in the models folder `models`.
pub fn shell_path(models: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "chrome-headless-shell.exe"
    } else {
        "chrome-headless-shell"
    };
    models.join(HEADLESS_SHELL).join(name)
}

/// The places browsers are usually installed on this system, most suitable first.
fn installed_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if cfg!(target_os = "macos") {
        let apps = [
            "Google Chrome.app/Contents/MacOS/Google Chrome",
            "Chromium.app/Contents/MacOS/Chromium",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "Brave Browser.app/Contents/MacOS/Brave Browser",
        ];
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        for app in apps {
            for root in &roots {
                candidates.push(root.join(app));
            }
        }
    } else if cfg!(windows) {
        let roots = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        for app in [
            "Google/Chrome/Application/chrome.exe",
            "Chromium/Application/chrome.exe",
            "Microsoft/Edge/Application/msedge.exe",
            "BraveSoftware/Brave-Browser/Application/brave.exe",
        ] {
            for root in &roots {
                candidates.push(root.join(app));
            }
        }
    } else {
        let names = [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "microsoft-edge-stable",
            "brave-browser",
        ];
        let dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        for name in names {
            for dir in &dirs {
                candidates.push(dir.join(name));
            }
        }
    }
    candidates
}

/// A running browser with its DevTools connection. Dropping it closes the browser and
/// removes its profile.
pub(crate) struct Browser {
    child: Child,
    /// On Unix the browser runs under a small shell that closes it once this pipe closes,
    /// so a browser never outlives FossCut, even when FossCut is killed.
    watchdog: Option<ChildStdin>,
    profile: PathBuf,
    pub(crate) cdp: Cdp,
}

impl Browser {
    /// Starts `found` with a fresh profile and connects to it.
    pub(crate) fn start(found: &Found) -> Result<Browser, WebError> {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let profile = std::env::temp_dir().join(format!(
            "fosscut-browser-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).map_err(|e| WebError::BrowserFailed {
            message: format!("cannot create a browser profile: {e}"),
        })?;
        let log = std::fs::File::create(profile.join("fosscut-browser.log")).map_err(|e| {
            WebError::BrowserFailed {
                message: format!("cannot create the browser log: {e}"),
            }
        })?;
        let args = arguments(found, &profile);
        let mut command = if cfg!(unix) {
            // `$@` is the browser and its arguments. A background job's standard input is
            // /dev/null unless redirected, so the pipe is passed to `cat` as descriptor 3.
            let mut command = Command::new("/bin/sh");
            command
                .arg("-c")
                .arg("exec 3<&0; \"$@\" </dev/null 3<&- & child=$!; (cat <&3 >/dev/null; kill \"$child\" 2>/dev/null) & exec 3<&-; wait \"$child\"")
                .arg("fosscut-browser")
                .arg(&found.path)
                .args(&args)
                .stdin(Stdio::piped());
            command
        } else {
            let mut command = Command::new(&found.path);
            command.args(&args).stdin(Stdio::null());
            command
        };
        let mut child = command
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .map_err(|e| WebError::BrowserFailed {
                message: format!("cannot start {}: {e}", found.path.display()),
            })?;
        let watchdog = child.stdin.take();
        let mut browser = BrowserGuard {
            child: Some(child),
            watchdog,
            profile: profile.clone(),
        };
        let url = browser.endpoint()?;
        let cdp = Cdp::connect(&url).map_err(|e| WebError::BrowserFailed {
            message: e.to_string(),
        })?;
        let (Some(child), watchdog) = (browser.child.take(), browser.watchdog.take()) else {
            return Err(WebError::BrowserFailed {
                message: "the browser stopped while connecting".into(),
            });
        };
        Ok(Browser {
            child,
            watchdog,
            profile,
            cdp,
        })
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.cdp.call(None, "Browser.close", serde_json::json!({}));
        stop(&mut self.child, self.watchdog.take(), &self.profile);
    }
}

/// Stops a browser that has not connected yet, when starting it fails.
struct BrowserGuard {
    child: Option<Child>,
    watchdog: Option<ChildStdin>,
    profile: PathBuf,
}

impl BrowserGuard {
    /// Waits for the browser to write its DevTools endpoint into the profile.
    fn endpoint(&mut self) -> Result<String, WebError> {
        let file = self.profile.join("DevToolsActivePort");
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            if let Ok(text) = std::fs::read_to_string(&file)
                && let Some((port, path)) = text.split_once('\n')
                && !path.trim().is_empty()
            {
                return Ok(format!("ws://127.0.0.1:{}{}", port.trim(), path.trim()));
            }
            let exited = self
                .child
                .as_mut()
                .and_then(|c| c.try_wait().ok().flatten())
                .is_some();
            if exited || Instant::now() > deadline {
                let mut log = String::new();
                if let Ok(mut f) = std::fs::File::open(self.profile.join("fosscut-browser.log")) {
                    let _ = f.read_to_string(&mut log);
                }
                let tail: Vec<&str> = log.lines().rev().take(5).collect();
                let tail: Vec<&str> = tail.into_iter().rev().collect();
                return Err(WebError::BrowserFailed {
                    message: format!(
                        "the browser {} before it was ready{}{}",
                        if exited {
                            "exited"
                        } else {
                            "did not start in time"
                        },
                        if tail.is_empty() { "" } else { ": " },
                        tail.join(" / ")
                    ),
                });
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for BrowserGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            stop(&mut child, self.watchdog.take(), &self.profile);
        }
    }
}

/// Closes the watchdog pipe (which ends the browser on Unix), kills the browser if it is
/// still running after a moment, and removes its profile.
fn stop(child: &mut Child, watchdog: Option<ChildStdin>, profile: &Path) {
    drop(watchdog);
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().ok().flatten().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    for _ in 0..10 {
        if std::fs::remove_dir_all(profile).is_ok() || !profile.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The browser's command line: headless, its own profile, an ephemeral loopback DevTools
/// port, sRGB output at a device scale of 1, software rendering, and no network, sync,
/// extensions, first-run pages or keychain prompts.
fn arguments(found: &Found, profile: &Path) -> Vec<String> {
    let mut args = Vec::new();
    if !found.shell {
        args.push("--headless=new".to_string());
    }
    args.extend(
        [
            "--remote-debugging-port=0",
            "--remote-debugging-address=127.0.0.1",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-component-extensions-with-background-pages",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-default-apps",
            "--disable-sync",
            "--disable-breakpad",
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
            "--disable-features=Translate,MediaRouter,OptimizationHints,DialMediaRouteProvider",
            "--metrics-recording-only",
            "--no-pings",
            "--mute-audio",
            "--hide-scrollbars",
            "--force-color-profile=srgb",
            "--force-device-scale-factor=1",
            "--font-render-hinting=none",
            "--disable-lcd-text",
            "--use-mock-keychain",
            "--password-store=basic",
            "--use-angle=swiftshader",
            "--enable-unsafe-swiftshader",
            "--host-resolver-rules=MAP * ~NOTFOUND",
        ]
        .map(String::from),
    );
    args.push(format!("--user-data-dir={}", profile.display()));
    args.push("about:blank".to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_comes_first_then_the_model_then_installed_browsers() {
        let dir = std::env::temp_dir().join(format!("fosscut-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("apps")).unwrap();
        let chrome = dir.join("apps").join("chrome");
        std::fs::write(&chrome, "").unwrap();
        let models = dir.join("models");
        let candidates = vec![dir.join("apps").join("missing"), chrome.clone()];

        let found = find_in(None, &models, &candidates).unwrap();
        assert_eq!(
            (found.path, found.source, found.shell),
            (chrome.clone(), "installed", false)
        );
        assert_eq!(find_in(None, &models, &[]), None);

        let shell = shell_path(&models);
        std::fs::create_dir_all(shell.parent().unwrap()).unwrap();
        std::fs::write(&shell, "").unwrap();
        let found = find_in(None, &models, &candidates).unwrap();
        assert_eq!(
            (found.path, found.source, found.shell),
            (shell, "model", true)
        );

        let named = find_in(
            Some("/opt/x/chrome-headless-shell".into()),
            &models,
            &candidates,
        )
        .unwrap();
        assert_eq!((named.source, named.shell), ("variable", true));
        let named = find_in(Some("".into()), &models, &[]).unwrap();
        assert_eq!(named.source, "model", "an empty variable is ignored");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regular_browsers_run_headless_without_network() {
        let found = Found {
            path: "chrome".into(),
            source: "installed",
            shell: false,
        };
        let args = arguments(&found, Path::new("/tmp/p"));
        assert_eq!(args[0], "--headless=new");
        assert!(args.contains(&"--host-resolver-rules=MAP * ~NOTFOUND".to_string()));
        assert!(args.contains(&"--user-data-dir=/tmp/p".to_string()));
        let shell = Found {
            shell: true,
            ..found
        };
        assert!(
            !arguments(&shell, Path::new("/tmp/p"))
                .iter()
                .any(|a| a.starts_with("--headless"))
        );
    }
}
