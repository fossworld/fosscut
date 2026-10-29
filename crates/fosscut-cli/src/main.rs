//! The `fosscut` command-line interface (ADR-0002): every editing command and query,
//! with `--json` output for scripts and AI agents.

#![forbid(unsafe_code)]

mod args;
mod bundle;
mod output;
mod probe;

use std::io::Read;
use std::process::ExitCode;

use args::{
    Cli, ClipCommand, Command, MediaCommand, ProjectCommand, TextCommand, TimelineCommand,
    TrackCommand,
};
use std::io::IsTerminal;
use std::path::Path;

use clap::Parser;
use fosscut_core::{ApiError, REDO, Request, Response, Session, UNDO, codes};
use fosscut_engine::{ExportError, ExportPlan, ExportReport, ExportRequest, Preset, Progress};
use fosscut_model::{DirStore, MemoryStore, Project, TimelineFormat};
use fosscut_time::Rate;
use serde_json::{Map, Value, json};

enum Outcome {
    Write(Response),
    Query(String, Value),
    Export(Box<ExportReport>),
    ExportPlan(Box<ExportPlan>),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(outcome) => {
            print(&cli, &outcome);
            ExitCode::SUCCESS
        }
        Err(error) => {
            output::print_error(&error, cli.json);
            ExitCode::FAILURE
        }
    }
}

fn print(cli: &Cli, outcome: &Outcome) {
    match outcome {
        Outcome::Write(response) if cli.json => println!("{}", pretty(&json!(response))),
        Outcome::Write(response) => output::print_response(response),
        Outcome::Query(_, value) if cli.json => println!("{}", pretty(value)),
        Outcome::Query(name, value) => output::print_query(name, value),
        Outcome::Export(report) if cli.json => println!("{}", pretty(&json!(report))),
        Outcome::Export(report) => output::print_export(report),
        Outcome::ExportPlan(plan) if cli.json => {
            let mut value = json!(plan);
            value["dry_run"] = json!(true);
            println!("{}", pretty(&value));
        }
        Outcome::ExportPlan(plan) => output::print_export_plan(plan),
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// Builds a JSON object, leaving out `None` values.
fn object(fields: Vec<(&str, Option<Value>)>) -> Value {
    let map: Map<String, Value> = fields
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k.to_string(), v)))
        .collect();
    Value::Object(map)
}

fn some<T: serde::Serialize>(value: &Option<T>) -> Option<Value> {
    value.as_ref().map(|v| json!(v))
}

fn run(cli: &Cli) -> Result<Outcome, ApiError> {
    match &cli.command {
        Command::Project(ProjectCommand::New {
            path,
            name,
            width,
            height,
            rate,
        }) => {
            let rate: Rate = rate.parse()?;
            let name = name.clone().unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled".into())
            });
            let format = TimelineFormat {
                width: *width,
                height: *height,
                rate,
                sample_rate: 48_000,
            };
            let session = bundle::create(path, &name, format)?;
            Ok(Outcome::Query(
                "project.info".into(),
                session.query("project.info", Value::Null)?,
            ))
        }
        Command::Project(ProjectCommand::Info) => query(cli, "project.info", Value::Null),
        Command::Capabilities { detail } => {
            let value =
                match bundle::bundle_path(cli.project.as_deref()).and_then(|p| bundle::open(&p)) {
                    Ok(session) => {
                        session.query("capabilities", json!({ "detail": detail.as_str() }))?
                    }
                    Err(_) => scratch_session()?
                        .query("capabilities", json!({ "detail": detail.as_str() }))?,
                };
            let mut value = value;
            value["export_presets"] = json!(fosscut_engine::presets());
            Ok(Outcome::Query("capabilities".into(), value))
        }
        Command::Media(MediaCommand::Import { file, name }) => {
            let bundle = bundle::bundle_path(cli.project.as_deref())?;
            let probed = probe::probe(file)?;
            let args = json!({
                "name": name.clone().unwrap_or_else(|| file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                "kind": probed.kind,
                "path": probe::path_hints(file, &bundle)?,
                "identity": probe::identity(file)?,
                "info": probed.info,
            });
            write(cli, "media.add", args)
        }
        Command::Media(MediaCommand::List) => query(cli, "media.list", Value::Null),
        Command::Track(TrackCommand::Add {
            kind,
            name,
            index,
            timeline,
        }) => write(
            cli,
            "track.add",
            object(vec![
                ("kind", Some(json!(kind.as_str()))),
                ("name", some(name)),
                ("index", some(index)),
                ("timeline", some(timeline)),
            ]),
        ),
        Command::Clip(ClipCommand::Add {
            asset,
            track,
            start,
            source_in,
            duration,
            timeline,
        }) => write(
            cli,
            "clip.add",
            object(vec![
                ("asset", Some(json!(asset))),
                ("track", some(track)),
                ("start", some(start)),
                ("source_in", some(source_in)),
                ("duration", some(duration)),
                ("timeline", some(timeline)),
            ]),
        ),
        Command::Clip(ClipCommand::Split { clip, at }) => {
            write(cli, "clip.split", json!({ "clip": clip, "at": at }))
        }
        Command::Clip(ClipCommand::Move { clip, start, track }) => write(
            cli,
            "clip.move",
            object(vec![
                ("clip", Some(json!(clip))),
                ("start", some(start)),
                ("track", some(track)),
            ]),
        ),
        Command::Clip(ClipCommand::Remove { clip, ripple }) => write(
            cli,
            "clip.remove",
            json!({ "clip": clip, "ripple": ripple }),
        ),
        Command::Text(TextCommand::Add {
            text,
            track,
            start,
            duration,
            color,
            size,
            font,
            timeline,
        }) => {
            let style = (color.is_some() || size.is_some() || font.is_some()).then(|| {
                object(vec![
                    ("color", some(color)),
                    ("size", some(size)),
                    ("font", some(font)),
                ])
            });
            write(
                cli,
                "text.add",
                object(vec![
                    ("text", Some(json!(text.replace("\\n", "\n")))),
                    ("track", some(track)),
                    ("start", some(start)),
                    ("duration", some(duration)),
                    ("style", style),
                    ("timeline", some(timeline)),
                ]),
            )
        }
        Command::Timeline(TimelineCommand::Read { detail, timeline }) => query(
            cli,
            "timeline.read",
            object(vec![
                ("detail", Some(json!(detail.as_str()))),
                ("timeline", some(timeline)),
            ]),
        ),
        Command::History { limit } => {
            query(cli, "history.list", object(vec![("limit", some(limit))]))
        }
        Command::Undo => write(cli, UNDO, json!({})),
        Command::Redo => write(cli, REDO, json!({})),
        Command::Exec { request } => {
            let mut request: Request = serde_json::from_str(&read_input(request)?)
                .map_err(|e| ApiError::new(codes::REQUEST_INVALID, format!("request: {e}")))?;
            apply_options(cli, &mut request, false);
            execute(cli, request)
        }
        Command::Export {
            output,
            preset,
            timeline,
            presets,
        } => export(
            cli,
            output.as_deref(),
            preset.as_deref(),
            timeline.as_deref(),
            *presets,
        ),
        Command::Query { name, args } => {
            let args = match args {
                Some(text) => serde_json::from_str(&read_input(text)?).map_err(|e| {
                    ApiError::new(codes::INVALID_ARGUMENTS, format!("query arguments: {e}"))
                })?,
                None => Value::Null,
            };
            query(cli, name, args)
        }
    }
}

/// Fills envelope fields from command-line options; with `overwrite` they replace request values.
fn apply_options(cli: &Cli, request: &mut Request, overwrite: bool) {
    let w = &cli.write;
    if overwrite || request.expected_revision.is_none() {
        request.expected_revision = w.expected_revision.or(request.expected_revision);
    }
    request.dry_run |= w.dry_run;
    request.allow_destructive |= w.allow_destructive;
    if overwrite || request.request_id.is_none() {
        request.request_id = w.request_id.clone().or(request.request_id.take());
    }
    if overwrite || request.reason.is_none() {
        request.reason = w.reason.clone().or(request.reason.take());
    }
    if overwrite || request.actor == "unknown" {
        request.actor = w.actor.clone();
    }
    request.at_ms = request.at_ms.or_else(bundle::now_ms);
}

fn write(cli: &Cli, command: &str, args: Value) -> Result<Outcome, ApiError> {
    let mut request = Request::single(command, args);
    apply_options(cli, &mut request, true);
    execute(cli, request)
}

fn execute(cli: &Cli, request: Request) -> Result<Outcome, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let _lock = bundle::BundleLock::acquire(&path)?;
    let mut session = bundle::open(&path)?;
    Ok(Outcome::Write(session.execute(request)?))
}

fn query(cli: &Cli, name: &str, args: Value) -> Result<Outcome, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let session: Session<DirStore> = bundle::open(&path)?;
    Ok(Outcome::Query(name.to_string(), session.query(name, args)?))
}

/// Renders and verifies an export (milestone 3), or lists the presets.
fn export(
    cli: &Cli,
    output: Option<&Path>,
    preset: Option<&str>,
    timeline: Option<&str>,
    list: bool,
) -> Result<Outcome, ApiError> {
    if list {
        return Ok(Outcome::Query(
            "export.presets".into(),
            json!({ "presets": fosscut_engine::presets() }),
        ));
    }
    let output = output.ok_or_else(|| {
        ApiError::new(codes::INVALID_ARGUMENTS, "an output file is required")
            .with_hint("For example: fosscut export Promo.mp4. List presets with --presets.")
    })?;
    let bundle = bundle::bundle_path(cli.project.as_deref())?;
    let session = bundle::open(&bundle)?;
    let project = session.project();
    let timeline = match timeline {
        Some(text) => fosscut_model::Id::parse(text)
            .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("timeline: {e}")))?,
        None => project.manifest.primary_timeline.clone(),
    };
    let format = &project
        .timelines
        .get(&timeline)
        .ok_or_else(|| {
            ApiError::new(
                codes::TIMELINE_NOT_FOUND,
                format!("timeline {timeline} does not exist"),
            )
        })?
        .format;
    let preset = match preset {
        Some(name) => Preset::parse(name).ok_or_else(|| {
            let names: Vec<&str> = Preset::ALL.iter().map(|p| p.name()).collect();
            ApiError::new(codes::INVALID_ARGUMENTS, format!("unknown preset {name}"))
                .with_hint(format!("Presets: {}.", names.join(", ")))
        })?,
        None => preset_for(output)?,
    };
    let request = ExportRequest {
        bundle: bundle.clone(),
        timeline,
        settings: preset.settings(format),
        output: output.to_path_buf(),
        allow_overwrite: cli.write.allow_destructive,
        expected_revision: cli.write.expected_revision,
        request_id: cli.write.request_id.clone(),
        cache: fosscut_engine::cache_dir(),
    };
    if cli.write.dry_run {
        let plan = fosscut_engine::plan(project, &request).map_err(|e| export_error(&e))?;
        return Ok(Outcome::ExportPlan(Box::new(plan)));
    }
    let mut renderer =
        fosscut_render::Renderer::headless().map_err(|e| ApiError::new(e.code(), e.to_string()))?;
    let report = fosscut_engine::export(project, &request, &mut renderer, &mut progress_printer())
        .map_err(|e| export_error(&e))?;
    Ok(Outcome::Export(Box::new(report)))
}

/// The preset an output path implies when none is given.
fn preset_for(output: &Path) -> Result<Preset, ApiError> {
    let extension = output
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let by_extension = match extension.as_str() {
        "mov" => Some(Preset::MovProRes),
        "webm" => Some(Preset::WebmVp9),
        "m4a" => Some(Preset::M4aAac),
        _ => None,
    };
    by_extension.or_else(Preset::default_video).ok_or_else(|| {
        ApiError::new(
            "fosscut.export.codec_unavailable",
            "this machine cannot encode video",
        )
    })
}

/// Progress on stderr: a live counter in a terminal, every tenth otherwise (for agents).
fn progress_printer() -> impl FnMut(Progress) -> bool {
    let terminal = std::io::stderr().is_terminal();
    let mut reported = 0;
    move |progress: Progress| {
        if terminal {
            eprint!("\rexporting {}/{}", progress.done, progress.total);
            if progress.done == progress.total {
                eprintln!();
            }
        } else {
            let tenth = progress.done * 10 / progress.total.max(1);
            if tenth > reported {
                reported = tenth;
                eprintln!("exporting {}%", tenth * 10);
            }
        }
        true
    }
}

fn export_error(error: &ExportError) -> ApiError {
    let mut api = ApiError::new(error.code(), error.to_string()).with_details(error.details());
    if let Some(hint) = error.hint() {
        api = api.with_hint(hint);
    }
    api
}

/// A throwaway in-memory session so that `capabilities` works without a project.
fn scratch_session() -> Result<Session<MemoryStore>, ApiError> {
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let id = fosscut_model::Id::new(fosscut_model::IdKind::Project, 0);
    Session::create(MemoryStore::new(), Project::new(id, "capabilities", format))
}

/// Reads JSON text given inline, as `@file`, or `-` for stdin.
fn read_input(text: &str) -> Result<String, ApiError> {
    if text == "-" {
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer).map_err(|e| {
            ApiError::new(codes::REQUEST_INVALID, format!("cannot read stdin: {e}"))
        })?;
        Ok(buffer)
    } else if let Some(path) = text.strip_prefix('@') {
        std::fs::read_to_string(path)
            .map_err(|e| ApiError::new(codes::REQUEST_INVALID, format!("cannot read {path}: {e}")))
    } else {
        Ok(text.to_string())
    }
}
