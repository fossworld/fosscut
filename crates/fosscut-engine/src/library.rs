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
    match item.kind {
        "sound" | "music" => sound_media(bundle, &item.name),
        "device" | "drawing" => drawing_media(bundle, &item.name),
        "composition" => composition_media(bundle, &item.name, lines),
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
    static SCRATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let composition = fosscut_core::composition(name).map_err(failed)?;
    let html = composition.html(lines);
    let relative = place(bundle, name, "/index.html", html.as_bytes());
    let page = bundle.join(&relative);
    write_once(&page, html.as_bytes())?;
    let folder = page.parent().unwrap_or(bundle);
    // Measured against a scratch bundle, which takes the copy media_add_args makes.
    let scratch = std::env::temp_dir().join(format!(
        "fosscut-library-{}-{}",
        std::process::id(),
        SCRATCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let measured = crate::web::media_add_args(
        folder,
        Some(name),
        &scratch,
        &crate::web::CompositionOptions::default(),
    );
    let _ = std::fs::remove_dir_all(&scratch);
    let mut args = measured.map_err(failed)?;
    args["path"] = json!({ "relative": relative });
    Ok(args)
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
        assert_eq!(shop.lines[0], "running shoes");
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
        assert!(page.contains("\"desk lamps\",\"Trail Runner\""));
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
}
