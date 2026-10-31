//! The engine's operations as `capabilities` lists them (ADR-0002, item 6): work that reads
//! a project and produces files or images, beside the core's commands and queries.

use std::path::Path;

use fosscut_core::{ApiError, Session};
use fosscut_model::{Id, IdKind, MemoryStore, Project, TimelineFormat};
use fosscut_time::Rate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{engine_error_catalogue, presets, project};

/// One engine operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationInfo {
    /// Stable name, for example `frame.grab`.
    pub name: String,
    /// What it does, for an agent deciding whether to use it.
    pub summary: String,
    /// How to run it from the command line.
    pub cli: String,
}

/// Every engine operation.
pub fn operations() -> Vec<OperationInfo> {
    [
        (
            "export.start",
            "Render a timeline to a video or audio file with a preset, then verify the file by probing it.",
            "fosscut export <file> [--preset NAME] [--timeline ID]",
        ),
        (
            "media.analyze",
            "Measure an asset's loudness, silences and shot cuts, or transcribe its speech with word timings, on this machine; results are cached by content.",
            "fosscut media analyze <asset> [loudness|silences|scenes|transcript ...] [--refresh]",
        ),
        (
            "captions.generate",
            "Caption the clips of a track word by word from their cached transcripts, as pages on a caption track with a preset look; one undoable step.",
            "fosscut captions generate [--preset NAME] [--source-track ID] [--replace]",
        ),
        (
            "captions.import",
            "Add caption pages from an SRT subtitle file, one page per cue with its lines kept.",
            "fosscut captions import <file.srt> [--preset NAME] [--replace]",
        ),
        (
            "captions.export",
            "Write a caption track to an SRT subtitle file, one cue per page.",
            "fosscut captions export <file.srt> [--track ID]",
        ),
        (
            "color.auto",
            "Measure a video or image clip's picture and add a color.adjust effect that corrects its white balance, brightness and levels; one undoable step.",
            "fosscut color auto --clip ID",
        ),
        (
            "color.match",
            "Measure two clips and add a color.adjust effect to the first that brings its balance, brightness, contrast and saturation close to the second; one undoable step.",
            "fosscut color match --clip ID --reference ID",
        ),
        (
            "audio.normalize",
            "Measure clips' loudness as they play (ITU-R BS.1770) and set their volumes to reach a target with peaks under a ceiling, adding an audio.limiter where needed; one undoable step.",
            "fosscut audio normalize --clip ID [--clip ID ...] [--target LUFS] [--ceiling DBFS]",
        ),
        (
            "library.add",
            "Add an item of FossCut's own library: a sound or music loop (written into the bundle and imported), a shape, sticker, background, title or project template, a text or caption style, or a look; one undoable step.",
            "fosscut library add <name> [--at TIME] [--clip ID] [--line TEXT] [--media ID]",
        ),
        (
            "clip.split_scenes",
            "Measure the shot cuts in a video clip's media (cached by content) and split the clip at them, so each shot is a clip of its own; one undoable step.",
            "fosscut clip split-scenes --clip ID",
        ),
        (
            "clip.stabilize",
            "Measure how the camera moved in a video clip and key its position, rotation and scale to cancel the shake, with one zoom up to 1.5 that hides the moving edges; one undoable step.",
            "fosscut clip stabilize --clip ID [--strength 0..3]",
        ),
        (
            "clip.track",
            "Follow a region of a video clip's picture from a time on and key one of its masks, or another clip such as a title or sticker, to move with it; one undoable step.",
            "fosscut clip track --clip ID --region X,Y,W,H [--at TIME] (--mask ID | --follow ID)",
        ),
        (
            "multicam.sync",
            "Measure how far apart angles recorded at the same time started, from their sound, as the offsets of multicam.create (which multicam create measures itself unless given offsets).",
            "fosscut multicam sync <asset> <asset> ... [--max-offset SECONDS]",
        ),
        (
            "record.start",
            "Record a voiceover (48 kHz WAV), a camera or the screen into the bundle's recordings folder through the same encoders and checks as exports, then import it; stops after a duration or when asked, keeping what was recorded.",
            "fosscut record microphone|camera|screen [--device NAME] [--microphone NAME] [--duration TIME] [--at TIME] [--list]",
        ),
        (
            "preferences.edit",
            "Read or change the user's preferences: editing and timeline defaults the CLI and the app fill requests with, performance and cache settings (ADR-0031); agents read them with the query preferences.read.",
            "fosscut preferences [get KEY | set KEY VALUE | reset KEY | path]",
        ),
        (
            "media.proxy",
            "Make a smaller ProRes Proxy copy of a video asset in FossCut's cache for the desktop preview; exports, frame grabs and contact sheets always read the original.",
            "fosscut media proxy <asset> [--size half|quarter]",
        ),
        (
            "media.remux",
            "Copy a video asset in a hard-to-seek container (MPEG-TS, MPEG-PS, AVI, FLV) into QuickTime in FossCut's cache, without re-encoding, for the desktop preview.",
            "fosscut media remux <asset>",
        ),
        (
            "cache.clear",
            "Report the size of FossCut's caches (analysis, proxies, remuxed copies, rendered frames), empty them, or clean them by the cache preferences; everything in them is made again when needed.",
            "fosscut cache info | clear [KIND ...] | clean",
        ),
        (
            "speech.speak",
            "Say text with a local English voice (Kokoro) into the bundle's speech folder, import it and place it; the same text, voice and speed reuse the file.",
            "fosscut speak \"TEXT\" [--voice NAME] [--speed N] [--at TIME] | --voices",
        ),
        (
            "captions.translate",
            "Translate a caption track's pages between English and Hungarian with a local model, as a second line under each page or in place of its words; one undoable step.",
            "fosscut captions translate --to hu|en [--mode bilingual|replace] [--track ID]",
        ),
        (
            "captions.dub",
            "Speak each caption page (its words or its translation) with a local English voice, one clip per page at its start on an audio track; one undoable step.",
            "fosscut captions dub [--voice NAME] [--speed N] [--translation] [--track ID]",
        ),
        (
            "update.check",
            "Ask GitHub once for the latest FossCut release and say whether it is newer; nothing about the user is sent and nothing is installed.",
            "fosscut update",
        ),
        (
            "models.install",
            "Download a local model (speech for transcripts; matte, pose, voice and translate; and the ONNX Runtime they run on) after the user agrees to its size and license; every file is verified by SHA-256.",
            "fosscut models install <name> --yes [--from <file>]",
        ),
        (
            "frame.grab",
            "Render one frame of a timeline as a PNG image, exactly as an export shows it.",
            "fosscut frame <time> --out <file.png> [--width N] [--timeline ID]",
        ),
        (
            "frame.contact_sheet",
            "Render frames spread evenly over a timeline into one PNG grid, to check an edit at a glance.",
            "fosscut contact-sheet --out <file.png> [--count N] [--width N] [--timeline ID]",
        ),
    ]
    .into_iter()
    .map(|(name, summary, cli)| OperationInfo {
        name: name.into(),
        summary: summary.into(),
        cli: cli.into(),
    })
    .collect()
}

/// Everything this editor can do (ADR-0002, item 6): the core's commands and queries at
/// `detail` (`outline`, `normal` or `full`), the engine's operations, the analyzers, the
/// models with whether each is installed in [`crate::models_dir`], and the export presets
/// with whether this machine can encode them. At full detail, `errors` lists the core's and
/// the engine's error codes. With `names`, only those commands and queries, each with its
/// schemas: a small answer an agent can read before calling a command.
///
/// With a `bundle` that opens, the core reports for that project; otherwise for a default
/// 1080p30 project, so capabilities work before any project exists.
pub fn capabilities(
    detail: &str,
    names: &[String],
    bundle: Option<&Path>,
) -> Result<Value, ApiError> {
    let args = json!({ "detail": detail, "names": names });
    let mut value = match bundle.map(|bundle| project::query(bundle, "capabilities", args.clone()))
    {
        Some(Ok(value)) => value,
        _ => scratch_session()?.query("capabilities", args)?,
    };
    if !names.is_empty() {
        return Ok(value);
    }
    value["operations"] = json!(operations());
    value["analyzers"] = json!(fosscut_analysis::analyzers());
    value["models"] = json!(fosscut_analysis::models::list(&crate::models_dir()));
    value["export_presets"] = json!(presets());
    if let Some(errors) = value.get_mut("errors").and_then(Value::as_array_mut) {
        errors.extend(
            engine_error_catalogue()
                .into_iter()
                .map(|e| serde_json::to_value(e).unwrap_or_default()),
        );
    }
    Ok(value)
}

/// A throwaway in-memory session, so capabilities work without a project.
fn scratch_session() -> Result<Session<MemoryStore>, ApiError> {
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let id = Id::new(IdKind::Project, 0);
    Session::create(MemoryStore::new(), Project::new(id, "capabilities", format))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_dotted() {
        let all = operations();
        let mut names: Vec<&str> = all.iter().map(|o| o.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
        assert!(
            all.iter()
                .all(|o| o.name.contains('.') && o.cli.starts_with("fosscut "))
        );
    }

    #[test]
    fn capabilities_combine_core_and_engine() {
        let outline = capabilities("outline", &[], None).unwrap();
        assert!(outline["commands"].as_array().unwrap().len() > 3);
        assert_eq!(
            outline["operations"].as_array().unwrap().len(),
            operations().len()
        );
        assert!(outline["export_presets"].is_array());
        assert_eq!(outline["analyzers"][1]["name"], "silences");
        assert_eq!(outline["models"][0]["name"], "whisper-large-v3-turbo");
        assert!(outline["models"][0]["installed"].is_boolean());
        assert!(outline.get("errors").is_none());

        let full = capabilities("full", &[], Some(Path::new("/no/such/Show.fosscut"))).unwrap();
        let codes: Vec<&str> = full["errors"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["code"].as_str())
            .collect();
        assert!(codes.contains(&"fosscut.revision.conflict"));
        assert!(codes.contains(&"fosscut.time.out_of_range"));
        assert!(full["commands"][0]["input_schema"].is_object());
        assert_eq!(
            capabilities("loud", &[], None).unwrap_err().code,
            "fosscut.request.invalid_arguments"
        );
        let named = capabilities("outline", &["text.add".to_string()], None).unwrap();
        assert_eq!(named["commands"].as_array().unwrap().len(), 1);
        assert!(named["commands"][0]["input_schema"].is_object());
        assert!(named.get("operations").is_none() && named.get("export_presets").is_none());
        assert!(named.get("analyzers").is_none() && named.get("models").is_none());
    }
}
