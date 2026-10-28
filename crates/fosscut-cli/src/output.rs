//! Human-readable output. With `--json` the raw JSON is printed instead.

use fosscut_core::{ApiError, Response};
use serde_json::Value;

fn s(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

fn range(placement: &Value) -> String {
    format!(
        "{} – {} ({} frames)",
        s(&placement["start"]["timecode"]),
        s(&placement["end"]["timecode"]),
        placement["duration"]["frames"]
    )
}

fn describe(result: &Value) -> String {
    if result.get("left").is_some() {
        return format!(
            "split: {} {}  |  {} {}",
            s(&result["left"]["clip"]),
            range(&result["left"]),
            s(&result["right"]["clip"]),
            range(&result["right"])
        );
    }
    if result.get("clip").is_some() && result.get("start").is_some() {
        return format!(
            "{} on {}: {}",
            s(&result["clip"]),
            s(&result["track"]),
            range(result)
        );
    }
    if result.get("removed").is_some() {
        return format!(
            "removed {} from {}",
            s(&result["removed"]),
            s(&result["track"])
        );
    }
    if let Some(asset) = result.get("asset") {
        let note = if result["existing"] == Value::Bool(true) {
            " (already imported)"
        } else {
            ""
        };
        return format!("{}{note}", s(asset));
    }
    if result.get("track").is_some() && result.get("index").is_some() {
        return format!("{} at index {}", s(&result["track"]), result["index"]);
    }
    if let Some(seq) = result.get("target_seq") {
        return format!("history entry {seq}");
    }
    result.to_string()
}

/// Prints a write response.
pub fn print_response(response: &Response) {
    let status = if response.replayed {
        format!("replayed earlier request, revision {}", response.revision)
    } else if response.dry_run {
        format!("dry run, nothing saved (revision {})", response.revision)
    } else if response.committed {
        format!("saved revision {}", response.revision)
    } else {
        format!("no changes (revision {})", response.revision)
    };
    println!("{status}");
    for result in &response.results {
        println!("  {}", describe(result));
    }
    let c = &response.changes;
    if !c.is_empty() {
        let list = |label: &str, ids: &[String]| {
            if ids.is_empty() {
                String::new()
            } else {
                format!(" {label} {}", ids.join(", "))
            }
        };
        println!(
            "  changes:{}{}{}",
            list("+", &c.added),
            list("~", &c.modified),
            list("-", &c.removed)
        );
    }
}

/// Prints a query result.
pub fn print_query(name: &str, value: &Value) {
    match name {
        "timeline.read" => print_timeline(value),
        "project.info" => print_info(value),
        "media.list" => print_media(value),
        "history.list" => print_history(value),
        "capabilities" => print_capabilities(value),
        _ => println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        ),
    }
}

fn print_timeline(v: &Value) {
    println!(
        "{} ({})  {}x{} @ {} fps  revision {}  duration {}",
        s(&v["name"]),
        s(&v["timeline"]),
        v["format"]["width"],
        v["format"]["height"],
        s(&v["format"]["rate"]),
        v["revision"],
        s(&v["duration"]["timecode"])
    );
    for track in v["tracks"].as_array().into_iter().flatten() {
        println!(
            "{} {}  ({} clips, ends {})",
            s(&track["id"]),
            s(&track["kind"]),
            track["clip_count"],
            s(&track["end"]["timecode"])
        );
        for clip in track["clips"].as_array().into_iter().flatten() {
            let what = match s(&clip["kind"]) {
                "text" => format!("\"{}\"", s(&clip["text"])),
                _ => format!(
                    "{} from {}",
                    s(&clip["asset_name"]),
                    s(&clip["source_in"]["timecode"])
                ),
            };
            println!(
                "  {} {:<5} {}  {}",
                s(&clip["id"]),
                s(&clip["kind"]),
                range(clip),
                what
            );
        }
    }
}

fn print_info(v: &Value) {
    println!(
        "{} ({})  revision {}  format {}",
        s(&v["name"]),
        s(&v["id"]),
        v["revision"],
        s(&v["format_version"])
    );
    println!(
        "assets: {}  can undo: {}  can redo: {}",
        v["assets"], v["can_undo"], v["can_redo"]
    );
    for t in v["timelines"].as_array().into_iter().flatten() {
        let primary = if t["id"] == v["primary_timeline"] {
            " (primary)"
        } else {
            ""
        };
        println!(
            "  {} {}{primary}: {}x{} @ {} fps, {} tracks, {} clips, {}",
            s(&t["id"]),
            s(&t["name"]),
            t["format"]["width"],
            t["format"]["height"],
            s(&t["format"]["rate"]),
            t["tracks"],
            t["clips"],
            s(&t["duration"]["timecode"])
        );
    }
}

fn print_media(v: &Value) {
    for a in v["assets"].as_array().into_iter().flatten() {
        let duration = a["duration"]["timecode"].as_str().unwrap_or("still");
        let size = if a["video"].is_object() {
            format!(" {}x{}", a["video"]["width"], a["video"]["height"])
        } else {
            String::new()
        };
        println!(
            "{} {:<5} {}{size}  {}",
            s(&a["id"]),
            s(&a["kind"]),
            duration,
            s(&a["name"])
        );
    }
}

fn print_history(v: &Value) {
    for e in v["entries"].as_array().into_iter().flatten() {
        let commands: Vec<&str> = e["commands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let undone = if e["undone"] == Value::Bool(true) {
            "  (undone)"
        } else {
            ""
        };
        println!(
            "#{} r{}→r{} {} by {}: {}{undone}",
            e["seq"],
            e["revision_before"],
            e["revision_after"],
            s(&e["kind"]),
            s(&e["actor"]),
            commands.join(", ")
        );
    }
}

fn print_capabilities(v: &Value) {
    println!(
        "API {}  project format {}",
        s(&v["api_version"]),
        s(&v["format_version"])
    );
    println!("commands:");
    for c in v["commands"].as_array().into_iter().flatten() {
        println!("  {:<14} {}", s(&c["name"]), s(&c["summary"]));
    }
    println!("queries:");
    for q in v["queries"].as_array().into_iter().flatten() {
        println!("  {:<14} {}", s(&q["name"]), s(&q["summary"]));
    }
}

/// Prints an error to stderr, or as `{"error": ...}` JSON on stdout.
pub fn print_error(error: &ApiError, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "error": error }))
                .unwrap_or_default()
        );
    } else {
        eprintln!("error: {} [{}]", error.message, error.code);
        if let Some(hint) = &error.hint {
            eprintln!("hint: {hint}");
        }
        if !error.details.is_null() {
            eprintln!("details: {}", error.details);
        }
    }
}
