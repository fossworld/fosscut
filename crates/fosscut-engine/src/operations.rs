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
            "models.install",
            "Download a model an analyzer needs (speech for transcript) after the user agrees to its size and license; it is verified by SHA-256.",
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
    let mut value = match bundle.map(project::open) {
        Some(Ok(session)) => session.query("capabilities", args)?,
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
