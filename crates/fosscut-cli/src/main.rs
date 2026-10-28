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
use clap::Parser;
use fosscut_core::{ApiError, REDO, Request, Response, Session, UNDO, codes};
use fosscut_model::{DirStore, MemoryStore, Project, TimelineFormat};
use fosscut_time::Rate;
use serde_json::{Map, Value, json};

enum Outcome {
    Write(Response),
    Query(String, Value),
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
