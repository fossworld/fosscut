//! The engine's operations as `capabilities` lists them (ADR-0002, item 6): work that reads
//! a project and produces files or images, beside the core's commands and queries.

use serde::{Deserialize, Serialize};

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
}
