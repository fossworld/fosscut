//! Web compositions (ADR-0039): HTML pages, with the files they load, imported as `web`
//! assets and shown by video clips, rendered frame by frame in a headless browser.
//!
//! - **Import** ([`media_add_args`]): a folder with an `index.html`, or one `.html` file, is
//!   copied into the bundle's `web/` folder under the hash of its content, and becomes the
//!   arguments of `media.add`. Its size, frame rate and length come from the arguments or
//!   from a `<meta name="fosscut:composition">` tag in the page ([`META_NAME`]).
//! - **Frames** ([`WebDecoder`]): the page is loaded once per decoder in a browser
//!   ([`browser`]) with FossCut's clock, and each frame is a screenshot with a transparent
//!   background. Preview, frame grabs and exports all decode it this way, so they agree.
//! - **Remotion** ([`crate::remotion`]) renders a composition of the user's own Remotion
//!   project into the bundle, after the user confirms, and imports the file as video.

pub mod browser;
mod cdp;
mod decoder;

use std::io::Read;
use std::path::{Path, PathBuf};

use fosscut_core::{ApiError, ErrorCode};
use fosscut_media::VideoDecoder;
use fosscut_model::{
    Asset, AssetKind, ContentIdentity, Id, MediaInfo, PathHints, Project, VideoInfo,
};
use fosscut_time::{Flicks, Rate, TimeInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::EngineError;

pub use browser::{BROWSER_VARIABLE, Found, HEADLESS_SHELL, find as find_browser};
pub use decoder::WebDecoder;

/// The `name` of the meta tag that describes a composition, for example
/// `<meta name="fosscut:composition" content="width=1080, height=1920, fps=30, duration=6">`.
/// `fps` is a whole number or a fraction such as `30000/1001`; `duration` is in seconds, or
/// with a unit (`6s`, `180f`).
pub const META_NAME: &str = "fosscut:composition";

/// The bundle folder compositions are copied into.
pub const WEB_DIR: &str = "web";

/// Largest canvas side of a composition, in pixels.
pub const MAX_SIDE: u32 = 8192;

/// Most files and bytes a composition folder may hold.
const MAX_FILES: usize = 10_000;
const MAX_BYTES: u64 = 1 << 30;

/// Why a web composition could not be imported or shown, or a Remotion render failed.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum WebError {
    /// No browser to render compositions with was found.
    #[error("no Chrome, Chromium, Edge or Brave browser was found to render web compositions")]
    BrowserMissing {
        /// The places looked at.
        tried: Vec<String>,
    },
    /// The browser could not be started or stopped answering.
    #[error("the browser failed: {message}")]
    BrowserFailed {
        /// What went wrong.
        message: String,
    },
    /// The page failed to load, threw, or could not be captured.
    #[error("the web composition failed: {message}")]
    PageFailed {
        /// What went wrong.
        message: String,
    },
    /// The path is not a web composition.
    #[error("{path} is not a web composition: {message}")]
    NotComposition {
        /// The path given.
        path: String,
        /// Why.
        message: String,
    },
    /// The composition's size, rate or length is neither given nor in its meta tag.
    #[error("the composition's {} is not known", missing.join(", "))]
    CompositionUnknown {
        /// The values missing: `width`, `height`, `duration`.
        missing: Vec<String>,
    },
    /// A value of the composition is out of range or malformed.
    #[error("invalid composition: {message}")]
    InvalidComposition {
        /// What is wrong.
        message: String,
    },
    /// The page loads files from the network, which compositions cannot do.
    #[error("the page loads {} from the network", references.join(", "))]
    RemoteReference {
        /// The addresses found.
        references: Vec<String>,
    },
    /// The folder holds too many files or bytes to copy into the bundle.
    #[error(
        "the composition folder holds {files} files and {bytes} bytes, more than a composition may"
    )]
    TooLarge {
        /// Files counted.
        files: usize,
        /// Bytes counted.
        bytes: u64,
    },
    /// The folder is not a Remotion project.
    #[error("{path} is not a Remotion project: {message}")]
    NotRemotion {
        /// The folder given.
        path: String,
        /// Why.
        message: String,
    },
    /// Remotion, or `npx` to run it, is not installed for the project.
    #[error("{message}")]
    RemotionUnavailable {
        /// What is missing.
        message: String,
    },
    /// Running Remotion needs the user's confirmation first.
    #[error("rendering with Remotion needs confirmation: {licence}")]
    RemotionConfirmation {
        /// The licence note to tell the user.
        licence: String,
        /// The command that would run.
        command: Vec<String>,
    },
    /// Remotion failed.
    #[error("Remotion failed: {message}")]
    RemotionFailed {
        /// What went wrong.
        message: String,
        /// The last lines Remotion printed.
        log: Vec<String>,
    },
}

impl WebError {
    /// The stable error code (ADR-0006).
    pub fn code(&self) -> &'static str {
        match self {
            WebError::BrowserMissing { .. } => "fosscut.web.browser_missing",
            WebError::BrowserFailed { .. } => "fosscut.web.browser_failed",
            WebError::PageFailed { .. } => "fosscut.web.page_failed",
            WebError::NotComposition { .. } => "fosscut.web.not_composition",
            WebError::CompositionUnknown { .. } => "fosscut.web.composition_unknown",
            WebError::InvalidComposition { .. } => "fosscut.web.invalid_composition",
            WebError::RemoteReference { .. } => "fosscut.web.remote_reference",
            WebError::TooLarge { .. } => "fosscut.web.too_large",
            WebError::NotRemotion { .. } => "fosscut.web.not_remotion",
            WebError::RemotionUnavailable { .. } => "fosscut.web.remotion_unavailable",
            WebError::RemotionConfirmation { .. } => "fosscut.web.confirmation_required",
            WebError::RemotionFailed { .. } => "fosscut.web.remotion_failed",
        }
    }

    /// What to do about it.
    pub fn hint(&self) -> Option<String> {
        let text = match self {
            WebError::BrowserMissing { .. } => format!(
                "Install Google Chrome, Chromium, Microsoft Edge or Brave, or set {BROWSER_VARIABLE} to one. Or ask the user before installing Chrome for Testing's headless shell, since it downloads: models install {HEADLESS_SHELL} (fosscut models install {HEADLESS_SHELL})."
            ),
            WebError::BrowserFailed { .. } => format!(
                "Check that the browser starts on its own, or point {BROWSER_VARIABLE} at another one."
            ),
            WebError::PageFailed { .. } => "Open the page in a browser to find the error. Compositions load files from their own folder only; window.fosscut.seek must not throw and its promise must settle.".into(),
            WebError::NotComposition { .. } => "Give a folder with an index.html, or one .html file.".into(),
            WebError::CompositionUnknown { .. } => format!(
                "Add <meta name=\"{META_NAME}\" content=\"width=1080, height=1920, fps=30, duration=6\"> to the page, or pass width, height, fps and duration."
            ),
            WebError::InvalidComposition { .. } => format!(
                "Width and height are 1 to {MAX_SIDE} pixels, fps a whole number or fraction from 1 to 240, and duration positive and at most an hour."
            ),
            WebError::RemoteReference { .. } => "Download those files into the composition's folder and refer to them by relative paths: compositions render offline, from their own folder only.".into(),
            WebError::TooLarge { .. } => "Import a folder with only the page and the files it loads (no node_modules or source trees).".into(),
            WebError::NotRemotion { .. } => "Give the folder whose package.json depends on remotion.".into(),
            WebError::RemotionUnavailable { .. } => "Ask the user to install the project's dependencies (npm install in the project) and Node.js; FossCut never installs Remotion.".into(),
            WebError::RemotionConfirmation { .. } => "Tell the user that this runs Remotion from their own project under Remotion's licence (quoted in the message); run it with --yes (CLI) or confirm: true (MCP) only if they agree.".into(),
            WebError::RemotionFailed { .. } => "Read details.log; check that the composition id exists (npx remotion compositions in the project) and that its props are valid.".into(),
        };
        Some(text)
    }

    /// Machine-readable details.
    pub fn details(&self) -> Value {
        match self {
            WebError::BrowserMissing { tried } => json!({ "tried": tried }),
            WebError::NotComposition { path, .. } | WebError::NotRemotion { path, .. } => {
                json!({ "path": path })
            }
            WebError::CompositionUnknown { missing } => json!({ "missing": missing }),
            WebError::RemoteReference { references } => json!({ "references": references }),
            WebError::TooLarge { files, bytes } => json!({ "files": files, "bytes": bytes }),
            WebError::RemotionConfirmation { licence, command } => {
                json!({ "licence": licence, "command": command })
            }
            WebError::RemotionFailed { log, .. } => json!({ "log": log }),
            _ => Value::Null,
        }
    }
}

impl From<WebError> for EngineError {
    fn from(error: WebError) -> EngineError {
        EngineError::Web(error)
    }
}

impl From<WebError> for ApiError {
    fn from(error: WebError) -> ApiError {
        ApiError::from(EngineError::Web(error))
    }
}

/// Every `fosscut.web.*` error code with what it means and what to do.
pub fn error_catalogue() -> Vec<ErrorCode> {
    let entry = |code, meaning, hint| ErrorCode {
        code,
        meaning,
        hint,
    };
    vec![
        entry(
            "fosscut.web.browser_missing",
            "No Chrome, Chromium, Edge or Brave was found to render web compositions.",
            "Install one, set FOSSCUT_BROWSER, or ask the user before models install chrome-headless-shell.",
        ),
        entry(
            "fosscut.web.browser_failed",
            "The browser could not be started, or stopped answering.",
            "Check that it starts on its own, or point FOSSCUT_BROWSER at another one.",
        ),
        entry(
            "fosscut.web.page_failed",
            "A web composition failed to load, threw, or could not be captured.",
            "Open the page in a browser to find the error; it may load files from its own folder only.",
        ),
        entry(
            "fosscut.web.not_composition",
            "The path is neither a folder with an index.html nor an .html file.",
            "Give a folder with an index.html, or one .html file.",
        ),
        entry(
            "fosscut.web.composition_unknown",
            "The composition's width, height or duration is neither given nor in its fosscut:composition meta tag.",
            "Add the meta tag to the page, or pass the missing values (details.missing).",
        ),
        entry(
            "fosscut.web.invalid_composition",
            "A composition's size, frame rate or duration is out of range.",
            "Use 1 to 8192 pixels, 1 to 240 frames per second, and a positive duration of at most an hour.",
        ),
        entry(
            "fosscut.web.remote_reference",
            "The page loads scripts, styles, fonts or pictures from the network; compositions render offline.",
            "Copy the files in details.references into the folder and refer to them by relative paths.",
        ),
        entry(
            "fosscut.web.too_large",
            "The composition folder holds more than 10000 files or 1 GiB.",
            "Import a folder with only the page and what it loads.",
        ),
        entry(
            "fosscut.web.not_remotion",
            "The folder is not a Remotion project: no package.json that depends on remotion.",
            "Give the Remotion project's folder.",
        ),
        entry(
            "fosscut.web.remotion_unavailable",
            "Node.js's npx, or the project's installed Remotion, was not found. FossCut never installs Remotion.",
            "Ask the user to install Node.js and the project's dependencies (npm install).",
        ),
        entry(
            "fosscut.web.confirmation_required",
            "Rendering with Remotion runs the user's own Remotion under its licence and needs their confirmation.",
            "Tell the user the licence note in details.licence; confirm only if they agree.",
        ),
        entry(
            "fosscut.web.remotion_failed",
            "Remotion exited with an error or wrote no file.",
            "Read details.log; check the composition id and its props.",
        ),
    ]
}

/// A composition's canvas, frame rate and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Composition {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Frames per second.
    pub rate: Rate,
    /// Length, a whole number of frames.
    pub duration: Flicks,
}

impl Composition {
    /// Number of frames.
    pub fn frames(&self) -> i64 {
        self.rate.nearest_frame(self.duration)
    }

    /// The composition of a web asset, from its media information.
    pub fn of(asset: &Asset) -> Option<Composition> {
        let video = asset.info.video.as_ref()?;
        Some(Composition {
            width: video.width,
            height: video.height,
            rate: video.rate?,
            duration: asset.info.duration?,
        })
    }
}

/// A composition's values as given by a caller or a meta tag; missing ones come from the
/// other source.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompositionOptions {
    /// Canvas width in pixels.
    #[serde(default)]
    pub width: Option<u32>,
    /// Canvas height in pixels.
    #[serde(default)]
    pub height: Option<u32>,
    /// Frames per second: a whole number or a fraction such as `30000/1001`; 30 when
    /// neither given nor in the page.
    #[serde(default)]
    pub fps: Option<String>,
    /// Length, for example `{"seconds": 6}` or `{"frames": 180}` (`6s` or `180f` on the
    /// command line).
    #[serde(default)]
    pub duration: Option<TimeInput>,
}

impl CompositionOptions {
    /// These values, with those missing taken from `other`.
    fn or(self, other: CompositionOptions) -> CompositionOptions {
        CompositionOptions {
            width: self.width.or(other.width),
            height: self.height.or(other.height),
            fps: self.fps.or(other.fps),
            duration: self.duration.or(other.duration),
        }
    }

    /// The composition these values describe.
    pub fn resolve(&self) -> Result<Composition, WebError> {
        let missing: Vec<String> = [
            ("width", self.width.is_none()),
            ("height", self.height.is_none()),
            ("duration", self.duration.is_none()),
        ]
        .into_iter()
        .filter(|(_, missing)| *missing)
        .map(|(name, _)| name.to_string())
        .collect();
        if !missing.is_empty() {
            return Err(WebError::CompositionUnknown { missing });
        }
        let invalid = |message: String| WebError::InvalidComposition { message };
        let (width, height) = (self.width.unwrap_or(0), self.height.unwrap_or(0));
        for (name, side) in [("width", width), ("height", height)] {
            if !(1..=MAX_SIDE).contains(&side) {
                return Err(invalid(format!(
                    "{name} {side} is not 1 to {MAX_SIDE} pixels"
                )));
            }
        }
        let fps = self.fps.as_deref().unwrap_or("30");
        let rate: Rate = fps
            .parse()
            .map_err(|e| invalid(format!("fps {fps}: {e}")))?;
        let per_second = f64::from(rate.num()) / f64::from(rate.den());
        if !(1.0..=240.0).contains(&per_second) {
            return Err(invalid(format!("fps {fps} is not 1 to 240")));
        }
        let duration = self
            .duration
            .as_ref()
            .map(|d| d.resolve(rate))
            .transpose()
            .map_err(|e| invalid(format!("duration: {e}")))?
            .unwrap_or(Flicks::ZERO);
        let frames = rate.nearest_frame(duration);
        if frames < 1 || duration > Flicks::from_millis(3_600_000) {
            return Err(invalid(format!(
                "duration {} s is not at least one frame and at most an hour",
                duration.to_seconds_string()
            )));
        }
        Ok(Composition {
            width,
            height,
            rate,
            duration: rate.frames_to_flicks(frames),
        })
    }
}

/// The values of the first `<meta name="fosscut:composition" content="...">` tag in `html`.
/// Unknown keys are ignored; a bare duration is in seconds.
pub fn parse_meta(html: &str) -> Result<CompositionOptions, WebError> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find("<meta") {
        let start = from + found;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        let tag = &html[start..end];
        from = end;
        if attribute(tag, "name").is_some_and(|n| n.eq_ignore_ascii_case(META_NAME))
            && let Some(content) = attribute(tag, "content")
        {
            return parse_content(&content);
        }
    }
    Ok(CompositionOptions::default())
}

fn parse_content(content: &str) -> Result<CompositionOptions, WebError> {
    let invalid = |message: String| WebError::InvalidComposition { message };
    let mut options = CompositionOptions::default();
    for pair in content
        .split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(invalid(format!("{META_NAME}: {pair:?} is not key=value")));
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        let number = |name: &str| {
            value.parse::<u32>().map_err(|_| {
                invalid(format!(
                    "{META_NAME}: {name} {value:?} is not a whole number"
                ))
            })
        };
        match key.as_str() {
            "width" => options.width = Some(number("width")?),
            "height" => options.height = Some(number("height")?),
            "fps" | "rate" => options.fps = Some(value.to_string()),
            "duration" => {
                let unit = value.ends_with(['s', 'S', 'f', 'F']) || value.contains(':');
                let text = if unit {
                    value.to_string()
                } else {
                    format!("{value}s")
                };
                options.duration = Some(
                    text.parse()
                        .map_err(|e| invalid(format!("{META_NAME}: duration {value:?}: {e}")))?,
                );
            }
            _ => {}
        }
    }
    Ok(options)
}

/// The value of attribute `name` in the start tag `tag`, unquoted.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let start = from + found;
        from = start + name.len();
        let before = lower[..start].chars().next_back();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(quote @ ('"' | '\'')) => rest[1..].split(quote).next().unwrap_or_default(),
            _ => rest.split(char::is_whitespace).next().unwrap_or_default(),
        };
        return Some(value.to_string());
    }
    None
}

/// Addresses on the network that `html` loads scripts, styles, fonts, pictures or modules
/// from (links to follow are not counted).
pub fn remote_references(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(position) = ["http://", "https://"]
        .iter()
        .filter_map(|scheme| lower[from..].find(scheme).map(|p| from + p))
        .min()
    {
        from = position + 4;
        let before: String = lower[..position]
            .chars()
            .rev()
            .take(16)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let before = before.trim_end_matches(['"', '\'', ' ', '\n', '\t']);
        let loads = [
            "src=", "srcset=", "poster=", "url(", "from", "import(", "@import", "import",
        ]
        .iter()
        .any(|cue| before.ends_with(cue));
        let link = before.ends_with("href=") && {
            let tag = lower[..position]
                .rfind('<')
                .map_or("", |t| &lower[t..position]);
            tag.starts_with("<link")
        };
        if loads || link {
            let address: String = html[position..]
                .chars()
                .take_while(|c| !matches!(c, '"' | '\'' | ')' | ' ' | '>' | '\n'))
                .collect();
            if !found.contains(&address) {
                found.push(address);
            }
        }
    }
    found
}

/// True when `path` is a web composition: a folder with an `index.html`, or an `.html` or
/// `.htm` file.
pub fn is_composition(path: &Path) -> bool {
    if path.is_dir() {
        return path.join("index.html").is_file();
    }
    path.is_file() && is_html(path)
}

fn is_html(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
}

/// The arguments of a `media.add` command that imports the composition at `source` (a
/// folder with an `index.html`, or one `.html` file) into the project at `bundle`, named
/// `name` or after the folder or file. `options` override the page's meta tag. The files
/// are copied into `web/<hash>/` in the bundle first, once per content.
pub fn media_add_args(
    source: &Path,
    name: Option<&str>,
    bundle: &Path,
    options: &CompositionOptions,
) -> Result<Value, ApiError> {
    let (root, entry, files) = collect(source)?;
    let html =
        std::fs::read_to_string(root.join(&entry)).map_err(|e| WebError::NotComposition {
            path: source.display().to_string(),
            message: format!("cannot read {entry}: {e}"),
        })?;
    let composition = options.clone().or(parse_meta(&html)?).resolve()?;
    let references = remote_references(&html);
    if !references.is_empty() {
        return Err(WebError::RemoteReference { references }.into());
    }
    let identity = identity(&root, &files)?;
    let folder = bundle.join(WEB_DIR).join(&identity.sample_blake3[..16]);
    copy_into(&root, &files, &folder)?;
    let name = name.map(str::to_string).unwrap_or_else(|| {
        source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "composition".into())
    });
    let copied = folder.join(&entry);
    let mut path = crate::import::path_hints(&copied, bundle)?;
    if path.relative.is_none() {
        path = PathHints {
            relative: Some(format!(
                "{WEB_DIR}/{}/{entry}",
                &identity.sample_blake3[..16]
            )),
            ..path
        };
    }
    let info = MediaInfo {
        duration: Some(composition.duration),
        container: Some("html".into()),
        video: Some(VideoInfo {
            width: composition.width,
            height: composition.height,
            rate: Some(composition.rate),
            codec: Some("html".into()),
            rotation: 0,
        }),
        ..MediaInfo::default()
    };
    Ok(json!({
        "name": name,
        "kind": AssetKind::Web,
        "path": path,
        "identity": identity,
        "info": info,
    }))
}

/// The folder to copy, the page's file name in it, and every file to copy (relative paths
/// with `/`, sorted).
fn collect(source: &Path) -> Result<(PathBuf, String, Vec<String>), WebError> {
    let not = |message: &str| WebError::NotComposition {
        path: source.display().to_string(),
        message: message.into(),
    };
    if source.is_dir() {
        if !source.join("index.html").is_file() {
            return Err(not("the folder has no index.html"));
        }
        let mut files = Vec::new();
        let mut bytes = 0;
        walk(source, "", &mut files, &mut bytes)?;
        files.sort();
        return Ok((source.to_path_buf(), "index.html".into(), files));
    }
    if !source.is_file() {
        return Err(not("it does not exist"));
    }
    if !is_html(source) {
        return Err(not("it is not an .html file"));
    }
    let entry = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| not("it has no file name"))?;
    let root = source.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok((root, entry.clone(), vec![entry]))
}

/// Lists the files under `dir` (at `prefix` from the root), skipping hidden entries and
/// `node_modules`.
fn walk(
    dir: &Path,
    prefix: &str,
    files: &mut Vec<String>,
    bytes: &mut u64,
) -> Result<(), WebError> {
    let failed = |e: std::io::Error| WebError::NotComposition {
        path: dir.display().to_string(),
        message: format!("cannot read the folder: {e}"),
    };
    for entry in std::fs::read_dir(dir).map_err(failed)? {
        let entry = entry.map_err(failed)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = entry.path();
        let relative = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if path.is_dir() {
            walk(&path, &relative, files, bytes)?;
        } else if path.is_file() {
            *bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            files.push(relative);
            if files.len() > MAX_FILES || *bytes > MAX_BYTES {
                return Err(WebError::TooLarge {
                    files: files.len(),
                    bytes: *bytes,
                });
            }
        }
    }
    Ok(())
}

/// The composition's content identity: its total size, and a BLAKE3 hash of every file's
/// relative path, size and bytes in path order.
fn identity(root: &Path, files: &[String]) -> Result<ContentIdentity, WebError> {
    let mut hasher = blake3::Hasher::new();
    let mut total = 0u64;
    let mut buffer = vec![0u8; 1 << 16];
    for file in files {
        let path = root.join(file);
        let failed = |e: std::io::Error| WebError::NotComposition {
            path: path.display().to_string(),
            message: format!("cannot read it: {e}"),
        };
        let mut reader = std::fs::File::open(&path).map_err(failed)?;
        let size = reader.metadata().map_err(failed)?.len();
        hasher.update(file.as_bytes());
        hasher.update(&[0]);
        hasher.update(&size.to_le_bytes());
        loop {
            let n = reader.read(&mut buffer).map_err(failed)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        total += size;
    }
    Ok(ContentIdentity {
        size: total,
        sample_blake3: hasher.finalize().to_hex().to_string(),
    })
}

/// Copies `files` from `root` into `folder`, unless it is there already: into a hidden
/// folder beside it first, which is renamed into place when complete.
fn copy_into(root: &Path, files: &[String], folder: &Path) -> Result<(), WebError> {
    if folder.is_dir() {
        return Ok(());
    }
    let parent = folder.parent().unwrap_or(folder);
    let failed = |e: std::io::Error| WebError::NotComposition {
        path: folder.display().to_string(),
        message: format!("cannot copy into the bundle: {e}"),
    };
    std::fs::create_dir_all(parent).map_err(failed)?;
    let staging = parent.join(format!(
        ".{}.partial",
        folder
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    let _ = std::fs::remove_dir_all(&staging);
    let result = (|| {
        for file in files {
            let target = file
                .split('/')
                .fold(staging.clone(), |path, part| path.join(part));
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::copy(root.join(file), &target)?;
        }
        std::fs::rename(&staging, folder)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging);
        if folder.is_dir() {
            return Ok(());
        }
        return Err(failed(error));
    }
    Ok(())
}

/// Opens the frames of `asset`: a [`WebDecoder`] for a web composition, FFmpeg for any
/// other media file at `path` with `options`.
pub(crate) fn open_decoder(
    project: &Project,
    asset: &Id,
    path: &Path,
    options: fosscut_media::DecoderOptions,
) -> Result<Box<dyn VideoDecoder + Send>, EngineError> {
    match project.media.get(asset) {
        Some(found) if found.kind == AssetKind::Web => {
            let composition = Composition::of(found).ok_or_else(|| WebError::PageFailed {
                message: format!("asset {asset} has no size, frame rate or duration"),
            })?;
            let browser = browser::find().ok_or_else(|| WebError::BrowserMissing {
                tried: tried_places(),
            })?;
            Ok(Box::new(WebDecoder::open(path, composition, &browser)?))
        }
        _ => Ok(Box::new(fosscut_media::ffmpeg::FfmpegVideoDecoder::open(
            path, options,
        )?)),
    }
}

fn tried_places() -> Vec<String> {
    vec![
        format!("${BROWSER_VARIABLE}"),
        browser::shell_path(&crate::models_dir())
            .display()
            .to_string(),
        "Google Chrome, Chromium, Microsoft Edge and Brave in their usual places".into(),
    ]
}

/// What `capabilities` reports about web compositions on this machine: the browser that
/// would render them, if any.
pub fn status() -> Value {
    json!({
        "browser": browser::find(),
        "meta_tag": META_NAME,
        "remotion": "fosscut web remotion <project> <composition> --yes renders the user's own Remotion project after confirmation",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meta_tag_describes_the_composition() {
        let html = r#"<html><head><meta charset="utf-8">
            <META name="fosscut:composition" content="width=1080, height=1920; fps=30000/1001, duration=6.5">
            </head></html>"#;
        let options = parse_meta(html).unwrap();
        assert_eq!(options.width, Some(1080));
        assert_eq!(options.height, Some(1920));
        assert_eq!(options.fps.as_deref(), Some("30000/1001"));
        let composition = options.resolve().unwrap();
        assert_eq!(composition.rate, Rate::NTSC_30);
        assert_eq!(composition.frames(), 195);
        assert_eq!(composition.duration, Rate::NTSC_30.frames_to_flicks(195));

        let frames = parse_meta(r#"<meta content='width=10 height=10' name='x'><meta name='fosscut:composition' content='width=64,height=32,duration=90f'>"#).unwrap();
        let composition = frames.resolve().unwrap();
        assert_eq!(
            (composition.width, composition.frames()),
            (64, 90),
            "fps defaults to 30"
        );

        assert_eq!(
            parse_meta("<p>no tag</p>").unwrap(),
            CompositionOptions::default()
        );
        assert_eq!(
            parse_meta(r#"<meta name="fosscut:composition" content="width=wide">"#)
                .unwrap_err()
                .code(),
            "fosscut.web.invalid_composition"
        );
    }

    #[test]
    fn missing_and_invalid_values_are_named() {
        let unknown = CompositionOptions {
            width: Some(100),
            ..CompositionOptions::default()
        }
        .resolve()
        .unwrap_err();
        assert_eq!(
            unknown,
            WebError::CompositionUnknown {
                missing: vec!["height".into(), "duration".into()]
            }
        );
        let base = CompositionOptions {
            width: Some(100),
            height: Some(100),
            fps: None,
            duration: Some("2s".parse().unwrap()),
        };
        for broken in [
            CompositionOptions {
                width: Some(0),
                ..base.clone()
            },
            CompositionOptions {
                height: Some(MAX_SIDE + 1),
                ..base.clone()
            },
            CompositionOptions {
                fps: Some("29.97".into()),
                ..base.clone()
            },
            CompositionOptions {
                fps: Some("500".into()),
                ..base.clone()
            },
            CompositionOptions {
                duration: Some("0s".parse().unwrap()),
                ..base.clone()
            },
            CompositionOptions {
                duration: Some("3601s".parse().unwrap()),
                ..base.clone()
            },
        ] {
            assert_eq!(
                broken.resolve().unwrap_err().code(),
                "fosscut.web.invalid_composition",
                "{broken:?}"
            );
        }
        let given = CompositionOptions {
            width: Some(50),
            ..CompositionOptions::default()
        }
        .or(base.clone())
        .resolve()
        .unwrap();
        assert_eq!(
            (given.width, given.height),
            (50, 100),
            "arguments win over the tag"
        );
    }

    #[test]
    fn network_loads_are_found_and_links_are_not() {
        let html = r#"
            <svg xmlns="http://www.w3.org/2000/svg"></svg>
            <a href="https://example.com/about">about</a>
            <script src="https://cdn.example.com/gsap.min.js"></script>
            <link rel="stylesheet" href='https://fonts.example.com/css?family=Inter'>
            <style>@import "https://x.example.com/a.css"; body { background: url(https://x.example.com/bg.png) }</style>
            <script type="module">import { a } from "https://esm.example.com/a.js";</script>
            <img src="local.png">
        "#;
        assert_eq!(
            remote_references(html),
            vec![
                "https://cdn.example.com/gsap.min.js",
                "https://fonts.example.com/css?family=Inter",
                "https://x.example.com/a.css",
                "https://x.example.com/bg.png",
                "https://esm.example.com/a.js",
            ]
        );
        assert!(remote_references("<img src='a.png'>").is_empty());
    }

    #[test]
    fn folders_and_files_are_copied_once_by_content() {
        let dir = std::env::temp_dir().join(format!("fosscut-web-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let source = dir.join("promo");
        std::fs::create_dir_all(source.join("lib")).unwrap();
        std::fs::create_dir_all(source.join("node_modules/x")).unwrap();
        std::fs::write(source.join("node_modules/x/big.js"), "skipped").unwrap();
        std::fs::write(source.join(".DS_Store"), "skipped").unwrap();
        std::fs::write(
            source.join("index.html"),
            r#"<meta name="fosscut:composition" content="width=320,height=240,duration=2"><script src="lib/app.js"></script>"#,
        )
        .unwrap();
        std::fs::write(source.join("lib/app.js"), "console.log(1)").unwrap();
        let bundle = dir.join("Show.fosscut");
        std::fs::create_dir_all(&bundle).unwrap();

        let args = media_add_args(&source, None, &bundle, &CompositionOptions::default()).unwrap();
        assert_eq!(args["kind"], "web");
        assert_eq!(args["name"], "promo");
        assert_eq!(args["info"]["video"]["width"], 320);
        assert_eq!(args["info"]["video"]["codec"], "html");
        let relative = args["path"]["relative"].as_str().unwrap().to_string();
        assert!(
            relative.starts_with("web/") && relative.ends_with("/index.html"),
            "{relative}"
        );
        let copied = bundle.join(&relative);
        assert!(copied.is_file());
        let folder = copied.parent().unwrap();
        assert!(folder.join("lib/app.js").is_file());
        assert!(!folder.join("node_modules").exists() && !folder.join(".DS_Store").exists());

        // The same content again reuses the folder; changed content gets another one.
        let again = media_add_args(&source, None, &bundle, &CompositionOptions::default()).unwrap();
        assert_eq!(again["identity"], args["identity"]);
        std::fs::write(source.join("lib/app.js"), "console.log(2)").unwrap();
        let changed = media_add_args(
            &source,
            Some("Promo v2"),
            &bundle,
            &CompositionOptions::default(),
        )
        .unwrap();
        assert_ne!(changed["identity"], args["identity"]);
        assert_eq!(changed["name"], "Promo v2");
        assert_eq!(std::fs::read_dir(bundle.join(WEB_DIR)).unwrap().count(), 2);

        // One file, sized by arguments.
        let page = dir.join("card.html");
        std::fs::write(&page, "<p>card</p>").unwrap();
        let options = CompositionOptions {
            width: Some(100),
            height: Some(50),
            fps: Some("25".into()),
            duration: Some("1s".parse().unwrap()),
        };
        let single = media_add_args(&page, None, &bundle, &options).unwrap();
        assert!(
            single["path"]["relative"]
                .as_str()
                .unwrap()
                .ends_with("/card.html")
        );
        assert_eq!(single["info"]["video"]["rate"], "25");

        let not = media_add_args(&dir.join("missing"), None, &bundle, &options).unwrap_err();
        assert_eq!(not.code, "fosscut.web.not_composition");
        std::fs::write(
            dir.join("remote.html"),
            r#"<script src="https://cdn.example.com/x.js"></script>"#,
        )
        .unwrap();
        let remote = media_add_args(&dir.join("remote.html"), None, &bundle, &options).unwrap_err();
        assert_eq!(remote.code, "fosscut.web.remote_reference");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
