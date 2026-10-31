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
        "export.presets" => print_presets(value),
        "frame.grab" => print_frame(value),
        "media.analysis" => print_analysis(value),
        "skills.list" => print_skills(value),
        "models.list" => print_models(value),
        "models.install" => print_model_installed(value),
        "skills.install" => print_skills_installed(value),
        "frame.contact_sheet" => print_contact_sheet(value),
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

/// One line per `fosscut host` event: JSON with `--json`, a sentence otherwise. Flushed at
/// once, since another program may be waiting for it.
pub fn print_host_event(event: &Value, json: bool) {
    use std::io::Write;
    if json {
        println!("{}", serde_json::to_string(event).unwrap_or_default());
    } else {
        match event["event"].as_str() {
            Some("ready") => println!(
                "Hosting {} at revision {}. Edits from the CLI and MCP now run here; press Ctrl-C to stop.",
                s(&event["bundle"]),
                event["revision"]
            ),
            Some("changed") => {
                let commands: Vec<&str> = event["commands"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                println!(
                    "revision {}: {} by {} ({})",
                    event["revision"],
                    s(&event["kind"]),
                    s(&event["actor"]),
                    commands.join(", ")
                );
            }
            _ => println!("Stopped hosting at revision {}.", event["revision"]),
        }
    }
    let _ = std::io::stdout().flush();
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
    if v["session"]["live"] == true {
        println!(
            "open live in {} {}: edits run there",
            s(&v["session"]["host"]["name"]),
            s(&v["session"]["host"]["version"])
        );
    }
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
        let folder = a["folder"]
            .as_str()
            .map(|f| format!("  [{f}]"))
            .unwrap_or_default();
        let missing = if a["missing"] == true {
            "  (missing)"
        } else {
            ""
        };
        println!(
            "{} {:<5} {}{size}  {}{folder}{missing}",
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
    // A lookup by name: each command or query with its arguments.
    if v.get("operations").is_none() {
        for item in [&v["commands"], &v["queries"]]
            .into_iter()
            .filter_map(Value::as_array)
            .flatten()
        {
            println!("{}  {}", s(&item["name"]), s(&item["summary"]));
            print_fields(&item["input_schema"]);
        }
        return;
    }
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
    println!("operations:");
    for o in v["operations"].as_array().into_iter().flatten() {
        println!("  {:<20} {}", s(&o["name"]), s(&o["summary"]));
    }
    println!("analyzers:");
    for a in v["analyzers"].as_array().into_iter().flatten() {
        println!("  {:<14} {}", s(&a["name"]), s(&a["summary"]));
    }
    println!("models (fosscut models install <name>):");
    for m in v["models"].as_array().into_iter().flatten() {
        println!(
            "  {:<22} {:>7.1} MB  {:<12} {}",
            s(&m["name"]),
            m["size_bytes"].as_f64().unwrap_or(0.0) / 1e6,
            s(&m["languages"]),
            if m["installed"] == Value::Bool(true) {
                "installed"
            } else {
                "not installed"
            }
        );
    }
    println!("export presets:");
    for p in v["export_presets"].as_array().into_iter().flatten() {
        println!("  {:<14} {}", s(&p["name"]), s(&p["description"]));
    }
}

/// One line per argument of a JSON Schema: name, whether it is required, and its description.
fn print_fields(schema: &Value) {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    for (name, field) in schema["properties"].as_object().into_iter().flatten() {
        let need = if required.contains(&name.as_str()) {
            "required"
        } else {
            "optional"
        };
        let about = s(&field["description"]).replace('\n', " ");
        println!("  {name:<16} {need:<9} {about}");
    }
}

/// "wrote" or, for a dry run, "would write".
fn wrote(v: &Value) -> &'static str {
    if v["dry_run"] == Value::Bool(true) {
        "dry run: would write"
    } else {
        "wrote"
    }
}

fn print_frame(v: &Value) {
    println!(
        "{} {} ({}x{}): frame {} at {}",
        wrote(v),
        s(&v["output"]),
        v["width"],
        v["height"],
        v["at"]["frames"],
        s(&v["at"]["timecode"])
    );
}

fn print_contact_sheet(v: &Value) {
    let tiles = v["tiles"].as_array().map(Vec::as_slice).unwrap_or_default();
    let (first, last) = match tiles {
        [first, .., last] => (&first["at"]["timecode"], &last["at"]["timecode"]),
        [only] => (&only["at"]["timecode"], &only["at"]["timecode"]),
        [] => (&Value::Null, &Value::Null),
    };
    println!(
        "{} {} ({}x{}): {} frames in {} columns, {} to {}",
        wrote(v),
        s(&v["output"]),
        v["width"],
        v["height"],
        tiles.len(),
        v["columns"],
        s(first),
        s(last)
    );
}

fn names(list: &Value) -> String {
    let names: Vec<&str> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}

fn print_analysis(v: &Value) {
    println!(
        "{}: measured {}; from the cache: {}",
        s(&v["asset"]),
        names(&v["computed"]),
        names(&v["cached"])
    );
    let r = &v["results"];
    if let Some(l) = r.get("loudness") {
        match (
            l["integrated_lufs"].as_f64(),
            l["sample_peak_dbfs"].as_f64(),
        ) {
            (Some(lufs), Some(peak)) => {
                println!("loudness  {lufs:.2} LUFS integrated, sample peak {peak:.2} dBFS")
            }
            (None, Some(peak)) => {
                println!("loudness  too short or quiet to integrate, sample peak {peak:.2} dBFS")
            }
            _ => println!("loudness  silent"),
        }
    }
    if let Some(silences) = r.get("silences") {
        let ranges: Vec<String> = silences["ranges"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|span| {
                format!(
                    "{}-{} s",
                    s(&span["start"]["seconds"]),
                    s(&span["end"]["seconds"])
                )
            })
            .collect();
        if ranges.is_empty() {
            println!(
                "silences  none below {} dBFS for {} ms",
                silences["threshold_dbfs"], silences["min_duration_ms"]
            );
        } else {
            println!(
                "silences  {} ranges, {} s in total: {}",
                ranges.len(),
                s(&silences["total"]["seconds"]),
                ranges.join(", ")
            );
        }
    }
    if let Some(scenes) = r.get("scenes") {
        let cuts: Vec<String> = scenes["cuts"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|cut| format!("frame {} ({} s)", cut["frame"], s(&cut["time"]["seconds"])))
            .collect();
        if cuts.is_empty() {
            println!("scenes    no cuts in {} frames", scenes["frames"]);
        } else {
            println!("scenes    {} cuts: {}", cuts.len(), cuts.join(", "));
        }
    }
    if let Some(t) = r.get("transcript") {
        print_transcript(t);
    }
}

/// The transcript part of `media analyze`: counts, fillers, pauses, then the text by segment.
fn print_transcript(t: &Value) {
    let list = |key: &str| t[key].as_array().cloned().unwrap_or_default();
    let (words, segments) = (list("words"), list("segments"));
    println!(
        "transcript {} words in {} segments, language {}, model {}",
        words.len(),
        segments.len(),
        s(&t["language"]),
        s(&t["model"])
    );
    let fillers: Vec<String> = list("fillers")
        .iter()
        .map(|f| format!("{} ({} s)", s(&f["text"]), s(&f["start"]["seconds"])))
        .collect();
    if fillers.is_empty() {
        println!("fillers   none");
    } else {
        println!("fillers   {}: {}", fillers.len(), fillers.join(", "));
    }
    let pauses: Vec<String> = list("pauses")
        .iter()
        .map(|p| {
            format!(
                "{}-{} s",
                s(&p["start"]["seconds"]),
                s(&p["end"]["seconds"])
            )
        })
        .collect();
    if pauses.is_empty() {
        println!("pauses    none over {} ms", t["min_pause_ms"]);
    } else {
        println!(
            "pauses    {} over {} ms: {}",
            pauses.len(),
            t["min_pause_ms"],
            pauses.join(", ")
        );
    }
    for segment in &segments {
        println!(
            "  {}-{} s {}",
            s(&segment["start"]["seconds"]),
            s(&segment["end"]["seconds"]),
            s(&segment["text"]).trim()
        );
    }
}

fn print_models(v: &Value) {
    for model in v["models"].as_array().into_iter().flatten() {
        println!(
            "{:<24} {:>9.1} MB  {:<10} {:<13} {}",
            s(&model["name"]),
            model["size_bytes"].as_f64().unwrap_or(0.0) / 1e6,
            s(&model["license"]),
            s(&model["languages"]),
            if model["installed"] == Value::Bool(true) {
                "installed"
            } else {
                "not installed"
            }
        );
    }
    println!("models folder: {}", s(&v["dir"]));
}

fn print_model_installed(v: &Value) {
    let done = match s(&v["status"]) {
        "unchanged" => "already installed:",
        "dry_run" => "dry run: would install",
        _ => "installed",
    };
    println!("{done} {} in {}", s(&v["model"]["name"]), s(&v["path"]));
}

fn print_skills(v: &Value) {
    for skill in v["skills"].as_array().into_iter().flatten() {
        println!("{}  {}", s(&skill["name"]), s(&skill["description"]));
    }
}

fn print_skills_installed(v: &Value) {
    let dry = v["dry_run"] == Value::Bool(true);
    for skill in v["skills"].as_array().into_iter().flatten() {
        let done = match (s(&skill["status"]), dry) {
            ("unchanged", _) => "already up to date:",
            ("updated", false) => "updated",
            ("updated", true) => "dry run: would update",
            (_, false) => "installed",
            (_, true) => "dry run: would install",
        };
        println!("{done} {} in {}", s(&skill["skill"]), s(&skill["path"]));
    }
}

fn print_presets(v: &Value) {
    for p in v["presets"].as_array().into_iter().flatten() {
        let mark = if p["available"] == Value::Bool(true) {
            "yes"
        } else {
            "no "
        };
        println!(
            "{:<11} {} .{:<5} {}",
            s(&p["name"]),
            mark,
            s(&p["extension"]),
            s(&p["description"])
        );
    }
}

/// Prints a verified export.
pub fn print_export(report: &fosscut_engine::ExportReport) {
    let file = &report.verified;
    let seconds = report.duration.get() as f64 / 705_600_000.0;
    let mut line = format!(
        "exported {} ({:.1} MB, {:.3} s",
        report.output,
        file.size_bytes as f64 / 1e6,
        seconds
    );
    if let Some(video) = &file.video {
        line.push_str(&format!(
            ", {} {}x{} @ {} fps, {} frames",
            video.codec,
            video.width,
            video.height,
            video.rate.map(|r| r.to_string()).unwrap_or_default(),
            video.frame_count
        ));
    }
    if let Some(audio) = &file.audio {
        line.push_str(&format!(
            ", {} {} Hz x {}",
            audio.codec, audio.sample_rate, audio.channels
        ));
    }
    line.push(')');
    println!("{line}");
    let how = if report.replayed {
        "replayed earlier request".to_string()
    } else {
        format!("rendered in {:.1} s", report.elapsed_ms as f64 / 1000.0)
    };
    let encoders: Vec<&str> = [&report.encoders.video, &report.encoders.audio]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    println!(
        "verified against the request; {how} with {}",
        encoders.join(" and ")
    );
}

/// A dry run of `fosscut export`: what would be written.
pub fn print_export_plan(plan: &fosscut_engine::ExportPlan) {
    let seconds = plan.duration.get() as f64 / 705_600_000.0;
    let mut line = format!("would export {} ({seconds:.3} s", plan.output);
    if let Some(video) = &plan.settings.video {
        line.push_str(&format!(
            ", {} {}x{} @ {} fps, {} frames",
            video.codec.probe_name(),
            video.width,
            video.height,
            video.rate,
            plan.frames
        ));
    }
    if let Some(audio) = &plan.settings.audio {
        line.push_str(&format!(
            ", {} {} Hz x {}",
            audio.codec.probe_name(),
            audio.sample_rate,
            audio.channels
        ));
    }
    line.push(')');
    println!("{line}");
    let encoders: Vec<&str> = [&plan.encoders.video, &plan.encoders.audio]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    println!(
        "dry run: nothing was written; it would use {}",
        encoders.join(" and ")
    );
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
