//! What the media bin and the timeline show of an asset without changing the project:
//! strips of thumbnails of a video or image, and the waveform of its sound; and importing
//! files as the user's edits.
//!
//! Pictures render through the same frame evaluation as exports (ADR-0008): a scratch copy
//! of the asset alone on a timeline of its own, in memory, never written. Sound is mixed by
//! the export's mixer from the same kind of scratch timeline. Both are cached by the
//! asset's content identity from the media table (not its path, so a relinked identical
//! file hits), in memory and on disk in the engine's cache directory, never in the bundle:
//! thumbnails in `frames/media-thumbnails/` and waveforms in `analysis/waveforms/`, so the
//! caches' sizes, clearing and age and size limits count them with the rendered frames and
//! the analyses (ADR-0032).

use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use fosscut_core::{ApiError, CommandCall, Request, codes};
use fosscut_engine::preview::Sound;
use fosscut_model::{
    Asset, AssetKind, Clip, ClipContent, ContentIdentity, Id, IdKind, Project, TimelineFormat,
    Track, TrackKind, VideoInfo,
};
use fosscut_render::{Image, Renderer};
use fosscut_time::{FLICKS_PER_SECOND, Flicks, Rate};
use serde::Serialize;
use serde_json::{Value, json};

use crate::commands::{open_project, parse_id};
use crate::words::Word;
use crate::workspace::Workspace;

/// Thumbnail height when the window asks for none, in pixels.
pub const DEFAULT_HEIGHT: u32 = 72;
/// Lowest thumbnail height, in pixels.
pub const MIN_HEIGHT: u32 = 16;
/// Highest thumbnail height, in pixels.
pub const MAX_HEIGHT: u32 = 360;
/// Most thumbnails in one strip.
pub const MAX_FRAMES: usize = 48;
/// Most waveform buckets, which is also how finely a waveform is measured and kept.
pub const MAX_BUCKETS: u32 = 65_536;

/// Version of the thumbnail files; bump it when how they are made changes.
const THUMBNAIL_VERSION: u32 = 1;
/// Version of the waveform files; bump it when how they are made changes.
const WAVEFORM_VERSION: u32 = 1;
/// The first bytes of a waveform file.
const WAVEFORM_MAGIC: &[u8; 4] = b"FCWF";
/// Bytes before a waveform file's peaks: the magic, the version and the count.
const WAVEFORM_HEADER: usize = 12;
/// The first bytes of every PNG file.
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
/// Thumbnails kept in memory.
const THUMBNAILS_KEPT: usize = 512;
/// Waveforms (at the finest resolution) kept in memory.
const WAVEFORMS_KEPT: usize = 16;
/// Seconds of sound mixed at a time while measuring a waveform.
const BLOCK_SECONDS: i64 = 5;
/// The scratch timeline's frame rate: thumbnail `i` is its frame `i`.
const SCRATCH_RATE: Rate = Rate::FPS_30;

/// One thumbnail of a strip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Thumbnail {
    /// The source time in flicks, as asked for or spread over the duration (0 for an image
    /// spread, which has none): the picture shows the frame containing it.
    pub flicks: i64,
    /// The picture as a PNG data URL.
    pub image: String,
}

/// Thumbnails of one asset, all of one size (`media_thumbnails`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ThumbnailStrip {
    /// The asset.
    pub asset: String,
    /// Width of every thumbnail in pixels, following the asset's aspect ratio.
    pub width: u32,
    /// Height of every thumbnail in pixels.
    pub height: u32,
    /// The thumbnails, in the order of the times asked for.
    pub frames: Vec<Thumbnail>,
}

/// The peaks of an asset's sound (`media_waveform`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Waveform {
    /// The asset.
    pub asset: String,
    /// The asset's duration in flicks.
    pub duration: i64,
    /// How many equal spans the duration is split into.
    pub buckets: u32,
    /// The lowest and highest sample of each span, from -1 to 1.
    pub peaks: Vec<[f32; 2]>,
}

/// A file that could not be imported, and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImportFailure {
    /// The file.
    pub path: String,
    /// Why, for people.
    pub error: String,
    /// The error's code, such as `fosscut.media.probe_failed`.
    pub code: String,
}

/// What `media_import` added.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImportResult {
    /// The assets added, in the order of the files.
    pub assets: Vec<String>,
    /// The files skipped.
    pub failed: Vec<ImportFailure>,
    /// The project's revision after the import.
    pub revision: u64,
}

/// The file kinds the import dialog offers, each named by a translated word; the first
/// holds every other, and the last lets any file be chosen. macOS offers only the union of
/// the extensions.
pub const IMPORT_FILTERS: [(Word, &[&str]); 10] = [
    (Word::MediaFiles, &MEDIA_EXTENSIONS),
    (Word::VideoFiles, &VIDEO_EXTENSIONS),
    (Word::AudioFiles, &AUDIO_EXTENSIONS),
    (Word::ImageFiles, &IMAGE_EXTENSIONS),
    (Word::DrawingFiles, &["svg"]),
    (Word::WebFiles, &WEB_EXTENSIONS),
    (Word::ModelFiles, &["glb", "gltf"]),
    (Word::LutFiles, &["cube"]),
    (Word::FontFiles, &FONT_EXTENSIONS),
    (Word::AllFiles, &["*"]),
];

const VIDEO_EXTENSIONS: [&str; 16] = [
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mts", "m2ts", "ts", "mxf", "mpg", "mpeg", "wmv",
    "flv", "3gp", "ogv",
];
const AUDIO_EXTENSIONS: [&str; 12] = [
    "mp3", "wav", "m4a", "aac", "flac", "ogg", "oga", "opus", "aif", "aiff", "caf", "wma",
];
const IMAGE_EXTENSIONS: [&str; 10] = [
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "heic", "heif",
];
const FONT_EXTENSIONS: [&str; 4] = ["ttf", "otf", "ttc", "otc"];
/// Pages of web compositions (ADR-0039): one that names its size and length in its
/// `fosscut:composition` meta tag imports from the dialog; a folder, or a page without
/// the tag, through `web_import`.
const WEB_EXTENSIONS: [&str; 2] = ["html", "htm"];
const MEDIA_EXTENSIONS: [&str; 48] = [
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mts", "m2ts", "ts", "mxf", "mpg", "mpeg", "wmv",
    "flv", "3gp", "ogv", "mp3", "wav", "m4a", "aac", "flac", "ogg", "oga", "opus", "aif", "aiff",
    "caf", "wma", "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "heic", "heif", "svg",
    "html", "htm", "glb", "gltf", "cube", "ttf", "otf", "ttc", "otc",
];

/// A small map that forgets the least recently used entry beyond `capacity`.
struct Lru<K, V> {
    capacity: usize,
    tick: u64,
    entries: HashMap<K, (V, u64)>,
}

impl<K: Eq + Hash + Clone, V: Clone> Lru<K, V> {
    fn new(capacity: usize) -> Lru<K, V> {
        Lru {
            capacity: capacity.max(1),
            tick: 0,
            entries: HashMap::new(),
        }
    }

    fn get(&mut self, key: &K) -> Option<V> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(key).map(|(value, used)| {
            *used = tick;
            value.clone()
        })
    }

    fn put(&mut self, key: K, value: V) {
        self.tick += 1;
        self.entries.insert(key, (value, self.tick));
        while self.entries.len() > self.capacity {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(key) => {
                    self.entries.remove(&key);
                }
                None => break,
            }
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

/// What thumbnails of an asset show at the times asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pictured {
    /// The frame at each time: a video.
    Moving,
    /// The same picture at every time: an image or a vector drawing.
    Still,
    /// The first frame at every time: a web composition, whose frames each need a browser.
    First,
}

/// One thumbnail in the caches: the content, the height and the source time shown (none
/// for a still image, which looks the same at every time).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PictureKey {
    content: String,
    height: u32,
    at: Option<i64>,
}

/// Thumbnails and waveforms of the open project's media, cached, with the headless
/// renderer thumbnails are drawn with, made on first use.
pub struct MediaViews {
    cache: PathBuf,
    renderer: Mutex<Option<Renderer>>,
    pictures: Mutex<Lru<PictureKey, Arc<Vec<u8>>>>,
    sounds: Mutex<Lru<String, Arc<Vec<[f32; 2]>>>>,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn internal(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INTERNAL, message)
}

fn kind_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Video => "video",
        AssetKind::Audio => "audio",
        AssetKind::Image => "image",
        AssetKind::Lut => "lut",
        AssetKind::Font => "font",
        AssetKind::Vector => "vector",
        AssetKind::Web => "web",
        AssetKind::Model => "model",
    }
}

/// The asset `id` of `project`.
fn find_asset<'a>(project: &'a Project, id: &Id) -> Result<&'a Asset, ApiError> {
    project.media.get(id).ok_or_else(|| {
        ApiError::new(codes::ASSET_NOT_FOUND, format!("asset {id} does not exist"))
            .with_details(json!({ "asset": id }))
    })
}

fn no_picture(asset: &Asset) -> ApiError {
    let kind = kind_name(asset.kind);
    ApiError::new(
        "fosscut.media.no_picture",
        format!("{} is a {kind} asset, which has no picture", asset.id),
    )
    .with_hint("Thumbnails show video, image, vector drawing and web composition assets.")
    .with_details(json!({ "asset": asset.id, "kind": kind }))
}

fn no_sound(asset: &Asset) -> ApiError {
    let kind = kind_name(asset.kind);
    let message = if asset.kind == AssetKind::Video {
        format!("{} is a video asset without sound", asset.id)
    } else {
        format!("{} is a {kind} asset, which has no sound", asset.id)
    };
    ApiError::new("fosscut.media.no_sound", message)
        .with_hint("Waveforms show audio assets and video assets with sound.")
        .with_details(json!({ "asset": asset.id, "kind": kind }))
}

fn incomplete(asset: &Asset, what: &str) -> ApiError {
    ApiError::new(
        codes::MEDIA_INCOMPLETE,
        format!("the media table has no {what} for {}", asset.id),
    )
    .with_details(json!({ "asset": asset.id }))
}

/// The folder-safe name of a file content, as the analysis cache names it: the identity's
/// hex digits (an edited media table cannot point outside the cache with them) and size.
fn content_name(identity: &ContentIdentity) -> String {
    let hash: String = identity
        .sample_blake3
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(64)
        .collect();
    format!("{hash}-{}", identity.size)
}

/// The size a picture shows at: rotated a quarter turn, its width and height swap.
fn display_size(video: &VideoInfo) -> (u32, u32) {
    if video.rotation.rem_euclid(180) == 90 {
        (video.height, video.width)
    } else {
        (video.width, video.height)
    }
}

/// A thumbnail `height` pixels high with the aspect ratio of `shown`, rounded to the
/// nearest pixel and at most [`fosscut_engine::frame::MAX_WIDTH`] wide; `None` without a
/// size.
fn thumbnail_size(shown: (u32, u32), height: u32) -> Option<(u32, u32)> {
    let (w, h) = (u64::from(shown.0), u64::from(shown.1));
    if w == 0 || h == 0 {
        return None;
    }
    let width = (2 * u64::from(height) * w + h) / (2 * h);
    let width = width.clamp(1, u64::from(fosscut_engine::frame::MAX_WIDTH));
    Some((u32::try_from(width).ok()?, height))
}

/// `count` times spread evenly over `duration`: the middle of each of `count` equal parts.
fn spread(duration: Option<Flicks>, count: u32) -> Vec<i64> {
    let duration = i128::from(duration.map_or(0, Flicks::get).max(0));
    let count = i128::from(count.max(1));
    (0..count)
        .map(|i| {
            // Below the duration, which fits in i64.
            i64::try_from(duration * (2 * i + 1) / (2 * count)).unwrap_or(i64::MAX)
        })
        .collect()
}

/// The source time a thumbnail of `time` shows: within the asset, at the start of the frame
/// containing it at the asset's frame rate, so nearby times share a picture.
fn shown_time(time: i64, duration: Option<Flicks>, rate: Option<Rate>) -> i64 {
    let last = duration.map_or(i64::MAX, |d| (d.get() - 1).max(0));
    let time = time.clamp(0, last);
    match rate {
        Some(rate) => rate
            .frames_to_flicks(rate.frame_at(Flicks::new(time)))
            .get()
            .clamp(0, last),
        None => time,
    }
}

/// Where the thumbnail of `key` is kept below `cache`.
fn thumbnail_path(cache: &Path, key: &PictureKey) -> PathBuf {
    let at = key
        .at
        .map_or_else(|| "still".to_string(), |at| at.to_string());
    cache
        .join("frames")
        .join("media-thumbnails")
        .join(&key.content)
        .join(format!("v{THUMBNAIL_VERSION}-{}-{at}.png", key.height))
}

/// Where the waveform of `content` is kept below `cache`.
fn waveform_path(cache: &Path, content: &str) -> PathBuf {
    cache
        .join("analysis")
        .join("waveforms")
        .join(format!("{content}.v{WAVEFORM_VERSION}.peaks"))
}

/// A sample from -1 to 1 as a 16-bit number.
fn quantize(value: f32) -> i16 {
    // Within i16 after the clamp.
    (value.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

fn dequantize(value: i16) -> f32 {
    f32::from(value) / 32767.0
}

/// The bytes of a waveform file: [`WAVEFORM_MAGIC`], then the version and the number of
/// peaks as little-endian 32-bit numbers, then each peak's lowest and highest sample as
/// little-endian 16-bit numbers (full scale 32767).
fn encode_waveform(peaks: &[[f32; 2]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(WAVEFORM_HEADER + peaks.len() * 4);
    bytes.extend_from_slice(WAVEFORM_MAGIC);
    bytes.extend_from_slice(&WAVEFORM_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(peaks.len() as u32).to_le_bytes());
    for [low, high] in peaks {
        bytes.extend_from_slice(&quantize(*low).to_le_bytes());
        bytes.extend_from_slice(&quantize(*high).to_le_bytes());
    }
    bytes
}

/// The peaks in a waveform file's bytes, if they are a whole file of this version.
fn decode_waveform(bytes: &[u8]) -> Option<Vec<[f32; 2]>> {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    if bytes.get(..4)? != WAVEFORM_MAGIC || word(4)? != WAVEFORM_VERSION {
        return None;
    }
    let count = usize::try_from(word(8)?).ok()?;
    if count > MAX_BUCKETS as usize || bytes.len() != WAVEFORM_HEADER + count * 4 {
        return None;
    }
    let (whole, _) = bytes[WAVEFORM_HEADER..].as_chunks::<4>();
    let peaks = whole
        .iter()
        .map(|[a, b, c, d]| {
            let low = i16::from_le_bytes([*a, *b]);
            let high = i16::from_le_bytes([*c, *d]);
            [dequantize(low), dequantize(high)]
        })
        .collect();
    Some(peaks)
}

/// Writes `bytes` to `path` through a temporary file beside it (named `.partial`, which
/// the cache policy leaves alone), so readers never see half of it. A cache that cannot be
/// written only costs making the entry again.
fn write_entry(path: &Path, bytes: &[u8]) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".{name}.{}-{n}.partial", std::process::id()));
    if std::fs::write(&temporary, bytes).is_err() || std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

/// Reads a cache entry and marks it used now, for the cache policy's age and size limits.
fn read_entry(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    if let Ok(file) = std::fs::File::options().write(true).open(path) {
        let now = std::time::SystemTime::now();
        let _ = file.set_times(
            std::fs::FileTimes::new()
                .set_accessed(now)
                .set_modified(now),
        );
    }
    Some(bytes)
}

fn data_url(png: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

/// A project in memory holding `asset` alone, on one `kind` track of its primary timeline
/// in `format` with `clips`: what thumbnails and waveforms are made from. It is never
/// stored, and the open project is not changed.
fn scratch(
    asset: &Asset,
    format: TimelineFormat,
    kind: TrackKind,
    clips: Vec<Clip>,
) -> Result<(Project, Id), ApiError> {
    let mut project = Project::new(Id::new(IdKind::Project, 0), "Scratch", format);
    project.media.assets = vec![asset.clone()];
    let timeline = project.manifest.primary_timeline.clone();
    // Read from JSON, so fields tracks gain later take their defaults.
    let mut track: Track =
        serde_json::from_value(json!({ "id": Id::new(IdKind::Track, 1), "kind": kind }))
            .map_err(|e| internal(format!("cannot make a scratch track: {e}")))?;
    track.clips = clips;
    let doc = project
        .timelines
        .get_mut(&timeline)
        .ok_or_else(|| internal("a new project has no primary timeline"))?;
    doc.tracks = vec![track];
    Ok((project, timeline))
}

/// The pixels of `area` of `image`, if it lies inside.
fn cut_out(image: &Image, area: &fosscut_engine::frame::Tile) -> Option<Image> {
    let row_bytes = area.width as usize * 4;
    let mut rgba = Vec::with_capacity(row_bytes * area.height as usize);
    for row in 0..area.height as usize {
        let start = ((area.y as usize + row) * image.width as usize + area.x as usize) * 4;
        rgba.extend_from_slice(image.rgba.get(start..start + row_bytes)?);
    }
    Some(Image {
        width: area.width,
        height: area.height,
        rgba,
    })
}

impl MediaViews {
    /// Views keeping their files in the cache directory `cache` (the engine's).
    pub fn new(cache: &Path) -> MediaViews {
        MediaViews {
            cache: cache.to_path_buf(),
            renderer: Mutex::new(None),
            pictures: Mutex::new(Lru::new(THUMBNAILS_KEPT)),
            sounds: Mutex::new(Lru::new(WAVEFORMS_KEPT)),
        }
    }

    /// Forgets the thumbnails and waveforms kept in memory, as when the caches are cleared.
    pub fn forget(&self) {
        guard(&self.pictures).clear();
        guard(&self.sounds).clear();
    }

    /// Thumbnails of video or image asset `asset` of the project open in `workspace`, at
    /// source times `times` in flicks (at most [`MAX_FRAMES`]), or else at `count` times
    /// spread evenly over its duration (the middle of each of `count` equal parts; 1 when
    /// omitted, at most [`MAX_FRAMES`]). They are `height` pixels high ([`DEFAULT_HEIGHT`]
    /// when omitted, kept from [`MIN_HEIGHT`] to [`MAX_HEIGHT`]) and as wide as the asset's
    /// aspect ratio makes them. Times outside the asset show its first or last frame; an
    /// image shows the same picture at every time. Thumbnails already made come from the
    /// caches, even when the file is missing now; the others need the file. An asset
    /// without a content identity in the media table is rendered every time.
    pub fn thumbnails(
        &self,
        workspace: &Workspace,
        asset: &str,
        times: Option<&[i64]>,
        count: Option<u32>,
        height: Option<u32>,
    ) -> Result<ThumbnailStrip, ApiError> {
        let id = parse_id(asset, "asset")?;
        let (bundle, project, _) = open_project(workspace)?;
        let asset = find_asset(&project, &id)?;
        let pictured = match asset.kind {
            AssetKind::Video => Pictured::Moving,
            // A drawing renders sharp at the thumbnail's size, through the same frame
            // evaluation as an image clip of it (ADR-0036).
            AssetKind::Image | AssetKind::Vector => Pictured::Still,
            // Every frame of a composition needs a browser (ADR-0039), so its first frame
            // stands for all of it: rendered once, then cached by its content.
            AssetKind::Web => Pictured::First,
            AssetKind::Audio | AssetKind::Lut | AssetKind::Font | AssetKind::Model => {
                return Err(no_picture(asset));
            }
        };
        let video = asset
            .info
            .video
            .as_ref()
            .ok_or_else(|| incomplete(asset, "picture size"))?;
        let height = height
            .unwrap_or(DEFAULT_HEIGHT)
            .clamp(MIN_HEIGHT, MAX_HEIGHT);
        let size = thumbnail_size(display_size(video), height)
            .ok_or_else(|| incomplete(asset, "picture size"))?;
        let wanted = match times {
            Some(times) if times.len() > MAX_FRAMES => {
                return Err(crate::commands::invalid(format!(
                    "a strip has at most {MAX_FRAMES} thumbnails, not {}",
                    times.len()
                )));
            }
            Some(times) => times.to_vec(),
            None => spread(
                asset.info.duration,
                count.unwrap_or(1).clamp(1, MAX_FRAMES as u32),
            ),
        };
        let shown: Vec<Option<i64>> = wanted
            .iter()
            .map(|&time| match pictured {
                Pictured::Moving => Some(shown_time(time, asset.info.duration, video.rate)),
                Pictured::Still => None,
                Pictured::First => Some(0),
            })
            .collect();
        let content = asset.identity.as_ref().map(content_name);
        let mut made: HashMap<Option<i64>, Arc<Vec<u8>>> = HashMap::new();
        let mut missing = Vec::new();
        for at in &shown {
            if made.contains_key(at) || missing.contains(at) {
                continue;
            }
            let key = content.as_ref().map(|content| PictureKey {
                content: content.clone(),
                height,
                at: *at,
            });
            match key.and_then(|key| self.kept_picture(&key)) {
                Some(png) => {
                    made.insert(*at, png);
                }
                None => missing.push(*at),
            }
        }
        if !missing.is_empty() {
            let rendered = self.render(&project, &bundle, asset, size, &missing)?;
            for (at, png) in missing.iter().zip(rendered) {
                if let Some(content) = &content {
                    self.keep_picture(
                        PictureKey {
                            content: content.clone(),
                            height,
                            at: *at,
                        },
                        &png,
                    );
                }
                made.insert(*at, png);
            }
        }
        let frames = wanted
            .iter()
            .zip(&shown)
            .map(|(&flicks, at)| {
                let png = made
                    .get(at)
                    .ok_or_else(|| internal("a thumbnail was not made"))?;
                Ok(Thumbnail {
                    flicks,
                    image: data_url(png),
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        Ok(ThumbnailStrip {
            asset: id.to_string(),
            width: size.0,
            height: size.1,
            frames,
        })
    }

    /// The thumbnail of `key` from memory, else from disk (then kept in memory too).
    fn kept_picture(&self, key: &PictureKey) -> Option<Arc<Vec<u8>>> {
        if let Some(png) = guard(&self.pictures).get(key) {
            return Some(png);
        }
        let bytes = read_entry(&thumbnail_path(&self.cache, key))?;
        if !bytes.starts_with(PNG_SIGNATURE) {
            return None;
        }
        let png = Arc::new(bytes);
        guard(&self.pictures).put(key.clone(), Arc::clone(&png));
        Some(png)
    }

    fn keep_picture(&self, key: PictureKey, png: &Arc<Vec<u8>>) {
        write_entry(&thumbnail_path(&self.cache, &key), png);
        guard(&self.pictures).put(key, Arc::clone(png));
    }

    /// Renders a thumbnail of `asset` at each source time of `times` (`None` for a still
    /// image) at `size`, as PNG. Each time is one frame of a scratch timeline, all rendered
    /// as one contact sheet so the asset's decoder stays open between them.
    fn render(
        &self,
        project: &Project,
        bundle: &Path,
        asset: &Asset,
        size: (u32, u32),
        times: &[Option<i64>],
    ) -> Result<Vec<Arc<Vec<u8>>>, ApiError> {
        // A missing file fails here, with the paths tried.
        fosscut_engine::import::asset_file(project, bundle, &asset.id)?;
        let frame = SCRATCH_RATE.frame_duration();
        let clips = times
            .iter()
            .enumerate()
            .map(|(index, at)| {
                let content = match at {
                    Some(at) => ClipContent::Video {
                        asset: asset.id.clone(),
                        source_in: Flicks::new(*at),
                    },
                    None => ClipContent::Image {
                        asset: asset.id.clone(),
                    },
                };
                Clip::new(
                    Id::new(IdKind::Clip, index as u64 + 1),
                    Flicks::new(frame.get() * index as i64),
                    frame,
                    content,
                )
            })
            .collect();
        let format = TimelineFormat {
            width: size.0,
            height: size.1,
            rate: SCRATCH_RATE,
            sample_rate: 48_000,
        };
        let (scratch, timeline) = scratch(asset, format, TrackKind::Video, clips)?;
        let count = u32::try_from(times.len()).map_err(|_| internal("too many thumbnails"))?;
        let mut renderer = guard(&self.renderer);
        if renderer.is_none() {
            let made = Renderer::headless().map_err(|e| ApiError::new(e.code(), e.to_string()))?;
            *renderer = Some(made);
        }
        let renderer = renderer
            .as_mut()
            .ok_or_else(|| internal("the renderer is missing"))?;
        let sheet = fosscut_engine::frame::contact_sheet_in(
            &scratch,
            bundle,
            &timeline,
            count,
            size.0,
            &self.cache,
            renderer,
        )
        .map_err(ApiError::from)?;
        sheet
            .tiles
            .iter()
            .map(|tile| {
                let image = cut_out(&sheet.image, tile)
                    .ok_or_else(|| internal("a thumbnail lies outside its sheet"))?;
                fosscut_engine::frame::png(&image)
                    .map(Arc::new)
                    .map_err(ApiError::from)
            })
            .collect()
    }

    /// The waveform of audio asset `asset`, or of video asset `asset`'s sound, of the
    /// project open in `workspace`: its duration split into `buckets` equal spans (kept
    /// from 1 to [`MAX_BUCKETS`]), each the lowest and highest sample of any of its
    /// channels in that span. Channels are not mixed down, so a sound on one side shows
    /// at its full height. The sound is measured once per content, [`MAX_BUCKETS`] spans
    /// fine (or one per sample when it has fewer), and coarser waveforms are merged from
    /// that, so asking for another bucket count costs no decoding; a merged bucket's edges
    /// may reach into its neighbours by at most one fine span. Waveforms already measured
    /// come from the caches, even when the file is missing now. An asset without a content
    /// identity in the media table is measured every time.
    pub fn waveform(
        &self,
        workspace: &Workspace,
        asset: &str,
        buckets: u32,
    ) -> Result<Waveform, ApiError> {
        let id = parse_id(asset, "asset")?;
        let (bundle, project, _) = open_project(workspace)?;
        let asset = find_asset(&project, &id)?;
        let sound = match asset.kind {
            AssetKind::Audio => true,
            AssetKind::Video => asset.info.audio.is_some(),
            // Drawings, compositions and 3D models have no sound (ADR-0036, ADR-0039).
            AssetKind::Image
            | AssetKind::Lut
            | AssetKind::Font
            | AssetKind::Vector
            | AssetKind::Web
            | AssetKind::Model => false,
        };
        if !sound {
            return Err(no_sound(asset));
        }
        let duration = asset
            .info
            .duration
            .filter(|d| d.is_positive())
            .ok_or_else(|| incomplete(asset, "duration"))?;
        let buckets = buckets.clamp(1, MAX_BUCKETS);
        let content = asset.identity.as_ref().map(content_name);
        let kept = content.as_deref().and_then(|c| self.kept_sound(c));
        let fine = match kept {
            Some(fine) => fine,
            None => {
                // Kept as stored, so the first answer equals every later one.
                let measured = measure(&project, &bundle, asset, duration)?;
                let bytes = encode_waveform(&measured);
                let fine = Arc::new(
                    decode_waveform(&bytes)
                        .ok_or_else(|| internal("a waveform does not read back"))?,
                );
                if let Some(content) = &content {
                    write_entry(&waveform_path(&self.cache, content), &bytes);
                    guard(&self.sounds).put(content.clone(), Arc::clone(&fine));
                }
                fine
            }
        };
        let peaks = merge_peaks(&fine, buckets as usize)
            .into_iter()
            .map(|[low, high]| [rounded(low), rounded(high)])
            .collect();
        Ok(Waveform {
            asset: id.to_string(),
            duration: duration.get(),
            buckets,
            peaks,
        })
    }

    /// The finest waveform of `content` from memory, else from disk (then kept in memory).
    fn kept_sound(&self, content: &str) -> Option<Arc<Vec<[f32; 2]>>> {
        let key = content.to_string();
        if let Some(fine) = guard(&self.sounds).get(&key) {
            return Some(fine);
        }
        let fine = Arc::new(decode_waveform(&read_entry(&waveform_path(
            &self.cache,
            content,
        ))?)?);
        guard(&self.sounds).put(key, Arc::clone(&fine));
        Some(fine)
    }
}

/// A peak value to four decimals, which is finer than any waveform is drawn.
fn rounded(value: f32) -> f32 {
    (value * 10_000.0).round() / 10_000.0
}

/// Measures the finest waveform of `asset` (`duration` long), mixing its sound alone, at
/// its own sample rate and with its own channels (two at most), by the export's mixer.
fn measure(
    project: &Project,
    bundle: &Path,
    asset: &Asset,
    duration: Flicks,
) -> Result<Vec<[f32; 2]>, ApiError> {
    // A missing file fails here, with the paths tried.
    fosscut_engine::import::asset_file(project, bundle, &asset.id)?;
    let audio = asset.info.audio.as_ref();
    let rate = audio
        .map(|a| a.sample_rate)
        .filter(|rate| (8_000..=384_000).contains(rate))
        .unwrap_or(48_000);
    // Mono stays mono, so its samples are read as they are, not spread over two channels.
    let channels: u16 = if audio.is_some_and(|a| a.channels == 1) {
        1
    } else {
        2
    };
    let total = u64::try_from(fosscut_audio::samples_in(duration, rate).max(0)).unwrap_or(0);
    let mut bins = Bins::new(total, MAX_BUCKETS as usize);
    let clip = Clip::new(
        Id::new(IdKind::Clip, 1),
        Flicks::ZERO,
        duration,
        ClipContent::Audio {
            asset: asset.id.clone(),
            source_in: Flicks::ZERO,
        },
    );
    let format = TimelineFormat {
        width: 16,
        height: 16,
        rate: SCRATCH_RATE,
        sample_rate: rate,
    };
    let (scratch, timeline) = scratch(asset, format, TrackKind::Audio, vec![clip])?;
    let mut sound = Sound::new();
    let block = Flicks::new(BLOCK_SECONDS * FLICKS_PER_SECOND);
    let mut at = Flicks::ZERO;
    while at < duration {
        let length = block.min(duration - at);
        let mixed = sound
            .mix(&scratch, bundle, &timeline, at, length, rate, channels)
            .map_err(ApiError::from)?;
        bins.push(&mixed.samples, usize::from(channels));
        at = at + length;
    }
    Ok(bins.finish())
}

/// Where span `index` of `count` equal spans of `total` items begins: spans split the
/// items as evenly as whole items allow.
fn boundary(index: usize, total: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    // At most `total`, as `index` is at most `count`.
    (index as u128 * total as u128 / count as u128) as usize
}

/// The peaks of equal spans of a sound fed in blocks: the lowest and highest sample of
/// any channel in each span. There are never more spans than sample frames, so none is
/// empty.
struct Bins {
    total: usize,
    count: usize,
    index: usize,
    end: usize,
    seen: usize,
    low: f32,
    high: f32,
    peaks: Vec<[f32; 2]>,
}

impl Bins {
    /// Spans of `total` sample frames: `count` of them, or one per frame when there are
    /// fewer frames.
    fn new(total: u64, count: usize) -> Bins {
        let total = usize::try_from(total).unwrap_or(usize::MAX);
        let count = count.min(total);
        Bins {
            total,
            count,
            index: 0,
            end: boundary(1, total, count),
            seen: 0,
            low: f32::INFINITY,
            high: f32::NEG_INFINITY,
            peaks: Vec::with_capacity(count),
        }
    }

    /// Takes the next interleaved `samples` with `channels` channels; frames beyond the
    /// total are ignored.
    fn push(&mut self, samples: &[f32], channels: usize) {
        for frame in samples.chunks_exact(channels.max(1)) {
            if self.index >= self.count {
                return;
            }
            for &sample in frame {
                // `min` and `max` skip a NaN.
                self.low = self.low.min(sample);
                self.high = self.high.max(sample);
            }
            self.seen += 1;
            if self.seen >= self.end {
                self.close();
            }
        }
    }

    fn close(&mut self) {
        let peak = if self.low <= self.high {
            [self.low.clamp(-1.0, 1.0), self.high.clamp(-1.0, 1.0)]
        } else {
            [0.0, 0.0]
        };
        self.peaks.push(peak);
        self.index += 1;
        self.end = boundary(self.index + 1, self.total, self.count);
        self.low = f32::INFINITY;
        self.high = f32::NEG_INFINITY;
    }

    /// The peaks; spans the samples never reached are silent.
    fn finish(mut self) -> Vec<[f32; 2]> {
        while self.index < self.count {
            self.close();
        }
        self.peaks
    }
}

/// The lowest and highest sample of any channel in each of `buckets` equal spans of
/// interleaved `samples` with `channels` channels, clamped to -1 to 1. Spans split the
/// sample frames as evenly as whole frames allow; with fewer frames than buckets, each
/// bucket shows the frame its span begins in. Without samples every bucket is silent.
pub fn peaks(samples: &[f32], channels: usize, buckets: usize) -> Vec<[f32; 2]> {
    let frames = samples.len() / channels.max(1);
    let mut bins = Bins::new(frames as u64, buckets);
    bins.push(samples, channels);
    merge_peaks(&bins.finish(), buckets)
}

/// `buckets` peaks from finer `peaks` of the same sound: each bucket the lowest and
/// highest of the fine peaks its span covers, split as [`peaks`] splits samples. More
/// buckets than fine peaks repeat them; no fine peaks give silence.
pub fn merge_peaks(fine: &[[f32; 2]], buckets: usize) -> Vec<[f32; 2]> {
    let n = fine.len();
    (0..buckets)
        .map(|i| {
            if n == 0 {
                return [0.0, 0.0];
            }
            let start = boundary(i, n, buckets);
            let end = boundary(i + 1, n, buckets).max(start + 1).min(n);
            fine[start..end]
                .iter()
                .fold([f32::INFINITY, f32::NEG_INFINITY], |[low, high], [a, b]| {
                    [low.min(*a), high.max(*b)]
                })
        })
        .collect()
}

/// Imports `files` into the project open in `workspace` as the user's one undoable edit
/// ("Import media"). Files that cannot be probed, or that `media.add` refuses, are
/// reported in `failed` and skipped; the rest are added together. No revision is
/// expected: adding media never conflicts with what the window shows, and the dialog may
/// stay open while agents edit.
pub fn import(workspace: &Workspace, files: &[PathBuf]) -> Result<ImportResult, ApiError> {
    let (bundle, project, _) = open_project(workspace)?;
    let mut failed = Vec::new();
    let mut ready: Vec<(&PathBuf, Value)> = Vec::new();
    for file in files {
        match fosscut_engine::import::media_add_args(file, None, &bundle) {
            Ok(args) => ready.push((file, args)),
            Err(error) => failed.push(failure(file, &error)),
        }
    }
    let mut revision = project.manifest.revision;
    let mut assets = Vec::new();
    // Each refused file is taken out and the rest tried again, so at most once per file.
    while !ready.is_empty() {
        let mut request = Request::single("media.add", Value::Null);
        request.commands = ready
            .iter()
            .map(|(_, args)| CommandCall {
                command: "media.add".into(),
                args: args.clone(),
            })
            .collect();
        request.reason = Some("Import media".into());
        match workspace.execute(request) {
            Ok(response) => {
                assets = response
                    .results
                    .iter()
                    .map(|result| {
                        result["asset"]
                            .as_str()
                            .map(str::to_string)
                            .ok_or_else(|| internal("media.add returned no asset"))
                    })
                    .collect::<Result<_, _>>()?;
                revision = response.revision;
                break;
            }
            Err(error) => {
                let index = error.details["command_index"]
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .filter(|i| *i < ready.len());
                let Some(index) = index else {
                    return Err(error);
                };
                let (file, _) = ready.remove(index);
                failed.push(failure(file, &error));
            }
        }
    }
    Ok(ImportResult {
        assets,
        failed,
        revision,
    })
}

fn failure(file: &Path, error: &ApiError) -> ImportFailure {
    ImportFailure {
        path: file.display().to_string(),
        error: error.message.clone(),
        code: error.code.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::NewProject;

    const SECOND: i64 = FLICKS_PER_SECOND;

    #[test]
    fn peaks_split_samples_evenly_over_the_buckets() {
        // Seven samples in three buckets: 2, 2 and 3 samples.
        let samples = [0.1, -0.2, 0.3, 0.4, -0.5, 0.6, 0.7];
        assert_eq!(
            peaks(&samples, 1, 3),
            [[-0.2, 0.1], [0.3, 0.4], [-0.5, 0.7]]
        );
        // As many buckets as samples: each sample its own.
        assert_eq!(
            peaks(&samples[..3], 1, 3),
            [[0.1, 0.1], [-0.2, -0.2], [0.3, 0.3]]
        );
        // One bucket holds everything.
        assert_eq!(peaks(&samples, 1, 1), [[-0.5, 0.7]]);
    }

    #[test]
    fn fewer_samples_than_buckets_repeat_and_nothing_is_silence() {
        assert_eq!(
            peaks(&[0.5, -0.5], 1, 4),
            [[0.5, 0.5], [0.5, 0.5], [-0.5, -0.5], [-0.5, -0.5]]
        );
        assert_eq!(peaks(&[0.25], 1, 3), [[0.25, 0.25]; 3]);
        assert_eq!(peaks(&[], 2, 3), [[0.0, 0.0]; 3]);
        assert_eq!(peaks(&[0.0; 4800], 1, 10), [[0.0, 0.0]; 10]);
        assert!(peaks(&[0.5], 1, 0).is_empty());
    }

    #[test]
    fn peaks_cover_every_channel_and_stay_in_full_scale() {
        // Left loud and negative, right quiet: the envelope of both.
        let stereo = [0.1, -0.9, 0.2, 0.05, -0.3, 0.8];
        assert_eq!(peaks(&stereo, 2, 1), [[-0.9, 0.8]]);
        assert_eq!(
            peaks(&stereo, 2, 3),
            [[-0.9, 0.1], [0.05, 0.2], [-0.3, 0.8]]
        );
        // Clipping is clamped; a NaN is skipped.
        assert_eq!(
            peaks(&[1.5, -3.0, f32::NAN, 0.5], 1, 2),
            [[-1.0, 1.0], [0.5, 0.5]]
        );
        // A trailing part of a frame is no frame.
        assert_eq!(peaks(&[0.5, 0.5, 0.9], 2, 1), [[0.5, 0.5]]);
    }

    #[test]
    fn bins_fed_in_blocks_equal_bins_fed_at_once() {
        let samples: Vec<f32> = (0..10_007)
            .map(|i| ((i as f32) * 0.013).sin() * (i % 7) as f32 / 7.0)
            .collect();
        let mut whole = Bins::new(5_003, 97);
        whole.push(&samples, 2);
        let mut blocks = Bins::new(5_003, 97);
        for block in samples.chunks(2 * 333) {
            blocks.push(block, 2);
        }
        let (whole, blocks) = (whole.finish(), blocks.finish());
        assert_eq!(whole.len(), 97);
        assert_eq!(whole, blocks);
        // Samples that never came are silence; more than announced are ignored.
        let mut short = Bins::new(10, 5);
        short.push(&[0.5, 0.5, 0.5], 1);
        assert_eq!(
            short.finish(),
            [[0.5, 0.5], [0.5, 0.5], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]]
        );
        let mut long = Bins::new(2, 2);
        long.push(&[0.1, 0.2, 0.9, 0.9], 1);
        assert_eq!(long.finish(), [[0.1, 0.1], [0.2, 0.2]]);
    }

    #[test]
    fn coarser_peaks_merge_from_finer_ones() {
        let samples: Vec<f32> = (0..64)
            .map(|i| ((i * 37) % 23) as f32 / 23.0 - 0.5)
            .collect();
        let fine = peaks(&samples, 1, 16);
        assert_eq!(merge_peaks(&fine, 4), peaks(&samples, 1, 4));
        assert_eq!(merge_peaks(&fine, 16), fine, "the same count is unchanged");
        assert_eq!(merge_peaks(&fine, 1), peaks(&samples, 1, 1));
        // Uneven: 16 fine peaks in 3 buckets of 5, 5 and 6.
        let three = merge_peaks(&fine, 3);
        let span = |a: usize, b: usize| merge_peaks(&fine[a..b], 1)[0];
        assert_eq!(three, [span(0, 5), span(5, 10), span(10, 16)]);
        assert_eq!(merge_peaks(&[], 2), [[0.0, 0.0]; 2]);
        assert_eq!(merge_peaks(&fine[..2], 4).len(), 4);
    }

    #[test]
    fn waveform_files_round_trip_and_refuse_anything_else() {
        let fine = [[-0.5, 0.25], [0.0, 0.0], [-1.0, 1.0]];
        let bytes = encode_waveform(&fine);
        assert_eq!(bytes.len(), WAVEFORM_HEADER + 3 * 4);
        let back = decode_waveform(&bytes).unwrap();
        for (a, b) in fine.iter().flatten().zip(back.iter().flatten()) {
            assert!((a - b).abs() < 1e-4, "{a} {b}");
        }
        assert_eq!(rounded(back[0][1]), 0.25);
        assert!(
            decode_waveform(&bytes[..bytes.len() - 1]).is_none(),
            "cut short"
        );
        assert!(decode_waveform(b"FCWF").is_none());
        let mut other = bytes.clone();
        other[4] = 9;
        assert!(decode_waveform(&other).is_none(), "another version");
        assert!(decode_waveform(&[0u8; 16]).is_none());
        assert_eq!(quantize(2.0), 32767);
        assert_eq!(quantize(-2.0), -32767);
    }

    #[test]
    fn times_spread_over_the_middle_of_equal_parts() {
        assert_eq!(
            spread(Some(Flicks::new(4 * SECOND)), 4),
            [SECOND / 2, 3 * SECOND / 2, 5 * SECOND / 2, 7 * SECOND / 2]
        );
        assert_eq!(spread(Some(Flicks::new(10)), 1), [5]);
        assert_eq!(spread(None, 3), [0, 0, 0], "an image has no duration");
        assert_eq!(spread(Some(Flicks::new(9)), 0), [4], "at least one");
    }

    #[test]
    fn thumbnails_keep_the_aspect_of_the_picture_as_shown() {
        let video = |width, height, rotation| VideoInfo {
            width,
            height,
            rate: None,
            codec: None,
            rotation,
        };
        assert_eq!(display_size(&video(1920, 1080, 0)), (1920, 1080));
        assert_eq!(display_size(&video(1920, 1080, -90)), (1080, 1920));
        assert_eq!(display_size(&video(1920, 1080, 270)), (1080, 1920));
        assert_eq!(display_size(&video(1920, 1080, 180)), (1920, 1080));
        assert_eq!(thumbnail_size((1920, 1080), 72), Some((128, 72)));
        assert_eq!(
            thumbnail_size((1080, 1920), 72),
            Some((41, 72)),
            "40.5 rounds up"
        );
        assert_eq!(thumbnail_size((4, 3), 100), Some((133, 100)));
        assert_eq!(thumbnail_size((100_000, 10), 360), Some((4096, 360)));
        assert_eq!(thumbnail_size((1, 100_000), 16), Some((1, 16)));
        assert_eq!(thumbnail_size((0, 1080), 72), None);
    }

    #[test]
    fn thumbnails_show_the_frame_containing_a_time_within_the_asset() {
        let rate = Some(Rate::FPS_30);
        let frame = Rate::FPS_30.frame_duration().get();
        let two = Some(Flicks::new(2 * SECOND));
        assert_eq!(shown_time(frame + 5, two, rate), frame);
        assert_eq!(shown_time(frame + 5, two, None), frame + 5);
        assert_eq!(shown_time(-SECOND, two, rate), 0);
        assert_eq!(shown_time(9 * SECOND, two, rate), 2 * SECOND - frame);
        assert_eq!(shown_time(9 * SECOND, two, None), 2 * SECOND - 1);
        assert_eq!(shown_time(9 * SECOND, None, None), 9 * SECOND);
    }

    #[test]
    fn caches_are_keyed_by_content_height_and_time() {
        let identity = ContentIdentity {
            size: 1234,
            sample_blake3: format!("{}/../{}", "ab".repeat(20), "cd".repeat(20)),
        };
        let content = content_name(&identity);
        assert_eq!(
            content,
            format!("{}-1234", "ab".repeat(20) + &"cd".repeat(12))
        );
        let cache = Path::new("/cache");
        let key = |height, at| PictureKey {
            content: content.clone(),
            height,
            at,
        };
        let moving = thumbnail_path(cache, &key(72, Some(SECOND)));
        assert_eq!(
            moving,
            cache
                .join("frames/media-thumbnails")
                .join(&content)
                .join(format!("v1-72-{SECOND}.png"))
        );
        assert_ne!(moving, thumbnail_path(cache, &key(36, Some(SECOND))));
        assert_ne!(moving, thumbnail_path(cache, &key(72, Some(0))));
        assert!(
            thumbnail_path(cache, &key(72, None))
                .to_string_lossy()
                .ends_with("v1-72-still.png")
        );
        assert_eq!(
            waveform_path(cache, &content),
            cache
                .join("analysis/waveforms")
                .join(format!("{content}.v1.peaks"))
        );
    }

    #[test]
    fn the_memory_cache_forgets_the_least_recently_used() {
        let mut kept = Lru::new(2);
        kept.put("a", 1);
        kept.put("b", 2);
        assert_eq!(kept.get(&"a"), Some(1), "a is used again");
        kept.put("c", 3);
        assert_eq!(kept.get(&"b"), None, "b went first");
        assert_eq!((kept.get(&"a"), kept.get(&"c")), (Some(1), Some(3)));
        kept.put("a", 4);
        assert_eq!(kept.get(&"a"), Some(4), "replaced");
        kept.clear();
        assert_eq!(kept.get(&"c"), None);
    }

    #[test]
    fn the_import_dialog_offers_every_kind_of_media() {
        let last = IMPORT_FILTERS.len() - 1;
        let (first, all) = (IMPORT_FILTERS[0].1, IMPORT_FILTERS[last].1);
        for (_, extensions) in &IMPORT_FILTERS[1..last] {
            for extension in *extensions {
                assert!(first.contains(extension), "{extension} is media");
            }
        }
        let others: usize = IMPORT_FILTERS[1..last].iter().map(|(_, e)| e.len()).sum();
        assert_eq!(first.len(), others, "media is exactly the other kinds");
        assert_eq!(all, ["*"]);
        // Drawings, web compositions and 3D models are offered too (ADR-0036, ADR-0039,
        // ADR-0040).
        for extension in ["svg", "html", "htm", "glb", "gltf"] {
            assert!(first.contains(&extension), "{extension} is offered");
        }
    }

    // ---- With generated media ----

    /// True when the FFmpeg programs are here; in CI (`FOSSCUT_REQUIRE_FFMPEG`) they must be.
    fn ffmpeg_programs() -> bool {
        let runs = |program: &str| {
            std::process::Command::new(program)
                .arg("-version")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        let found = runs("ffmpeg") && runs("ffprobe");
        let required =
            std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
        assert!(
            found || !required,
            "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg or ffprobe is missing"
        );
        found
    }

    /// True when a GPU adapter is here; in CI (`FOSSCUT_REQUIRE_GPU`) one must be.
    fn gpu() -> bool {
        match Renderer::headless() {
            Ok(_) => true,
            Err(error) => {
                let required = std::env::var_os("FOSSCUT_REQUIRE_GPU")
                    .is_some_and(|v| !v.is_empty() && v != "0");
                assert!(
                    !required,
                    "FOSSCUT_REQUIRE_GPU is set but there is no adapter: {error}"
                );
                false
            }
        }
    }

    fn ffmpeg(folder: &Path, args: &[&str]) {
        let status = std::process::Command::new("ffmpeg")
            .current_dir(folder)
            .args(["-y", "-v", "error"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg {args:?}");
    }

    /// A workspace with a new 640 by 360 project open, a short runtime directory (Unix
    /// socket paths are limited to about 100 bytes) and a folder to work in.
    fn opened(name: &str) -> (Workspace, PathBuf, PathBuf) {
        let base = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let runtime = base.join(format!("fcd-{name}-{}", std::process::id()));
        let work =
            std::env::temp_dir().join(format!("fosscut-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).unwrap();
        let workspace = Workspace::new(runtime.clone(), None);
        workspace
            .create(&NewProject {
                path: work.join("Media.fosscut"),
                name: None,
                width: 640,
                height: 360,
                rate: "30".into(),
            })
            .unwrap();
        (workspace, runtime, work)
    }

    fn done(workspace: Workspace, runtime: PathBuf, work: PathBuf) {
        workspace.close();
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
    }

    /// Imports `file` from `work` and returns its asset.
    fn add(workspace: &Workspace, work: &Path, file: &str) -> String {
        let imported = import(workspace, &[work.join(file)]).unwrap();
        assert!(imported.failed.is_empty(), "{imported:?}");
        imported.assets[0].clone()
    }

    fn revision(workspace: &Workspace) -> u64 {
        workspace.query("project.info", Value::Null).unwrap()["revision"]
            .as_u64()
            .unwrap()
    }

    /// The PNG in a data URL, and its width and height from its header.
    fn png_of(url: &str) -> (Vec<u8>, u32, u32) {
        let encoded = url.strip_prefix("data:image/png;base64,").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert!(bytes.starts_with(PNG_SIGNATURE));
        assert_eq!(&bytes[12..16], b"IHDR");
        let number = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        let (width, height) = (number(16), number(20));
        (bytes, width, height)
    }

    /// The RGB pixels of a PNG, decoded by FFmpeg.
    fn pixels(work: &Path, png: &[u8]) -> Vec<u8> {
        let file = work.join("decoded.png");
        std::fs::write(&file, png).unwrap();
        let output = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&file)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .unwrap();
        assert!(output.status.success(), "FFmpeg reads the PNG");
        output.stdout
    }

    fn thumbnail_files(cache: &Path) -> usize {
        fosscut_engine::cache::cache_info(cache)
            .into_iter()
            .find(|usage| usage.kind == fosscut_engine::cache::CacheKind::Frames)
            .map_or(0, |usage| usage.entries as usize)
    }

    #[test]
    fn video_and_image_thumbnails_render_once_and_then_come_from_the_caches() {
        if !ffmpeg_programs() {
            eprintln!("skipped: ffmpeg or ffprobe is missing");
            return;
        }
        if !gpu() {
            eprintln!("skipped: no GPU adapter");
            return;
        }
        let (workspace, runtime, work) = opened("thumbs");
        let cache = work.join("cache");
        // A moving test picture, 2 seconds at 30 frames a second, and a still of 200 by 100
        // (a JPEG: the renderer refuses the RGB frames FFmpeg decodes PNG files to).
        ffmpeg(
            &work,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=30:duration=2",
                "-c:v",
                "mpeg4",
                "-q:v",
                "3",
                "moving.mov",
            ],
        );
        ffmpeg(
            &work,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=200x100",
                "-frames:v",
                "1",
                "still.jpg",
            ],
        );
        ffmpeg(&work, &["-f", "lavfi", "-i", "sine=duration=1", "tone.wav"]);
        let moving = add(&workspace, &work, "moving.mov");
        let still = add(&workspace, &work, "still.jpg");
        let tone = add(&workspace, &work, "tone.wav");
        let before = revision(&workspace);
        let views = MediaViews::new(&cache);

        let strip = views
            .thumbnails(&workspace, &moving, None, Some(4), None)
            .unwrap();
        assert_eq!(strip.asset, moving);
        assert_eq!(
            (strip.width, strip.height),
            (128, 72),
            "16:9 at the default height"
        );
        let times: Vec<i64> = strip.frames.iter().map(|f| f.flicks).collect();
        assert_eq!(
            times,
            [SECOND / 4, 3 * SECOND / 4, 5 * SECOND / 4, 7 * SECOND / 4]
        );
        let images: Vec<Vec<u8>> = strip
            .frames
            .iter()
            .map(|frame| {
                let (png, width, height) = png_of(&frame.image);
                assert_eq!((width, height), (128, 72));
                png
            })
            .collect();
        assert_ne!(images[0], images[3], "the picture moves");
        let rgb = pixels(&work, &images[0]);
        assert_eq!(rgb.len(), 128 * 72 * 3);
        let bright = rgb.iter().filter(|&&v| v > 128).count();
        assert!(
            bright > rgb.len() / 10,
            "a test picture, not black: {bright}"
        );
        assert_eq!(thumbnail_files(&cache), 4, "kept on disk with the frames");
        assert_eq!(revision(&workspace), before, "the project is not changed");

        // Given times, at another height.
        let asked = views
            .thumbnails(
                &workspace,
                &moving,
                Some(&[SECOND / 4, SECOND]),
                None,
                Some(36),
            )
            .unwrap();
        assert_eq!((asked.width, asked.height), (64, 36));
        assert_eq!(asked.frames.len(), 2);
        assert_eq!(asked.frames[1].flicks, SECOND);
        let clamped = views
            .thumbnails(&workspace, &moving, Some(&[]), None, Some(1000))
            .unwrap();
        assert_eq!((clamped.height, clamped.frames.len()), (MAX_HEIGHT, 0));

        // An image: the same picture at every time, as wide as its aspect.
        let picture = views
            .thumbnails(
                &workspace,
                &still,
                Some(&[0, 5 * SECOND, -SECOND]),
                None,
                None,
            )
            .unwrap();
        assert_eq!((picture.width, picture.height), (144, 72));
        assert_eq!(picture.frames.len(), 3);
        assert!(
            picture
                .frames
                .iter()
                .all(|f| f.image == picture.frames[0].image)
        );
        assert_eq!(picture.frames[1].flicks, 5 * SECOND);
        let (png, ..) = png_of(&picture.frames[0].image);
        assert_eq!(pixels(&work, &png).len(), 144 * 72 * 3);

        // The second time, nothing renders: the file can be gone, and a new app reads the
        // disk, the running one its memory.
        std::fs::rename(work.join("moving.mov"), work.join("away.mov")).unwrap();
        let again = MediaViews::new(&cache);
        let from_disk = again
            .thumbnails(&workspace, &moving, None, Some(4), None)
            .unwrap();
        assert_eq!(from_disk, strip);
        let _ = std::fs::remove_dir_all(cache.join("frames"));
        assert_eq!(
            again
                .thumbnails(&workspace, &moving, None, Some(4), None)
                .unwrap(),
            strip,
            "from memory"
        );
        let missing = again
            .thumbnails(&workspace, &moving, Some(&[SECOND / 2]), None, None)
            .unwrap_err();
        assert_eq!(missing.code, "fosscut.export.missing_media", "{missing:?}");
        again.forget();
        assert!(
            again
                .thumbnails(&workspace, &moving, None, Some(4), None)
                .is_err(),
            "forgotten, and the file is gone"
        );

        // Refused: sound, unknown assets, too many times, no project.
        let audio = views
            .thumbnails(&workspace, &tone, None, None, None)
            .unwrap_err();
        assert_eq!(audio.code, "fosscut.media.no_picture");
        assert_eq!(audio.details["kind"], "audio");
        let unknown = views
            .thumbnails(&workspace, "asset_0zzz", None, None, None)
            .unwrap_err();
        assert_eq!(unknown.code, codes::ASSET_NOT_FOUND);
        let bad = views
            .thumbnails(&workspace, "not an id", None, None, None)
            .unwrap_err();
        assert_eq!(bad.code, codes::INVALID_ARGUMENTS);
        let many = views
            .thumbnails(&workspace, &still, Some(&[0; 49]), None, None)
            .unwrap_err();
        assert!(many.message.contains("at most 48"), "{many:?}");
        workspace.close();
        let closed = views
            .thumbnails(&workspace, &still, None, None, None)
            .unwrap_err();
        assert!(closed.message.contains("no project"), "{closed:?}");
        done(workspace, runtime, work);
    }

    #[test]
    fn waveforms_measure_the_sound_once_and_refuse_pictures() {
        if !ffmpeg_programs() {
            eprintln!("skipped: ffmpeg or ffprobe is missing");
            return;
        }
        let (workspace, runtime, work) = opened("waves");
        let cache = work.join("cache");
        // Two seconds of a 440 Hz tone at half scale, then a second of silence, mono.
        ffmpeg(
            &work,
            &[
                "-f",
                "lavfi",
                "-i",
                "aevalsrc='if(lt(t,2),0.5*sin(2*PI*440*t),0)':s=48000:d=3",
                "-c:a",
                "pcm_s16le",
                "tone.wav",
            ],
        );
        // A second of the tone on the left only, silence on the right.
        ffmpeg(
            &work,
            &[
                "-f",
                "lavfi",
                "-i",
                "aevalsrc='0.5*sin(2*PI*440*t)|0':s=44100:d=1",
                "-c:a",
                "pcm_s16le",
                "left.wav",
            ],
        );
        // A picture with sound, one without, and an image.
        ffmpeg(
            &work,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=30:duration=1",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc='0.25*sin(2*PI*220*t)':s=48000:d=1",
                "-c:v",
                "mpeg4",
                "-c:a",
                "pcm_s16le",
                "-shortest",
                "talk.mov",
            ],
        );
        ffmpeg(
            &work,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=30:duration=1",
                "-c:v",
                "mpeg4",
                "mute.mov",
            ],
        );
        ffmpeg(
            &work,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64",
                "-frames:v",
                "1",
                "still.png",
            ],
        );
        let tone = add(&workspace, &work, "tone.wav");
        let left = add(&workspace, &work, "left.wav");
        let talk = add(&workspace, &work, "talk.mov");
        let mute = add(&workspace, &work, "mute.mov");
        let still = add(&workspace, &work, "still.png");
        let views = MediaViews::new(&cache);

        let wave = views.waveform(&workspace, &tone, 30).unwrap();
        assert_eq!(wave.asset, tone);
        assert_eq!(wave.duration, 3 * SECOND);
        assert_eq!((wave.buckets, wave.peaks.len()), (30, 30));
        // Bucket 20 begins where the tone ends: merged from the finest measurement, its
        // edge may reach a sample of the tone.
        for (i, [low, high]) in wave.peaks.iter().enumerate() {
            if i < 20 {
                let full = (high - 0.5).abs() < 0.01 && (low + 0.5).abs() < 0.01;
                assert!(full, "{i}: {low} {high}");
            } else if i > 20 {
                let silent = low.abs() < 0.001 && high.abs() < 0.001;
                assert!(silent, "{i} is silent: {low} {high}");
            }
        }
        let analyses = fosscut_engine::cache::cache_info(&cache)
            .into_iter()
            .find(|usage| usage.kind == fosscut_engine::cache::CacheKind::Analysis)
            .unwrap();
        assert_eq!(analyses.entries, 1, "kept with the analyses");
        let kept = std::fs::read_dir(cache.join("analysis/waveforms"))
            .unwrap()
            .count();
        assert_eq!(kept, 1);

        // Another bucket count, and another app, come from the measurement kept: the file
        // can be gone.
        std::fs::rename(work.join("tone.wav"), work.join("away.wav")).unwrap();
        let coarse = MediaViews::new(&cache)
            .waveform(&workspace, &tone, 6)
            .unwrap();
        assert_eq!(coarse.peaks.len(), 6);
        assert!(
            coarse.peaks[..4]
                .iter()
                .all(|[_, high]| (high - 0.5).abs() < 0.01)
        );
        assert!(coarse.peaks[5].iter().all(|v| v.abs() < 0.001));
        assert_eq!(views.waveform(&workspace, &tone, 30).unwrap(), wave);
        let fine = views.waveform(&workspace, &tone, 200_000).unwrap();
        assert_eq!(fine.buckets, MAX_BUCKETS, "clamped");
        assert_eq!(fine.peaks.len(), MAX_BUCKETS as usize);
        assert_eq!(views.waveform(&workspace, &tone, 0).unwrap().peaks.len(), 1);

        // Every channel counts: the tone on the left shows at full height.
        let sided = views.waveform(&workspace, &left, 4).unwrap();
        assert!(
            sided
                .peaks
                .iter()
                .all(|[_, high]| (high - 0.5).abs() < 0.01),
            "{sided:?}"
        );
        // A video's sound.
        let spoken = views.waveform(&workspace, &talk, 5).unwrap();
        assert!(
            spoken
                .peaks
                .iter()
                .all(|[low, high]| (high - 0.25).abs() < 0.01 && (low + 0.25).abs() < 0.01),
            "{spoken:?}"
        );

        // Refused: no sound to show.
        let silent = views.waveform(&workspace, &mute, 10).unwrap_err();
        assert_eq!(silent.code, "fosscut.media.no_sound");
        assert!(silent.message.contains("without sound"), "{silent:?}");
        let picture = views.waveform(&workspace, &still, 10).unwrap_err();
        assert_eq!(picture.code, "fosscut.media.no_sound");
        assert_eq!(picture.details["kind"], "image");
        assert_eq!(
            views
                .waveform(&workspace, "asset_0zzz", 10)
                .unwrap_err()
                .code,
            codes::ASSET_NOT_FOUND
        );
        done(workspace, runtime, work);
    }

    #[test]
    fn importing_adds_every_file_it_can_as_one_edit_and_reports_the_rest() {
        if !ffmpeg_programs() {
            eprintln!("skipped: ffmpeg or ffprobe is missing");
            return;
        }
        let (workspace, runtime, work) = opened("import");
        ffmpeg(&work, &["-f", "lavfi", "-i", "sine=duration=1", "a.wav"]);
        ffmpeg(
            &work,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64",
                "-frames:v",
                "1",
                "b.png",
            ],
        );
        std::fs::write(work.join("notes.xyz"), "not media").unwrap();
        let before = revision(&workspace);
        let files = [
            work.join("a.wav"),
            work.join("notes.xyz"),
            work.join("gone.mp4"),
            work.join("b.png"),
        ];
        let imported = import(&workspace, &files).unwrap();
        assert_eq!(imported.assets.len(), 2, "{imported:?}");
        assert_eq!(imported.revision, before + 1, "one edit");
        assert_eq!(revision(&workspace), imported.revision);
        let failed: Vec<&str> = imported.failed.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            failed,
            [
                work.join("notes.xyz").display().to_string(),
                work.join("gone.mp4").display().to_string()
            ]
        );
        assert!(
            imported
                .failed
                .iter()
                .all(|f| f.code == codes::PROBE_FAILED && !f.error.is_empty())
        );
        let listed = workspace.query("media.list", Value::Null).unwrap();
        let names: Vec<&str> = listed["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["a.wav", "b.png"]);
        let ids: Vec<&str> = listed["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, imported.assets);
        let history = workspace
            .query("history.list", json!({ "limit": 1 }))
            .unwrap();
        let entry = &history["entries"][0];
        assert_eq!(
            (entry["actor"].as_str(), entry["reason"].as_str()),
            (Some("human"), Some("Import media"))
        );

        // Nothing importable changes nothing; nothing at all neither.
        let none = import(&workspace, &[work.join("notes.xyz")]).unwrap();
        assert!(none.assets.is_empty() && none.failed.len() == 1);
        assert_eq!(none.revision, imported.revision);
        let empty = import(&workspace, &[]).unwrap();
        assert_eq!((empty.assets.len(), empty.revision), (0, imported.revision));
        let serialized = serde_json::to_value(&imported).unwrap();
        assert!(serialized["failed"][0]["path"].is_string() && serialized["revision"].is_u64());
        workspace.close();
        assert!(
            import(&workspace, &files)
                .unwrap_err()
                .message
                .contains("no project")
        );
        done(workspace, runtime, work);
    }

    /// A small mark drawn for these tests: an orange disc with a white stroke and an open,
    /// round-capped line, 200 by 100 units.
    const MARK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 200 100">
  <circle id="disc" cx="50" cy="50" r="40" fill="#FF7A45" stroke="#FFFFFF" stroke-width="4"/>
  <path id="swoosh" d="M 110 70 Q 150 10 190 60" fill="none" stroke="#2C62C0" stroke-width="8" stroke-linecap="round"/>
</svg>"##;

    #[test]
    fn drawings_import_explode_from_the_window_and_show_one_sharp_picture() {
        let (workspace, runtime, work) = opened("vector");
        std::fs::write(work.join("mark.svg"), MARK).unwrap();
        let asset = add(&workspace, &work, "mark.svg");
        let listed = workspace.query("media.list", Value::Null).unwrap();
        assert_eq!(listed["assets"][0]["kind"], "vector");
        assert_eq!(listed["assets"][0]["vector"]["paths"], 2);

        // The window sends only the asset; the paths come from the file.
        let exploded = workspace
            .execute(Request::single("vector.explode", json!({ "asset": asset })))
            .unwrap();
        let parts = exploded.results[0]["parts"].as_array().unwrap();
        let names: Vec<&str> = parts.iter().filter_map(|p| p["name"].as_str()).collect();
        assert_eq!(names, ["disc", "swoosh"], "{parts:?}");
        let compound = exploded.results[0]["timeline"].as_str().unwrap();
        let inside = workspace
            .query(
                "timeline.read",
                json!({ "timeline": compound, "detail": "full" }),
            )
            .unwrap();
        let shapes: Vec<&Value> = inside["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["clips"].as_array().unwrap())
            .collect();
        assert_eq!(shapes.len(), 2);
        assert!(shapes.iter().all(|c| c["kind"] == "shape"), "{shapes:?}");

        let cache = work.join("cache");
        let views = MediaViews::new(&cache);
        let silent = views.waveform(&workspace, &asset, 64).unwrap_err();
        assert_eq!(silent.code, "fosscut.media.no_sound");
        assert_eq!(silent.details["kind"], "vector");
        if !gpu() {
            eprintln!("skipped the pictures: no GPU adapter");
            done(workspace, runtime, work);
            return;
        }
        let strip = views
            .thumbnails(&workspace, &asset, Some(&[0, 3 * SECOND]), None, Some(40))
            .unwrap();
        assert_eq!(
            (strip.width, strip.height),
            (80, 40),
            "the drawing's aspect"
        );
        assert_eq!(
            strip.frames[0].image, strip.frames[1].image,
            "a drawing looks the same at every time"
        );
        let (_, width, height) = png_of(&strip.frames[0].image);
        assert_eq!((width, height), (80, 40));
        let made = thumbnail_files(&cache);
        assert_eq!(made, 1, "one picture, kept once");
        let again = views
            .thumbnails(&workspace, &asset, None, Some(4), Some(40))
            .unwrap();
        assert!(
            again
                .frames
                .iter()
                .all(|f| f.image == strip.frames[0].image)
        );
        assert_eq!(thumbnail_files(&cache), made, "from the caches");
        done(workspace, runtime, work);
    }

    #[test]
    fn compositions_show_their_first_frame_or_say_why_not() {
        let (workspace, runtime, work) = opened("webthumb");
        std::fs::write(
            work.join("card.html"),
            r#"<!doctype html><html><head>
<meta name="fosscut:composition" content="width=320, height=180, fps=30, duration=2">
<style>html,body{margin:0;background:#2C62C0}</style></head><body></body></html>"#,
        )
        .unwrap();
        let asset = add(&workspace, &work, "card.html");
        let listed = workspace.query("media.list", Value::Null).unwrap();
        assert_eq!(listed["assets"][0]["kind"], "web");
        let views = MediaViews::new(&work.join("cache"));
        let silent = views.waveform(&workspace, &asset, 16).unwrap_err();
        assert_eq!(silent.code, "fosscut.media.no_sound");
        if !gpu() {
            eprintln!("skipped the pictures: no GPU adapter");
            done(workspace, runtime, work);
            return;
        }
        let asked = views.thumbnails(&workspace, &asset, Some(&[0, SECOND]), None, Some(36));
        match fosscut_engine::web::find_browser() {
            None => {
                let error = asked.unwrap_err();
                assert_eq!(error.code, "fosscut.web.browser_missing", "{error:?}");
            }
            Some(_) => {
                let strip = asked.unwrap();
                assert_eq!((strip.width, strip.height), (64, 36));
                assert_eq!(
                    strip.frames[0].image, strip.frames[1].image,
                    "the first frame stands for the composition"
                );
                assert_eq!(
                    strip.frames.iter().map(|f| f.flicks).collect::<Vec<_>>(),
                    [0, SECOND],
                    "the times asked for are answered"
                );
            }
        }
        done(workspace, runtime, work);
    }
}
