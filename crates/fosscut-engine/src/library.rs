//! The built-in library (ADR-0028): shapes, stickers, backgrounds, project and title
//! templates, text and caption presets, looks, and sound effects and music loops that
//! FossCut generates itself, so everything is free to use, also commercially, and nothing
//! is downloaded. Sounds, drawings (device frames and a map) and web compositions are
//! written into the bundle's `library/` folder the first time they are used and then
//! imported like any file.

use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;

/// The bundle folder generated sounds are written to, as relative paths start.
pub const FOLDER: &str = "library/";
/// The sample rate sounds are generated at.
const RATE: u32 = 48_000;

/// One item of the library.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LibraryItem {
    /// Stable name, unique within its kind.
    pub name: String,
    /// `shape`, `sticker`, `background`, `template`, `title`, `text_style`,
    /// `caption_style`, `look`, `particles`, `model` (a 3D object), `device`, `drawing`,
    /// `composition`, `sound` or `music`.
    pub kind: &'static str,
    /// Display title.
    pub title: String,
    /// What it is and what it suits.
    pub description: String,
    /// Search words.
    pub tags: Vec<String>,
    /// A sound's length in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// A music loop's tempo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    /// Free to use in commercial work: true for everything FossCut makes.
    pub commercial: bool,
    /// A device frame's open screen, in the drawing's pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen: Option<fosscut_core::DeviceScreen>,
    /// A composition's lines, which `lines` replace in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    /// A composition's parameters and slots, which `params` and `media` set (ADR-0042).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<fosscut_core::CompositionParam>,
}

fn item(name: &str, kind: &'static str, title: &str, description: &str) -> LibraryItem {
    LibraryItem {
        name: name.to_string(),
        kind,
        title: title.to_string(),
        description: description.to_string(),
        tags: Vec::new(),
        seconds: None,
        bpm: None,
        commercial: true,
        screen: None,
        lines: Vec::new(),
        params: Vec::new(),
    }
}

/// Every item of the library.
pub fn catalog() -> Vec<LibraryItem> {
    let mut items = Vec::new();
    for visual in fosscut_core::visuals() {
        let kind = match visual.kind {
            fosscut_core::VisualKind::Shape => "shape",
            fosscut_core::VisualKind::Sticker => "sticker",
            fosscut_core::VisualKind::Background => "background",
        };
        items.push(LibraryItem {
            tags: visual.tags,
            ..item(&visual.name, kind, &visual.title, &visual.description)
        });
    }
    for template in fosscut_core::project_templates() {
        items.push(item(
            &template.name,
            "template",
            &template.name,
            &template.description,
        ));
    }
    for title in fosscut_core::text_templates() {
        items.push(item(&title.name, "title", &title.name, &title.description));
    }
    for preset in fosscut_core::text_style_presets() {
        items.push(item(
            &preset.name,
            "text_style",
            &preset.name,
            &preset.description,
        ));
    }
    for preset in fosscut_core::caption_presets() {
        items.push(item(
            &preset.name,
            "caption_style",
            &preset.name,
            &preset.description,
        ));
    }
    for look in fosscut_core::looks() {
        items.push(item(&look.name, "look", &look.title, &look.description));
    }
    // Particle emitters (ADR-0038).
    for preset in fosscut_core::particle_presets() {
        items.push(LibraryItem {
            tags: preset.tags,
            ..item(
                &preset.name,
                "particles",
                &preset.title,
                &preset.description,
            )
        });
    }
    // Device frames, drawings and web compositions, imported as media when added.
    for drawing in fosscut_core::drawings() {
        let kind = match drawing.kind {
            fosscut_core::DrawingKind::Device => "device",
            fosscut_core::DrawingKind::Drawing => "drawing",
        };
        items.push(LibraryItem {
            tags: drawing.tags,
            screen: drawing.screen,
            ..item(&drawing.name, kind, &drawing.title, &drawing.description)
        });
    }
    for composition in fosscut_core::compositions() {
        items.push(LibraryItem {
            lines: composition.lines(),
            params: composition.all_params(),
            tags: composition.tags,
            ..item(
                &composition.name,
                "composition",
                &composition.title,
                &composition.description,
            )
        });
    }
    // 3D objects (ADR-0040).
    for preset in fosscut_core::model_presets() {
        items.push(LibraryItem {
            tags: preset.tags,
            ..item(&preset.name, "model", &preset.title, &preset.description)
        });
    }
    for sound in fosscut_audio::synth::sounds() {
        let kind = match sound.kind {
            fosscut_audio::synth::SoundKind::Effect => "sound",
            fosscut_audio::synth::SoundKind::Music => "music",
        };
        items.push(LibraryItem {
            tags: sound.tags.iter().map(|t| t.to_string()).collect(),
            seconds: Some(sound.seconds),
            bpm: sound.bpm,
            ..item(sound.name, kind, sound.title, sound.description)
        });
    }
    items
}

/// The items of `kind` (any when `None`) whose name, title, description or tags contain
/// every word of `words`, case-insensitively.
pub fn search(kind: Option<&str>, words: Option<&str>) -> Vec<LibraryItem> {
    let words: Vec<String> = words
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    catalog()
        .into_iter()
        .filter(|i| kind.is_none_or(|k| i.kind == k))
        .filter(|i| {
            let text = format!(
                "{} {} {} {}",
                i.name,
                i.title,
                i.description,
                i.tags.join(" ")
            )
            .to_lowercase();
            words.iter().all(|w| text.contains(w))
        })
        .collect()
}

/// The item called `name`, of `kind` when given (names repeat across kinds, such as
/// `highlight`, a text style and a caption style).
pub fn find(name: &str, kind: Option<&str>) -> Result<LibraryItem, EngineError> {
    let matches: Vec<LibraryItem> = catalog()
        .into_iter()
        .filter(|i| i.name == name && kind.is_none_or(|k| i.kind == k))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(EngineError::InvalidArguments {
            message: format!("the library has no {} {name:?}", kind.unwrap_or("item")),
        }),
        several => Err(EngineError::InvalidArguments {
            message: format!(
                "{name:?} is a {}; say which kind",
                several
                    .iter()
                    .map(|i| i.kind)
                    .collect::<Vec<_>>()
                    .join(" and a ")
            ),
        }),
    }
}

/// Writes sound or music `name` into the bundle's library folder, once, and returns the
/// `media.add` arguments that import it (`media.add` gives the existing asset back when
/// the project has it already).
pub fn sound_media(bundle: &Path, name: &str) -> Result<Value, EngineError> {
    let samples =
        fosscut_audio::synth::render(name, RATE).ok_or_else(|| EngineError::InvalidArguments {
            message: format!("the library has no sound {name:?}"),
        })?;
    let relative = format!("{FOLDER}{name}.wav");
    let path = bundle.join(&relative);
    let bytes = fosscut_audio::wav::wav_bytes(&samples, 2, RATE);
    let current = std::fs::read(&path).ok();
    if current.as_deref() != Some(bytes.as_slice()) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| EngineError::InvalidArguments {
                message: format!("cannot create {}: {e}", parent.display()),
            })?;
        }
        std::fs::write(&path, &bytes).map_err(|e| EngineError::InvalidArguments {
            message: format!("cannot write {}: {e}", path.display()),
        })?;
    }
    let identity = crate::import::identity(&path).map_err(|e| EngineError::Failed {
        code: e.code,
        message: e.message,
    })?;
    let frames = samples.len() / 2;
    let duration = fosscut_time::Flicks::new(
        (frames as i128 * i128::from(fosscut_time::FLICKS_PER_SECOND) / i128::from(RATE)) as i64,
    );
    Ok(json!({
        "name": format!("{name}.wav"),
        "kind": "audio",
        "path": { "relative": relative },
        "identity": identity,
        "info": {
            "duration": duration.get(),
            "container": "wav",
            "audio": { "sample_rate": RATE, "channels": 2, "codec": "pcm_s16le" },
        },
    }))
}

/// The longest music bed [`music_bed_media`] writes: ten minutes.
pub const MAX_BED: fosscut_time::Flicks = fosscut_time::Flicks::from_millis(600_000);

/// How long the end of a music bed fades out: half a second.
const BED_FADE_SECONDS: f64 = 0.5;

/// Music loop `name` repeated back to back to last exactly `length`, its last half second
/// faded out (the last repeat trimmed where the length ends), written into the bundle's
/// library folder once per length. Returns the `media.add` arguments that import it, so a
/// bed of any length is one clip and one asset.
pub fn music_bed_media(
    bundle: &Path,
    name: &str,
    length: fosscut_time::Flicks,
) -> Result<Value, EngineError> {
    let is_music = fosscut_audio::synth::sounds()
        .iter()
        .any(|s| s.name == name && s.kind == fosscut_audio::synth::SoundKind::Music);
    if !is_music {
        return Err(EngineError::InvalidArguments {
            message: format!("{name} is not a music loop; only music fills a length"),
        });
    }
    if !length.is_positive() || length > MAX_BED {
        return Err(EngineError::InvalidArguments {
            message: format!(
                "a music bed lasts more than 0 s and at most {} s, not {} s",
                MAX_BED.as_seconds_f64(),
                length.as_seconds_f64()
            ),
        });
    }
    let samples =
        fosscut_audio::synth::render(name, RATE).ok_or_else(|| EngineError::InvalidArguments {
            message: format!("the library has no sound {name:?}"),
        })?;
    let frames = (i128::from(length.get()) * i128::from(RATE)
        / i128::from(fosscut_time::FLICKS_PER_SECOND)) as usize;
    let bed = fill(
        &samples,
        2,
        frames,
        (BED_FADE_SECONDS * f64::from(RATE)) as usize,
    );
    let millis = (length.as_seconds_f64() * 1000.0).round() as i64;
    let relative = format!("{FOLDER}{name}-{millis}ms.wav");
    let path = bundle.join(&relative);
    let bytes = fosscut_audio::wav::wav_bytes(&bed, 2, RATE);
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| EngineError::InvalidArguments {
                message: format!("cannot create {}: {e}", parent.display()),
            })?;
        }
        std::fs::write(&path, &bytes).map_err(|e| EngineError::InvalidArguments {
            message: format!("cannot write {}: {e}", path.display()),
        })?;
    }
    let identity = crate::import::identity(&path).map_err(|e| EngineError::Failed {
        code: e.code,
        message: e.message,
    })?;
    let duration = fosscut_time::Flicks::new(
        (frames as i128 * i128::from(fosscut_time::FLICKS_PER_SECOND) / i128::from(RATE)) as i64,
    );
    let seconds = format!("{}", (length.as_seconds_f64() * 1000.0).round() / 1000.0);
    Ok(json!({
        "name": format!("{name} {seconds}s.wav"),
        "kind": "audio",
        "path": { "relative": relative },
        "identity": identity,
        "info": {
            "duration": duration.get(),
            "container": "wav",
            "audio": { "sample_rate": RATE, "channels": 2, "codec": "pcm_s16le" },
        },
    }))
}

/// `loop_samples` (interleaved, `channels` per frame) repeated back to back for `frames`
/// frames, the last `fade` frames faded out on a quarter cosine.
fn fill(loop_samples: &[f32], channels: usize, frames: usize, fade: usize) -> Vec<f32> {
    let length = loop_samples.len() / channels.max(1);
    if length == 0 {
        return vec![0.0; frames * channels];
    }
    let mut out = Vec::with_capacity(frames * channels);
    for frame in 0..frames {
        let at = (frame % length) * channels;
        out.extend_from_slice(&loop_samples[at..at + channels]);
    }
    let fade = fade.min(frames);
    for i in 0..fade {
        let left = (fade - i) as f64 / fade as f64;
        let gain = (left * std::f64::consts::FRAC_PI_2).sin() as f32;
        let frame = frames - fade + i;
        for sample in &mut out[frame * channels..(frame + 1) * channels] {
            *sample *= gain;
        }
    }
    out
}

/// True for the items that become media: sounds and music, device frames, drawings and web
/// compositions. [`media_args`] writes them into the bundle and gives the `media.add`
/// arguments; `clip.add` then places the asset.
pub fn imports_media(item: &LibraryItem) -> bool {
    matches!(
        item.kind,
        "sound" | "music" | "device" | "drawing" | "composition"
    )
}

/// Writes media item `item` into the bundle's library folder, once per content, and
/// returns the `media.add` arguments that import it (`media.add` gives the existing asset
/// back when the project has it already). `lines` replace a composition's lines in order.
pub fn media_args(
    bundle: &Path,
    item: &LibraryItem,
    lines: &[String],
) -> Result<Value, EngineError> {
    let input = CompositionInput {
        lines: lines.to_vec(),
        ..CompositionInput::default()
    };
    media_args_with(bundle, item, &input, None)
}

/// What a composition is added with besides its name (ADR-0042).
#[derive(Clone, Debug, Default)]
pub struct CompositionInput {
    /// Lines in place of its placeholder lines, in order.
    pub lines: Vec<String>,
    /// Values of its parameters by name (an object); slots take asset ids, and composition
    /// slots also library composition names.
    pub params: Option<Value>,
    /// Assets for the slots `params` leaves out, in the order the slots are declared.
    pub media: Vec<String>,
}

/// [`media_args`] with a composition's parameters and slots; `project` is the project at
/// `bundle`, needed when a slot names an asset.
pub fn media_args_with(
    bundle: &Path,
    item: &LibraryItem,
    input: &CompositionInput,
    project: Option<&fosscut_model::Project>,
) -> Result<Value, EngineError> {
    if item.kind != "composition" && (input.params.is_some() || !input.media.is_empty()) {
        return Err(EngineError::InvalidArguments {
            message: format!(
                "{} is a {}: only compositions take params, and only compositions and project templates take media",
                item.name, item.kind
            ),
        });
    }
    match item.kind {
        "sound" | "music" => sound_media(bundle, &item.name),
        "device" | "drawing" => drawing_media(bundle, &item.name),
        "composition" => composition_media_with(bundle, &item.name, input, project),
        other => Err(EngineError::InvalidArguments {
            message: format!(
                "{} is a {other}, not media: add it with its command",
                item.name
            ),
        }),
    }
}

fn failed(error: fosscut_core::ApiError) -> EngineError {
    EngineError::Failed {
        code: error.code,
        message: error.message,
    }
}

/// Where library file `file` of item `stem` with content `bytes` goes, relative to the
/// bundle: `library/<stem><file>` while that is free or holds the same bytes, otherwise
/// with a short hash of the content after the stem, so the file an asset uses never
/// changes under it.
fn place(bundle: &Path, stem: &str, file: &str, bytes: &[u8]) -> String {
    let plain = format!("{FOLDER}{stem}{file}");
    match std::fs::read(bundle.join(&plain)) {
        Ok(current) if current != bytes => {
            let hash = blake3::hash(bytes).to_hex();
            format!("{FOLDER}{stem}-{}{file}", &hash.as_str()[..8])
        }
        _ => plain,
    }
}

/// Writes `bytes` to `path` unless it holds them already.
fn write_once(path: &Path, bytes: &[u8]) -> Result<(), EngineError> {
    if std::fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| EngineError::InvalidArguments {
            message: format!("cannot create {}: {e}", parent.display()),
        })?;
    }
    std::fs::write(path, bytes).map_err(|e| EngineError::InvalidArguments {
        message: format!("cannot write {}: {e}", path.display()),
    })
}

/// Writes device frame or drawing `name` into the bundle's library folder as an SVG file
/// and returns the `media.add` arguments that import it as a vector drawing (ADR-0036).
pub fn drawing_media(bundle: &Path, name: &str) -> Result<Value, EngineError> {
    let drawing = fosscut_core::drawing(name).map_err(failed)?;
    let bytes = drawing.svg().as_bytes();
    let relative = place(bundle, name, ".svg", bytes);
    let path = bundle.join(&relative);
    write_once(&path, bytes)?;
    let mut args = crate::import::media_add_args(&path, Some(&format!("{name}.svg")), bundle)
        .map_err(failed)?;
    args["path"] = json!({ "relative": relative });
    Ok(args)
}

/// Writes web composition `name` with `lines` into a folder of the bundle's library folder
/// and returns the `media.add` arguments that import it (ADR-0039); the asset points at
/// that folder, so it is not copied into `web/` as well.
pub fn composition_media(
    bundle: &Path,
    name: &str,
    lines: &[String],
) -> Result<Value, EngineError> {
    let input = CompositionInput {
        lines: lines.to_vec(),
        ..CompositionInput::default()
    };
    composition_media_with(bundle, name, &input, None)
}

/// Writes web composition `name` into a folder of the bundle's library folder with
/// `input`'s lines, its parameters as `params.json` and its slots' files (ADR-0042), and
/// returns the `media.add` arguments that import it. The folder is `library/<name>/` while
/// that is free or holds the same files, otherwise `library/<name>-<hash>/` after the
/// content, so other parameters make another folder and asset, and a folder an asset uses
/// never changes. `project` is the project at `bundle`, for slots that name assets.
pub fn composition_media_with(
    bundle: &Path,
    name: &str,
    input: &CompositionInput,
    project: Option<&fosscut_model::Project>,
) -> Result<Value, EngineError> {
    let composition = fosscut_core::composition(name).map_err(failed)?;
    let params = composition
        .resolve_params(input.params.as_ref(), &input.media)
        .map_err(failed)?;
    let files = composition_files(bundle, &composition, &input.lines, &params, project)?;
    let folder = place_folder(bundle, name, &files)?;
    let relative = format!(
        "{FOLDER}{}/index.html",
        folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
    crate::web::media_add_args_in_place(&folder, name, &relative).map_err(failed)
}

/// One file of a composition folder: bytes, or a file of the project to copy.
enum Source {
    Bytes(std::borrow::Cow<'static, [u8]>),
    File(std::path::PathBuf),
}

impl Source {
    fn bytes(bytes: Vec<u8>) -> Source {
        Source::Bytes(std::borrow::Cow::Owned(bytes))
    }
}

/// The files of a composition folder by their paths in it.
type Files = std::collections::BTreeMap<String, Source>;

fn invalid_params(composition: &str, message: String) -> EngineError {
    EngineError::Failed {
        code: fosscut_core::codes::LIBRARY_INVALID_PARAMS.to_string(),
        message: format!("{composition}: {message}"),
    }
}

/// The files of `composition`'s folder: its page with `lines` and its canvas values, its
/// own and its kit's files, and when it has parameters `params.json` with every value and
/// the slots' files under `media/`. A slot's value in `params.json` is `null` (the page's
/// own) or `{"src", "kind", "name"}` with, for compositions, their `width`, `height`, `fps`
/// and `duration`.
fn composition_files(
    bundle: &Path,
    composition: &fosscut_core::LibraryComposition,
    lines: &[String],
    params: &serde_json::Map<String, Value>,
    project: Option<&fosscut_model::Project>,
) -> Result<Files, EngineError> {
    let mut files = Files::new();
    files.insert(
        "index.html".into(),
        Source::bytes(composition.html_with(lines, params).into_bytes()),
    );
    for (path, bytes) in composition.files() {
        files.insert(path, Source::Bytes(std::borrow::Cow::Borrowed(bytes)));
    }
    let declared = composition.all_params();
    if declared.is_empty() {
        return Ok(files);
    }
    let mut written = params.clone();
    for param in declared
        .iter()
        .filter(|p| p.kind == fosscut_core::ParamType::Slot)
    {
        let value = match params.get(&param.name) {
            Some(Value::String(text)) if text.starts_with("asset_") => {
                slot_asset(bundle, &composition.name, param, text, project, &mut files)?
            }
            Some(Value::String(name)) => slot_composition(param, name, &mut files)?,
            _ => Value::Null,
        };
        written.insert(param.name.clone(), value);
    }
    let json = serde_json::to_vec_pretty(&Value::Object(written)).unwrap_or_default();
    files.insert("params.json".into(), Source::bytes(json));
    Ok(files)
}

/// Copies the file of asset `id` (a folder for web compositions and `.gltf` models) into
/// `files` for slot `param`, and gives the slot's value.
fn slot_asset(
    bundle: &Path,
    composition: &str,
    param: &fosscut_core::CompositionParam,
    id: &str,
    project: Option<&fosscut_model::Project>,
    files: &mut Files,
) -> Result<Value, EngineError> {
    use fosscut_core::SlotKind;
    use fosscut_model::AssetKind;
    let project = project.ok_or_else(|| EngineError::InvalidArguments {
        message: format!(
            "{} takes asset {id}: add the composition to a project",
            param.name
        ),
    })?;
    let parsed = fosscut_model::Id::parse(id)
        .map_err(|e| invalid_params(composition, format!("{}: {e}", param.name)))?;
    let asset = project.media.get(&parsed).ok_or_else(|| {
        invalid_params(
            composition,
            format!("{}: the project has no asset {id}", param.name),
        )
    })?;
    let kind = match asset.kind {
        AssetKind::Image => Some(SlotKind::Image),
        AssetKind::Video => Some(SlotKind::Video),
        AssetKind::Vector => Some(SlotKind::Svg),
        AssetKind::Model => Some(SlotKind::Model),
        AssetKind::Web => Some(SlotKind::Composition),
        _ => None,
    };
    let Some(kind) = kind.filter(|k| param.accepts.contains(k)) else {
        return Err(invalid_params(
            composition,
            format!(
                "{} takes {}, and {id} is {} asset",
                param.name,
                param
                    .accepts
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(" or "),
                match asset.kind {
                    AssetKind::Image => "an image",
                    AssetKind::Audio => "an audio",
                    AssetKind::Video => "a video",
                    AssetKind::Lut => "a LUT",
                    AssetKind::Font => "a font",
                    AssetKind::Vector => "an SVG",
                    AssetKind::Web => "a web composition",
                    AssetKind::Model => "a model",
                }
            ),
        ));
    };
    let path = crate::import::asset_file(project, bundle, &parsed).map_err(failed)?;
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let whole_folder =
        kind == SlotKind::Composition || (kind == SlotKind::Model && extension == "gltf");
    let src = if whole_folder {
        let folder = path.parent().unwrap_or(bundle);
        for relative in folder_files(folder)? {
            let from = relative
                .split('/')
                .fold(folder.to_path_buf(), |p, part| p.join(part));
            files.insert(
                format!("media/{}/{relative}", param.name),
                Source::File(from),
            );
        }
        format!("media/{}/{file}", param.name)
    } else {
        let src = format!("media/{}.{extension}", param.name);
        files.insert(src.clone(), Source::File(path.clone()));
        src
    };
    let mut value = json!({ "src": src, "kind": kind.as_str(), "name": asset.name });
    if kind == SlotKind::Composition
        && let Some(found) = crate::web::Composition::of(asset)
    {
        canvas_into(&mut value, &found);
    }
    Ok(value)
}

/// Writes library composition `name` into `files` under `media/<slot>/` for slot `param`,
/// with its default parameters, and gives the slot's value.
fn slot_composition(
    param: &fosscut_core::CompositionParam,
    name: &str,
    files: &mut Files,
) -> Result<Value, EngineError> {
    let other = fosscut_core::composition(name).map_err(failed)?;
    let defaults = other.resolve_params(None, &[]).map_err(failed)?;
    let html = other.html_with(&[], &defaults);
    let canvas = crate::web::parse_meta(&html)
        .and_then(|options| options.resolve())
        .map_err(EngineError::from)?;
    for (path, source) in composition_files(Path::new(""), &other, &[], &defaults, None)? {
        files.insert(format!("media/{}/{path}", param.name), source);
    }
    let mut value = json!({
        "src": format!("media/{}/index.html", param.name),
        "kind": "composition",
        "name": name,
    });
    canvas_into(&mut value, &canvas);
    Ok(value)
}

fn canvas_into(value: &mut Value, canvas: &crate::web::Composition) {
    value["width"] = json!(canvas.width);
    value["height"] = json!(canvas.height);
    value["fps"] = json!(f64::from(canvas.rate.num()) / f64::from(canvas.rate.den()));
    value["duration"] = json!(canvas.duration.as_seconds_f64());
}

/// The files under `folder` (relative paths with `/`), skipping hidden entries and
/// `node_modules` as composition imports do.
fn folder_files(folder: &Path) -> Result<Vec<String>, EngineError> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let path = entry.path();
            if path.is_dir() {
                walk(&path, &relative, out)?;
            } else if path.is_file() {
                out.push(relative);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(folder, "", &mut out).map_err(|e| EngineError::InvalidArguments {
        message: format!("cannot read {}: {e}", folder.display()),
    })?;
    out.sort();
    Ok(out)
}

/// The content identity `files` will have as a composition folder (as
/// `fosscut_engine::web` measures folders): the total size, and a BLAKE3 hash of every
/// file's path, size and bytes in path order.
fn files_identity(files: &Files) -> Result<fosscut_model::ContentIdentity, EngineError> {
    let mut hasher = blake3::Hasher::new();
    let mut total = 0u64;
    for (path, source) in files {
        hasher.update(path.as_bytes());
        hasher.update(&[0]);
        let size = match source {
            Source::Bytes(bytes) => {
                hasher.update(&(bytes.len() as u64).to_le_bytes());
                hasher.update(bytes);
                bytes.len() as u64
            }
            Source::File(file) => {
                let unreadable = |e: std::io::Error| EngineError::InvalidArguments {
                    message: format!("cannot read {}: {e}", file.display()),
                };
                let mut reader = std::fs::File::open(file).map_err(unreadable)?;
                let size = reader.metadata().map_err(unreadable)?.len();
                hasher.update(&size.to_le_bytes());
                std::io::copy(&mut reader, &mut hasher).map_err(unreadable)?;
                size
            }
        };
        total += size;
    }
    Ok(fosscut_model::ContentIdentity {
        size: total,
        sample_blake3: hasher.finalize().to_hex().to_string(),
    })
}

/// The folder in the bundle's library folder that holds `files` for composition `name`,
/// written there unless it holds them already: `library/<name>/` while that is free or
/// the same, otherwise `library/<name>-<first 8 hex of the content hash>/`.
fn place_folder(
    bundle: &Path,
    name: &str,
    files: &Files,
) -> Result<std::path::PathBuf, EngineError> {
    let identity = files_identity(files)?;
    let library = bundle.join(FOLDER.trim_end_matches('/'));
    let plain = library.join(name);
    if !plain.exists() {
        write_folder(&plain, files)?;
        return Ok(plain);
    }
    if crate::web::folder_identity(&plain).as_ref() == Some(&identity) {
        return Ok(plain);
    }
    let hashed = library.join(format!("{name}-{}", &identity.sample_blake3[..8]));
    if crate::web::folder_identity(&hashed).as_ref() != Some(&identity) {
        write_folder(&hashed, files)?;
    }
    Ok(hashed)
}

/// Writes `files` into `folder`: into a hidden folder beside it first, which then takes its
/// place, so the folder is never seen half written.
fn write_folder(folder: &Path, files: &Files) -> Result<(), EngineError> {
    let fail = |e: std::io::Error| EngineError::InvalidArguments {
        message: format!("cannot write {}: {e}", folder.display()),
    };
    let parent = folder.parent().unwrap_or(folder);
    std::fs::create_dir_all(parent).map_err(fail)?;
    let staging = parent.join(format!(
        ".{}.partial",
        folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    let _ = std::fs::remove_dir_all(&staging);
    let written = (|| {
        for (path, source) in files {
            let target = path
                .split('/')
                .fold(staging.clone(), |p, part| p.join(part));
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            match source {
                Source::Bytes(bytes) => std::fs::write(&target, bytes)?,
                Source::File(from) => {
                    std::fs::copy(from, &target)?;
                }
            }
        }
        if folder.exists() {
            std::fs::remove_dir_all(folder)?;
        }
        std::fs::rename(&staging, folder)
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(fail(error));
    }
    Ok(())
}

/// Where the open screen of device `item` lies on `timeline` of `project` (the primary
/// timeline when `None`) when the device is placed as clips are by default (fitted,
/// centred, at scale 1), and how to use it; `None` for items without a screen.
pub fn screen_report(
    item: &LibraryItem,
    project: &fosscut_model::Project,
    timeline: Option<&str>,
) -> Option<Value> {
    let screen = item.screen.as_ref()?;
    let drawing = fosscut_core::drawing(&item.name).ok()?;
    let format = match timeline {
        Some(id) => {
            &project
                .timelines
                .iter()
                .find(|(key, _)| key.as_str() == id)?
                .1
                .format
        }
        None => &project.primary_timeline()?.format,
    };
    let placed = screen.on_canvas(drawing.size, (format.width, format.height));
    Some(json!({
        "position": placed.position,
        "size": placed.size,
        "scale": placed.scale,
        "canvas": screen.canvas,
        "note": "The screen is open: put what it shows on a track below the device, with this transform.position and transform.scale (a compound timeline or web composition of this canvas fills it exactly). To move, scale or turn the device with its screen, keep both in one compound timeline and animate the compound clip.",
    }))
}

/// Where to put an item, for [`command_for`].
#[derive(Clone, Debug, Default)]
pub struct Placement {
    /// When it starts, as a time input.
    pub start: Option<Value>,
    /// How long it lasts, as a time input.
    pub duration: Option<Value>,
    /// The track.
    pub track: Option<String>,
    /// The timeline, such as a scene's compound timeline; the primary timeline when
    /// omitted. Project templates make a timeline of their own.
    pub timeline: Option<String>,
    /// The clip a look applies to.
    pub clip: Option<String>,
    /// Lines for titles and templates; the text of a text style's clip.
    pub lines: Vec<String>,
    /// Assets for a project template's slots.
    pub media: Vec<String>,
}

/// The command that adds `item`, except media items ([`imports_media`]), which are imported
/// first with [`media_args`] and then placed with `clip.add`.
pub fn command_for(item: &LibraryItem, at: &Placement) -> Result<Value, EngineError> {
    let mut args = serde_json::Map::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            args.insert(key.to_string(), value);
        }
    };
    let lines = (!at.lines.is_empty()).then(|| json!(at.lines));
    let command = match item.kind {
        "shape" | "sticker" | "background" => {
            put("item", Some(json!(item.name)));
            put("start", at.start.clone());
            put("duration", at.duration.clone());
            put("track", at.track.clone().map(Value::from));
            put("timeline", at.timeline.clone().map(Value::from));
            "shape.add"
        }
        "template" => {
            put("template", Some(json!(item.name)));
            put("lines", lines);
            put("media", (!at.media.is_empty()).then(|| json!(at.media)));
            "template.apply"
        }
        "title" => {
            put("template", Some(json!(item.name)));
            put("lines", lines);
            put("start", at.start.clone());
            put("duration", at.duration.clone());
            put("track", at.track.clone().map(Value::from));
            put("timeline", at.timeline.clone().map(Value::from));
            "text.template"
        }
        "text_style" => {
            let text = at.lines.join("\n");
            put(
                "text",
                Some(json!(if text.trim().is_empty() {
                    "Title"
                } else {
                    &text
                })),
            );
            put("preset", Some(json!(item.name)));
            put("start", at.start.clone());
            put("duration", at.duration.clone());
            put("track", at.track.clone().map(Value::from));
            put("timeline", at.timeline.clone().map(Value::from));
            "text.add"
        }
        "caption_style" => {
            put("preset", Some(json!(item.name)));
            put("track", at.track.clone().map(Value::from));
            "captions.style"
        }
        "particles" => {
            put("preset", Some(json!(item.name)));
            put("start", at.start.clone());
            put("duration", at.duration.clone());
            put("track", at.track.clone().map(Value::from));
            put("timeline", at.timeline.clone().map(Value::from));
            "particles.add"
        }
        "model" => {
            put("preset", Some(json!(item.name)));
            put("start", at.start.clone());
            put("duration", at.duration.clone());
            put("track", at.track.clone().map(Value::from));
            put("timeline", at.timeline.clone().map(Value::from));
            "model.add"
        }
        "look" => {
            let clip = at
                .clip
                .clone()
                .ok_or_else(|| EngineError::InvalidArguments {
                    message: format!("the look {} needs the clip to apply it to", item.name),
                })?;
            put("look", Some(json!(item.name)));
            put("clip", Some(json!(clip)));
            "look.apply"
        }
        _ => {
            return Err(EngineError::InvalidArguments {
                message: format!(
                    "{} is a {}: import it with media_args, then place it with clip.add",
                    item.name, item.kind
                ),
            });
        }
    };
    Ok(json!({ "command": command, "args": Value::Object(args) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_covers_every_kind_and_searches() {
        let all = catalog();
        for kind in [
            "shape",
            "sticker",
            "background",
            "template",
            "title",
            "text_style",
            "caption_style",
            "look",
            "particles",
            "model",
            "device",
            "drawing",
            "composition",
            "sound",
            "music",
        ] {
            assert!(all.iter().any(|i| i.kind == kind), "{kind}");
        }
        assert!(all.iter().all(|i| i.commercial));
        let sale = search(None, Some("SALE badge"));
        assert!(sale.iter().any(|i| i.name == "badge-sale"), "{sale:?}");
        assert!(search(Some("music"), None).iter().all(|i| i.bpm.is_some()));
        assert!(
            find("highlight", None).is_err(),
            "a text style and a caption style"
        );
        assert_eq!(
            find("highlight", Some("caption_style")).unwrap().kind,
            "caption_style"
        );
        assert!(find("nothing", None).is_err());
    }

    #[test]
    fn items_map_to_commands() {
        let at = Placement {
            start: Some(json!("1s")),
            lines: vec!["Sam".into()],
            clip: Some("clip_0003".into()),
            ..Placement::default()
        };
        let command =
            |name: &str, kind: &str| command_for(&find(name, Some(kind)).unwrap(), &at).unwrap();
        assert_eq!(command("badge-new", "sticker")["command"], "shape.add");
        assert_eq!(
            command("lower-third", "title")["args"]["lines"],
            json!(["Sam"])
        );
        assert_eq!(command("neon", "text_style")["args"]["text"], "Sam");
        assert_eq!(
            command("vertical-promo", "template")["command"],
            "template.apply"
        );
        assert_eq!(command("teal_orange", "look")["args"]["clip"], "clip_0003");
        assert_eq!(
            command("karaoke", "caption_style")["command"],
            "captions.style"
        );
        let embers = command("embers", "particles");
        assert_eq!(embers["command"], "particles.add");
        let city = command("city-blocks", "model");
        assert_eq!(city["command"], "model.add");
        assert_eq!(city["args"]["preset"], "city-blocks");
        assert_eq!(embers["args"]["preset"], "embers");
        assert_eq!(embers["args"]["start"], "1s");
        let sound = &search(Some("sound"), None)[0];
        assert!(command_for(sound, &at).is_err());
        let bare = Placement::default();
        assert!(command_for(&find("teal_orange", Some("look")).unwrap(), &bare).is_err());
    }

    #[test]
    fn sounds_are_written_once_into_the_bundle() {
        let bundle = std::env::temp_dir().join(format!("fosscut-library-{}", std::process::id()));
        std::fs::create_dir_all(&bundle).unwrap();
        let name = search(Some("sound"), None)[0].name.clone();
        let args = sound_media(&bundle, &name).unwrap();
        let path = bundle.join(format!("{FOLDER}{name}.wav"));
        assert!(path.is_file());
        assert_eq!(args["path"]["relative"], format!("{FOLDER}{name}.wav"));
        assert_eq!(args["info"]["audio"]["channels"], 2);
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            sound_media(&bundle, &name).unwrap(),
            args,
            "the same file again"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        assert!(sound_media(&bundle, "no-such-sound").is_err());
        std::fs::remove_dir_all(&bundle).unwrap();
    }

    #[test]
    fn loops_fill_a_length_back_to_back_and_fade_at_the_end() {
        // A loop of four stereo frames, filled to ten with a fade over the last two.
        let samples: Vec<f32> = (0..8).map(|i| 0.1 * (i / 2 + 1) as f32).collect();
        let filled = fill(&samples, 2, 10, 2);
        assert_eq!(filled.len(), 20);
        assert_eq!(&filled[..8], &samples[..], "the loop as it is");
        assert_eq!(&filled[8..12], &samples[..4], "then again from its start");
        assert_eq!(
            filled[16], samples[0],
            "the fade's first frame keeps full level"
        );
        let last = filled[18] / samples[2];
        assert!(
            (last - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
            "{last}"
        );
        assert!(fill(&[], 2, 3, 1).iter().all(|s| *s == 0.0));

        let bundle =
            std::env::temp_dir().join(format!("fosscut-library-bed-{}", std::process::id()));
        std::fs::create_dir_all(&bundle).unwrap();
        let music = search(Some("music"), None)[0].name.clone();
        let length = fosscut_time::Flicks::from_seconds(60).unwrap();
        let args = music_bed_media(&bundle, &music, length).unwrap();
        assert_eq!(args["info"]["duration"], length.get());
        let relative = format!("{FOLDER}{music}-60000ms.wav");
        assert_eq!(args["path"]["relative"], relative);
        let bytes = std::fs::read(bundle.join(&relative)).unwrap();
        // 60 s of 16-bit stereo at 48 kHz after the 44-byte header.
        assert_eq!(bytes.len(), 44 + 60 * 48_000 * 4);
        assert_eq!(args["name"], format!("{music} 60s.wav"));
        for (name, seconds) in [(music.as_str(), 0), (music.as_str(), 601), ("whoosh", 10)] {
            let error = music_bed_media(
                &bundle,
                name,
                fosscut_time::Flicks::from_seconds(seconds).unwrap(),
            )
            .unwrap_err();
            assert_eq!(error.code(), "fosscut.request.invalid_arguments");
        }
        std::fs::remove_dir_all(&bundle).unwrap();
    }

    #[test]
    fn drawings_and_compositions_are_written_once_into_the_bundle() {
        let bundle =
            std::env::temp_dir().join(format!("fosscut-library-media-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bundle);
        std::fs::create_dir_all(&bundle).unwrap();
        let phone = find("phone", None).unwrap();
        assert_eq!(phone.kind, "device");
        assert!(imports_media(&phone) && phone.screen.is_some());
        assert!(command_for(&phone, &Placement::default()).is_err());
        let args = media_args(&bundle, &phone, &[]).unwrap();
        assert_eq!(args["kind"], "vector");
        assert_eq!(args["name"], "phone.svg");
        assert_eq!(args["path"], json!({ "relative": "library/phone.svg" }));
        assert_eq!(args["info"]["video"]["width"], 412);
        assert_eq!(
            media_args(&bundle, &phone, &[]).unwrap(),
            args,
            "the same file"
        );
        // Another version of a drawing never overwrites the file an asset uses.
        std::fs::write(bundle.join("library/phone.svg"), "<svg/>").unwrap();
        assert_eq!(
            place(&bundle, "phone", ".svg", phone_svg().as_bytes()),
            format!(
                "library/phone-{}.svg",
                &blake3::hash(phone_svg().as_bytes()).to_hex().as_str()[..8]
            )
        );

        let shop = find("shop-app", None).unwrap();
        assert_eq!(shop.kind, "composition");
        assert_eq!(shop.lines[0], "your product");
        let first = media_args(&bundle, &shop, &[]).unwrap();
        assert_eq!(first["kind"], "web");
        assert_eq!(
            first["path"],
            json!({ "relative": "library/shop-app/index.html" })
        );
        assert_eq!(first["info"]["video"]["width"], 720);
        assert_eq!(first["info"]["video"]["height"], 1560);
        assert!(!bundle.join("web").exists(), "not copied into web/");
        // Other lines make another page beside it.
        let lamps = media_args(&bundle, &shop, &["desk lamps".into()]).unwrap();
        let relative = lamps["path"]["relative"].as_str().unwrap().to_string();
        assert!(relative.starts_with("library/shop-app-") && relative.ends_with("/index.html"));
        assert_ne!(lamps["identity"], first["identity"]);
        let page = std::fs::read_to_string(bundle.join(&relative)).unwrap();
        assert!(page.contains("\"desk lamps\",\"Product name\""));
        std::fs::remove_dir_all(&bundle).unwrap();
    }

    fn phone_svg() -> &'static str {
        fosscut_core::drawing("phone").unwrap().svg()
    }

    #[test]
    fn devices_report_where_their_screen_is() {
        let project = fosscut_model::Project::new(
            fosscut_model::Id::new(fosscut_model::IdKind::Project, 1),
            "Ad",
            fosscut_model::TimelineFormat {
                width: 1080,
                height: 1920,
                rate: fosscut_time::Rate::FPS_30,
                sample_rate: 48_000,
            },
        );
        let laptop = find("laptop", None).unwrap();
        let report = screen_report(&laptop, &project, None).unwrap();
        assert_eq!(report["canvas"], json!([1920, 1080]));
        // On a vertical canvas the laptop fits by its width.
        let fit = 1080.0 / 1600.0;
        let width = report["size"][0].as_f64().unwrap();
        assert!((width - 1248.0 * fit / 1080.0).abs() < 1e-4, "{report}");
        assert!(
            report["position"][1].as_f64().unwrap() < 0.0,
            "above the middle"
        );
        assert!(screen_report(&find("street-map", None).unwrap(), &project, None).is_none());
        assert!(screen_report(&laptop, &project, Some("timeline_0099")).is_none());
    }

    fn asset(number: u64, kind: fosscut_model::AssetKind, file: &Path) -> fosscut_model::Asset {
        fosscut_model::Asset {
            id: fosscut_model::Id::new(fosscut_model::IdKind::Asset, number),
            kind,
            name: file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: fosscut_model::PathHints {
                relative: None,
                absolute: Some(file.display().to_string()),
            },
            identity: None,
            info: fosscut_model::MediaInfo::default(),
            folder: None,
        }
    }

    fn code(error: EngineError) -> String {
        fosscut_core::ApiError::from(error).code
    }

    #[test]
    fn compositions_take_params_and_copy_their_slots_into_their_own_folder() {
        use fosscut_model::AssetKind;
        let dir =
            std::env::temp_dir().join(format!("fosscut-library-params-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = dir.join("Ad.fosscut");
        std::fs::create_dir_all(&bundle).unwrap();
        let logo = dir.join("logo.png");
        std::fs::write(&logo, b"picture bytes").unwrap();
        let tone = dir.join("tone.wav");
        std::fs::write(&tone, b"sound bytes").unwrap();
        let promo = dir.join("promo");
        std::fs::create_dir_all(promo.join("lib")).unwrap();
        std::fs::write(
            promo.join("index.html"),
            r#"<meta name="fosscut:composition" content="width=400,height=800,duration=3">"#,
        )
        .unwrap();
        std::fs::write(promo.join("lib/app.js"), "1").unwrap();
        let mut project = fosscut_model::Project::new(
            fosscut_model::Id::new(fosscut_model::IdKind::Project, 1),
            "Ad",
            fosscut_model::TimelineFormat {
                width: 1080,
                height: 1920,
                rate: fosscut_time::Rate::FPS_30,
                sample_rate: 48_000,
            },
        );
        let mut web = asset(4, AssetKind::Web, &promo.join("index.html"));
        web.info.duration = Some(fosscut_time::Flicks::from_seconds(3).unwrap());
        web.info.video = Some(fosscut_model::VideoInfo {
            width: 400,
            height: 800,
            rate: Some(fosscut_time::Rate::FPS_30),
            codec: Some("html".into()),
            rotation: 0,
        });
        project.media.assets.extend([
            asset(1, AssetKind::Image, &logo),
            asset(2, AssetKind::Audio, &tone),
            web,
        ]);
        let read = |relative: &Value| -> (std::path::PathBuf, Value) {
            let page = bundle.join(relative.as_str().unwrap());
            let folder = page.parent().unwrap().to_path_buf();
            let params =
                serde_json::from_slice(&std::fs::read(folder.join("params.json")).unwrap())
                    .unwrap();
            (folder, params)
        };

        let parcel = find("parcel-drop-3d", None).unwrap();
        assert!(parcel.params.iter().any(|p| p.name == "logo"));
        assert!(parcel.params.iter().any(|p| p.name == "motion_blur"));
        let plain = media_args_with(&bundle, &parcel, &CompositionInput::default(), None).unwrap();
        assert_eq!(plain["kind"], "web");
        assert_eq!(
            plain["path"]["relative"],
            "library/parcel-drop-3d/index.html"
        );
        assert_eq!(plain["info"]["video"]["width"], 1080);
        let (folder, params) = read(&plain["path"]["relative"]);
        assert_eq!(params["logo"], Value::Null);
        assert_eq!(params["box_color"], "#F07A2B");
        assert_eq!(
            params["motion_blur"],
            json!({ "samples": 16, "shutter": 0.5 }),
            "the fast drop takes more samples"
        );
        for file in [
            "scene.js",
            "kit/stage.js",
            "vendor/three/LICENSE",
            "vendor/three/three.module.min.js",
        ] {
            assert!(folder.join(file).is_file(), "{file}");
        }
        assert_eq!(
            plain["identity"],
            json!(crate::web::folder_identity(&folder).unwrap()),
            "the asset's identity is the folder's"
        );
        // The same parameters give the same folder and asset.
        assert_eq!(
            media_args_with(&bundle, &parcel, &CompositionInput::default(), None).unwrap(),
            plain
        );

        // Other parameters and a slot make another folder, with the slot's file in it.
        let input = CompositionInput {
            params: Some(json!({ "box_color": "#3366ff", "width": 540, "height": 960 })),
            media: vec!["asset_0001".into()],
            ..CompositionInput::default()
        };
        let custom = media_args_with(&bundle, &parcel, &input, Some(&project)).unwrap();
        assert_ne!(custom["identity"], plain["identity"]);
        let relative = custom["path"]["relative"].as_str().unwrap().to_string();
        assert!(
            relative.starts_with("library/parcel-drop-3d-") && relative.ends_with("/index.html")
        );
        assert_eq!(
            custom["info"]["video"]["width"], 540,
            "the canvas follows width and height"
        );
        assert_eq!(custom["info"]["video"]["height"], 960);
        let (folder, params) = read(&custom["path"]["relative"]);
        assert_eq!(params["box_color"], "#3366FF");
        assert_eq!(params["width"], 540);
        assert_eq!(
            params["logo"],
            json!({ "src": "media/logo.png", "kind": "image", "name": "logo.png" })
        );
        assert_eq!(
            std::fs::read(folder.join("media/logo.png")).unwrap(),
            b"picture bytes"
        );
        let again = media_args_with(&bundle, &parcel, &input, Some(&project)).unwrap();
        assert_eq!(again, custom);

        // Slots refuse other kinds of media, unknown assets and a missing project.
        let with = |params: Value| CompositionInput {
            params: Some(params),
            ..CompositionInput::default()
        };
        let audio = media_args_with(
            &bundle,
            &parcel,
            &with(json!({ "logo": "asset_0002" })),
            Some(&project),
        );
        let message = format!("{:?}", audio.as_ref().unwrap_err());
        assert!(message.contains("takes image or svg"), "{message}");
        assert_eq!(code(audio.unwrap_err()), "fosscut.library.invalid_params");
        let missing = media_args_with(
            &bundle,
            &parcel,
            &with(json!({ "logo": "asset_0099" })),
            Some(&project),
        );
        assert_eq!(code(missing.unwrap_err()), "fosscut.library.invalid_params");
        assert!(
            media_args_with(
                &bundle,
                &parcel,
                &with(json!({ "logo": "asset_0001" })),
                None
            )
            .is_err()
        );
        let wrong = media_args_with(&bundle, &parcel, &with(json!({ "sparks": 1.5 })), None);
        assert_eq!(code(wrong.unwrap_err()), "fosscut.library.invalid_params");

        // The phone's screen is the shop-app composition by default, copied in whole; a web
        // asset fills it with its folder.
        let phone = find("phone-3d", None).unwrap();
        let shown = media_args_with(&bundle, &phone, &CompositionInput::default(), None).unwrap();
        let (folder, params) = read(&shown["path"]["relative"]);
        assert_eq!(
            params["screen"],
            json!({ "src": "media/screen/index.html", "kind": "composition", "name": "shop-app",
                    "width": 720, "height": 1560, "fps": 30.0, "duration": 9.0 })
        );
        assert_eq!(
            std::fs::read_to_string(folder.join("media/screen/index.html")).unwrap(),
            fosscut_core::composition("shop-app").unwrap().html(&[])
        );
        let own = media_args_with(
            &bundle,
            &phone,
            &with(json!({ "screen": "asset_0004" })),
            Some(&project),
        )
        .unwrap();
        let (folder, params) = read(&own["path"]["relative"]);
        assert_eq!(params["screen"]["src"], "media/screen/index.html");
        assert_eq!(params["screen"]["width"], 400);
        assert_eq!(params["screen"]["duration"], 3.0);
        assert!(folder.join("media/screen/lib/app.js").is_file());

        // Only compositions take parameters.
        let sound = &search(Some("sound"), None)[0];
        assert!(media_args_with(&bundle, sound, &with(json!({})), None).is_err());

        // A folder an asset uses is never rewritten: a changed one is left alone and the
        // composition goes beside it under its content's hash.
        std::fs::write(bundle.join("library/parcel-drop-3d/params.json"), "{}").unwrap();
        let moved = media_args_with(&bundle, &parcel, &CompositionInput::default(), None).unwrap();
        assert_eq!(moved["identity"], plain["identity"]);
        assert_ne!(moved["path"], plain["path"]);
        assert_eq!(
            std::fs::read_to_string(bundle.join("library/parcel-drop-3d/params.json")).unwrap(),
            "{}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
