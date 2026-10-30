//! The built-in library (ADR-0028): shapes, stickers, backgrounds, project and title
//! templates, text and caption presets, looks, and sound effects and music loops that
//! FossCut generates itself, so everything is free to use, also commercially, and nothing
//! is downloaded. Sounds are written into the bundle's `library/` folder the first time
//! they are used and then imported like any audio file.

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
    /// `caption_style`, `look`, `sound` or `music`.
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

/// Where to put an item, for [`command_for`].
#[derive(Clone, Debug, Default)]
pub struct Placement {
    /// When it starts, as a time input.
    pub start: Option<Value>,
    /// How long it lasts, as a time input.
    pub duration: Option<Value>,
    /// The track.
    pub track: Option<String>,
    /// The clip a look applies to.
    pub clip: Option<String>,
    /// Lines for titles and templates; the text of a text style's clip.
    pub lines: Vec<String>,
    /// Assets for a project template's slots.
    pub media: Vec<String>,
}

/// The command that adds `item`, except sounds and music, which are imported first with
/// [`sound_media`] and then placed with `clip.add`.
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
            "text.add"
        }
        "caption_style" => {
            put("preset", Some(json!(item.name)));
            put("track", at.track.clone().map(Value::from));
            "captions.style"
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
                    "{} is a {}: import it with sound_media, then place it with clip.add",
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
}
