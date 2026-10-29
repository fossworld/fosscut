//! User-facing skills (ADR-0013): every folder in `skills/` is well formed and passes its
//! evaluation, replayed through `fosscut mcp` as an agent would call the tools.
//!
//! An evaluation is `skills/<name>/eval/eval.json`:
//!
//! - `fixtures`: files generated with FFmpeg (`file`, and `ffmpeg` arguments before it);
//! - `steps`: tool calls in order. Each has `tool` and `args`, and optionally:
//!   - `expect`: values at dotted paths in the result (`results.0.clip`), compared for
//!     equality or with a matcher: `{"$len": n}`, `{"$min_len": n}`, `{"$one_of": [...]}`;
//!   - `expect_error`: the error code the call must fail with (`expect` then checks the
//!     error object);
//!   - `expect_image`: the `width` and `height` of the PNG image the call returns first;
//!   - `save`: names for values at dotted paths, used later as `"${name}"`;
//!   - `repeat_while`: call again (up to 20 times) while these paths hold these values.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::mcp::{Client, call, json_of, png_size, start};
use common::{can_render, workspace};
use serde_json::Value;

fn skills_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills")
}

/// Every skill folder, sorted.
fn skills() -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(skills_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    found.sort();
    found
}

/// The value at a dotted path: object keys and array indices.
fn at<'a>(value: &'a Value, path: &str) -> &'a Value {
    path.split('.')
        .fold(value, |value, key| match key.parse::<usize>() {
            Ok(index) if value.is_array() => &value[index],
            _ => &value[key],
        })
}

/// Replaces every string `"${name}"` in `value` with the saved value of that name.
fn substitute(value: &Value, saved: &HashMap<String, Value>) -> Value {
    match value {
        Value::String(text) => match text.strip_prefix("${").and_then(|t| t.strip_suffix('}')) {
            Some(name) => saved
                .get(name)
                .cloned()
                .unwrap_or_else(|| panic!("${{{name}}} is used before it is saved")),
            None => value.clone(),
        },
        Value::Array(items) => Value::Array(items.iter().map(|v| substitute(v, saved)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), substitute(v, saved)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Whether `actual` satisfies `expected`: equality, or a `$` matcher.
fn satisfies(actual: &Value, expected: &Value) -> bool {
    let matcher = expected
        .as_object()
        .filter(|m| m.len() == 1)
        .and_then(|m| m.iter().next())
        .filter(|(k, _)| k.starts_with('$'));
    let length = actual.as_array().map(Vec::len);
    match matcher {
        Some((name, arg)) => match name.as_str() {
            "$len" => length.is_some_and(|n| Some(n as u64) == arg.as_u64()),
            "$min_len" => length.is_some_and(|n| arg.as_u64().is_some_and(|min| n as u64 >= min)),
            "$one_of" => arg
                .as_array()
                .is_some_and(|options| options.contains(actual)),
            other => panic!("unknown matcher {other}"),
        },
        None => actual == expected,
    }
}

fn check(label: &str, result: &Value, expect: Option<&Value>, saved: &HashMap<String, Value>) {
    for (path, expected) in expect.and_then(Value::as_object).into_iter().flatten() {
        let expected = substitute(expected, saved);
        let actual = at(result, path);
        assert!(
            satisfies(actual, &expected),
            "{label}: {path} is {actual}, expected {expected}\n{result:#}"
        );
    }
}

async fn run(skill: &str, dir: &Path, eval: &Value) {
    for fixture in eval["fixtures"].as_array().into_iter().flatten() {
        let args: Vec<&str> = fixture["ffmpeg"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        let status = Command::new("ffmpeg")
            .current_dir(dir)
            .args(["-v", "error", "-y"])
            .args(&args)
            .arg(fixture["file"].as_str().unwrap())
            .status()
            .unwrap();
        assert!(status.success(), "{skill}: fixture {}", fixture["file"]);
    }
    let client: Client = start(dir, "fosscut-eval").await;
    let mut saved: HashMap<String, Value> = HashMap::new();
    for (index, step) in eval["steps"].as_array().unwrap().iter().enumerate() {
        let tool = step["tool"].as_str().unwrap();
        let label = format!(
            "{skill} step {index} ({tool}): {}",
            step["about"].as_str().unwrap_or("")
        );
        let args = substitute(&step["args"], &saved);
        let mut result = call(&client, tool, args.clone()).await;
        if let Some(condition) = step.get("repeat_while").and_then(Value::as_object) {
            for _ in 0..20 {
                let json = json_of(&result);
                if !condition
                    .iter()
                    .all(|(path, value)| at(&json, path) == value)
                {
                    break;
                }
                result = call(&client, tool, args.clone()).await;
            }
        }
        let json = json_of(&result);
        match step.get("expect_error").and_then(Value::as_str) {
            Some(code) => {
                assert_eq!(
                    result.is_error,
                    Some(true),
                    "{label}: should fail\n{json:#}"
                );
                assert_eq!(json["error"]["code"], code, "{label}\n{json:#}");
                check(&label, &json["error"], step.get("expect"), &saved);
            }
            None => {
                assert_ne!(result.is_error, Some(true), "{label}: failed\n{json:#}");
                check(&label, &json, step.get("expect"), &saved);
                if let Some(image) = step.get("expect_image") {
                    let size = png_size(&result);
                    assert_eq!(
                        (u64::from(size.0), u64::from(size.1)),
                        (
                            image["width"].as_u64().unwrap(),
                            image["height"].as_u64().unwrap()
                        ),
                        "{label}: image size"
                    );
                }
            }
        }
        for (name, path) in step
            .get("save")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let value = at(&json, path.as_str().unwrap());
            assert!(
                !value.is_null(),
                "{label}: nothing at {path} to save as {name}"
            );
            saved.insert(name.clone(), value.clone());
        }
    }
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn every_skill_is_well_formed() {
    let skills = skills();
    assert!(!skills.is_empty(), "skills/ has no skills");
    let dir = workspace("skills-form");
    let client = start(&dir, "fosscut-eval").await;
    let tools: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for (name, path) in &skills {
        // SKILL.md: frontmatter whose name is the folder, and a description agents can match.
        let text = std::fs::read_to_string(path.join("SKILL.md")).unwrap();
        let front = text
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map(|(front, _)| front)
            .unwrap_or_else(|| panic!("{name}: SKILL.md starts with --- frontmatter"));
        let field = |key: &str| {
            front
                .lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix(": "))
                .unwrap_or_else(|| panic!("{name}: frontmatter has no {key}"))
        };
        assert_eq!(field("name"), name, "the skill's name is its folder");
        let description = field("description");
        assert!(
            (40..=1024).contains(&description.len()),
            "{name}: the description says what the skill does and when to use it"
        );
        // Every skill has an evaluation (ADR-0013, item 4) that calls only real tools.
        let eval: Value =
            serde_json::from_str(&std::fs::read_to_string(path.join("eval/eval.json")).unwrap())
                .unwrap_or_else(|e| panic!("{name}: eval/eval.json: {e}"));
        assert_eq!(eval["skill"], name.as_str());
        let steps = eval["steps"].as_array().unwrap();
        assert!(!steps.is_empty());
        for step in steps {
            let tool = step["tool"].as_str().unwrap();
            assert!(tools.iter().any(|t| t == tool), "{name}: no tool {tool}");
        }
        // Tools named in the instructions exist.
        for tool in text.split('`').skip(1).step_by(2) {
            if tool.contains('_') && tool.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                let known = tools.iter().any(|t| t == tool)
                    || [
                        "expected_revision",
                        "request_id",
                        "allow_destructive",
                        "dry_run",
                        "wait_seconds",
                        "clip_count",
                    ]
                    .contains(&tool);
                assert!(
                    known,
                    "{name}: SKILL.md names `{tool}`, which is not a tool or argument"
                );
            }
        }
    }
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn every_skill_passes_its_evaluation() {
    if !can_render() {
        return;
    }
    for (name, path) in skills() {
        let eval: Value =
            serde_json::from_str(&std::fs::read_to_string(path.join("eval/eval.json")).unwrap())
                .unwrap();
        let dir = workspace(&format!("skill-{name}"));
        run(&name, &dir, &eval).await;
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn paths_substitutions_and_matchers_work() {
    let result: Value =
        serde_json::json!({ "results": [{ "clip": "clip_0003", "n": 3 }], "tiles": [1, 2] });
    assert_eq!(at(&result, "results.0.clip"), "clip_0003");
    assert!(at(&result, "results.1.clip").is_null());
    let saved = HashMap::from([("rev".to_string(), serde_json::json!(4))]);
    let args = substitute(
        &serde_json::json!({ "expected_revision": "${rev}", "keep": "$rev" }),
        &saved,
    );
    assert_eq!(
        args,
        serde_json::json!({ "expected_revision": 4, "keep": "$rev" })
    );
    assert!(satisfies(
        &result["tiles"],
        &serde_json::json!({ "$len": 2 })
    ));
    assert!(!satisfies(
        &result["tiles"],
        &serde_json::json!({ "$min_len": 3 })
    ));
    assert!(satisfies(
        &serde_json::json!("av1"),
        &serde_json::json!({ "$one_of": ["h264", "av1"] })
    ));
    assert!(!satisfies(
        &serde_json::json!("vp9"),
        &serde_json::json!({ "$one_of": ["h264", "av1"] })
    ));
    assert!(satisfies(
        &serde_json::json!({ "a": 1 }),
        &serde_json::json!({ "a": 1 })
    ));
}
