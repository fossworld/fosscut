//! FossCut's user-facing skills (ADR-0013), built into the binary, so `fosscut skills
//! install` and the MCP server's prompts and resources serve exactly the text in the
//! repository's `skills/` folder.

/// A user-facing skill: its folder name and the files an agent needs (its evaluation stays
/// in the repository).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Skill {
    /// Folder and frontmatter name, for example `fosscut-basics`.
    pub name: &'static str,
    /// Paths inside the skill folder with their contents; `SKILL.md` comes first.
    pub files: &'static [(&'static str, &'static str)],
}

/// Every built-in skill.
pub const SKILLS: &[Skill] = &[Skill {
    name: "fosscut-basics",
    files: &[(
        "SKILL.md",
        include_str!("../../../skills/fosscut-basics/SKILL.md"),
    )],
}];

impl Skill {
    /// The `SKILL.md` text.
    pub fn text(&self) -> &'static str {
        self.files
            .iter()
            .find(|(path, _)| *path == "SKILL.md")
            .map(|(_, text)| *text)
            .unwrap_or_default()
    }

    /// The frontmatter description: what the skill does and when to use it.
    pub fn description(&self) -> &'static str {
        let mut lines = self.text().lines();
        if lines.next() != Some("---") {
            return "";
        }
        lines
            .take_while(|line| *line != "---")
            .find_map(|line| line.strip_prefix("description: "))
            .unwrap_or_default()
    }

    /// The MCP resource address of its `SKILL.md`.
    pub fn uri(&self) -> String {
        format!("fosscut://skills/{}", self.name)
    }
}

/// The built-in skill called `name`.
pub fn skill(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|skill| skill.name == name)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// Every folder in `skills/` is built in with all its files except the evaluation,
    /// so a new skill cannot be forgotten.
    #[test]
    fn every_skill_folder_is_built_in() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        let mut folders: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        folders.sort();
        let mut built_in: Vec<&str> = SKILLS.iter().map(|s| s.name).collect();
        built_in.sort_unstable();
        assert_eq!(folders, built_in);
        for skill in SKILLS {
            let folder = root.join(skill.name);
            let mut on_disk = Vec::new();
            let mut pending = vec![folder.clone()];
            while let Some(dir) = pending.pop() {
                for entry in std::fs::read_dir(&dir).unwrap().filter_map(Result::ok) {
                    let path = entry.path();
                    if path.is_dir() {
                        if path.file_name().is_some_and(|n| n != "eval") {
                            pending.push(path);
                        }
                    } else {
                        let relative = path.strip_prefix(&folder).unwrap();
                        let relative = relative.to_string_lossy().replace('\\', "/");
                        on_disk.push((relative, std::fs::read_to_string(&path).unwrap()));
                    }
                }
            }
            on_disk.sort();
            let mut embedded: Vec<(String, String)> = skill
                .files
                .iter()
                .map(|(p, t)| (p.to_string(), t.to_string()))
                .collect();
            embedded.sort();
            assert_eq!(
                on_disk, embedded,
                "{} is built in as it is on disk",
                skill.name
            );
            assert_eq!(skill.files[0].0, "SKILL.md");
        }
    }

    #[test]
    fn skills_describe_themselves() {
        let basics = skill("fosscut-basics").unwrap();
        assert!(
            basics
                .description()
                .starts_with("Edit video projects with FossCut")
        );
        assert!(basics.text().contains("## The safe editing loop"));
        assert_eq!(basics.uri(), "fosscut://skills/fosscut-basics");
        assert!(skill("fosscut-magic").is_none());
    }
}
