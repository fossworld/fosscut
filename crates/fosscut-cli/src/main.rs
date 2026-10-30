//! The `fosscut` command-line interface (ADR-0002): every editing command and query,
//! with `--json` output for scripts and AI agents.

#![forbid(unsafe_code)]

mod args;
mod output;

use std::io::Read;
use std::process::ExitCode;

use args::{
    AdjustmentCommand, AnimationCommand, CaptionsCommand, Cli, ClipCommand, ColorCommand, Command,
    EffectCommand, KeyframeCommand, LookCommand, MaskCommand, MediaCommand, ModelsCommand,
    ProjectCommand, SkillTarget, SkillsCommand, TextCommand, TimelineCommand, TrackCommand,
    TransitionCommand,
};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use clap::Parser;
use fosscut_core::{ApiError, REDO, Request, Response, UNDO, codes};
use fosscut_engine::live;
use fosscut_engine::project as bundle;
use fosscut_engine::{EngineError, ExportPlan, ExportReport, ExportRequest, Preset, Progress};
use fosscut_model::{Id, Project, TimelineFormat};
use fosscut_time::{Rate, TimeInput};
use serde_json::{Map, Value, json};

enum Outcome {
    Write(Response),
    Query(String, Value),
    Export(Box<ExportReport>),
    ExportPlan(Box<ExportPlan>),
    /// Nothing to print: the MCP session used stdout itself.
    Served,
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
        Outcome::Served => {}
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

/// `PATH=VALUE` from `--set`: the value as JSON, or as a string when it is a bare word.
fn property_value(text: &str) -> Result<(&str, Value), ApiError> {
    let (path, value) = text.split_once('=').ok_or_else(|| {
        ApiError::new(
            codes::INVALID_ARGUMENTS,
            format!("--set {text}: write PATH=VALUE, for example crop.left=0.1"),
        )
    })?;
    let value = serde_json::from_str(value).unwrap_or_else(|_| json!(value));
    Ok((path.trim(), value))
}

/// A keyframe value: a number, or `x,y` for a pair.
fn keyframe_value(text: &str) -> Result<Value, ApiError> {
    let numbers: Result<Vec<f64>, _> = text.split(',').map(|n| n.trim().parse::<f64>()).collect();
    match numbers {
        Ok(numbers) if numbers.len() == 1 => Ok(json!(numbers[0])),
        Ok(numbers) if numbers.len() == 2 => Ok(json!(numbers)),
        _ => Err(ApiError::new(
            codes::INVALID_ARGUMENTS,
            format!("--value {text}: give a number, or x,y for position and stretch"),
        )),
    }
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
        Command::Project(ProjectCommand::Info) => {
            let path = bundle::bundle_path(cli.project.as_deref())?;
            let projects = bundle::Projects::new("cli");
            let mut value = projects.query(&path, "project.info", Value::Null)?;
            value["session"] = serde_json::to_value(projects.status(&path)?).unwrap_or_default();
            Ok(Outcome::Query("project.info".into(), value))
        }
        Command::Capabilities { detail, names } => {
            let bundle = bundle::bundle_path(cli.project.as_deref()).ok();
            let value = fosscut_engine::capabilities(detail.as_str(), names, bundle.as_deref())?;
            Ok(Outcome::Query("capabilities".into(), value))
        }
        Command::Media(MediaCommand::Import { file, name }) => {
            let bundle = bundle::bundle_path(cli.project.as_deref())?;
            let args = fosscut_engine::import::media_add_args(file, name.as_deref(), &bundle)?;
            write(cli, "media.add", args)
        }
        Command::Media(MediaCommand::List) => query(cli, "media.list", Value::Null),
        Command::Media(MediaCommand::Analyze {
            asset,
            analyzers,
            refresh,
            silence_threshold,
            min_silence_ms,
            scene_threshold,
            min_shot_ms,
            model,
            language,
            min_pause_ms,
        }) => {
            let mut options = fosscut_engine::analysis::Options::default();
            options.transcript.model.clone_from(model);
            options.transcript.language.clone_from(language);
            if let Some(ms) = min_pause_ms {
                options.transcript.min_pause_ms = *ms;
            }
            if let Some(threshold) = silence_threshold {
                options.silences.threshold_dbfs = *threshold;
            }
            if let Some(ms) = min_silence_ms {
                options.silences.min_duration_ms = *ms;
            }
            if let Some(threshold) = scene_threshold {
                options.scenes.threshold = *threshold;
            }
            if let Some(ms) = min_shot_ms {
                options.scenes.min_shot_ms = *ms;
            }
            analyze_media(cli, asset, analyzers, options, *refresh)
        }
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
        Command::Clip(ClipCommand::Trim { clip, start, end }) => write(
            cli,
            "clip.trim",
            object(vec![
                ("clip", Some(json!(clip))),
                ("start", some(start)),
                ("end", some(end)),
            ]),
        ),
        Command::Clip(ClipCommand::Speed {
            clip,
            rate,
            curve,
            preset,
            reverse,
            forward,
            pitch,
            blend,
            nearest,
            ripple,
            reset,
        }) => {
            let curve = curve
                .as_deref()
                .map(|text| -> Result<Value, ApiError> {
                    let points: Result<Vec<Value>, ApiError> = text
                        .split(',')
                        .map(|point| {
                            let parsed = point.split_once(':').and_then(|(at, rate)| {
                                Some((
                                    at.trim().parse::<f64>().ok()?,
                                    rate.trim().parse::<f64>().ok()?,
                                ))
                            });
                            parsed
                                .map(|(at, rate)| json!({ "at": at, "rate": rate }))
                                .ok_or_else(|| {
                                    ApiError::new(
                                        codes::INVALID_ARGUMENTS,
                                        format!(
                                            "--curve: {point} is not AT:RATE, for example 0.5:2"
                                        ),
                                    )
                                })
                        })
                        .collect();
                    Ok(json!(points?))
                })
                .transpose()?;
            let flag = |on: bool, off: bool| (on || off).then(|| json!(on));
            write(
                cli,
                "clip.speed",
                object(vec![
                    ("clip", Some(json!(clip))),
                    ("rate", some(rate)),
                    ("curve", curve),
                    ("preset", some(preset)),
                    ("reverse", flag(*reverse, *forward)),
                    ("pitch", some(pitch)),
                    (
                        "interpolate",
                        (*blend || *nearest)
                            .then(|| json!(if *blend { "blend" } else { "nearest" })),
                    ),
                    ("ripple", ripple.then(|| json!(true))),
                    ("reset", reset.then(|| json!(true))),
                ]),
            )
        }
        Command::Clip(ClipCommand::Freeze { clip, at, duration }) => write(
            cli,
            "clip.freeze",
            object(vec![
                ("clip", Some(json!(clip))),
                ("at", Some(json!(at))),
                ("duration", some(duration)),
            ]),
        ),
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
        Command::Clip(ClipCommand::Set {
            clip,
            position,
            scale,
            rotation,
            opacity,
            volume,
            stretch,
            blend,
            fade_in,
            fade_out,
            pan,
            enabled,
            properties,
        }) => {
            let mut set = object(vec![
                ("transform.position", some(position)),
                ("transform.scale", some(scale)),
                ("transform.stretch", some(stretch)),
                ("transform.rotation", some(rotation)),
                ("opacity", some(opacity)),
                ("blend", some(blend)),
                ("volume", some(volume)),
                ("audio.fade_in", some(fade_in)),
                ("audio.fade_out", some(fade_out)),
                ("audio.pan", some(pan)),
                ("enabled", some(enabled)),
            ]);
            for property in properties {
                let (path, value) = property_value(property)?;
                set[path] = value;
            }
            write(cli, "clip.set", json!({ "clip": clip, "set": set }))
        }
        Command::Keyframe(KeyframeCommand::Set {
            clip,
            property,
            at,
            offset,
            value,
            easing,
        }) => {
            let value = value.as_deref().map(keyframe_value).transpose()?;
            write(
                cli,
                "keyframe.set",
                object(vec![
                    ("clip", Some(json!(clip))),
                    ("property", Some(json!(property))),
                    ("at", some(at)),
                    ("offset", some(offset)),
                    ("value", value),
                    ("easing", some(easing)),
                ]),
            )
        }
        Command::Keyframe(KeyframeCommand::Remove {
            clip,
            property,
            at,
            offset,
            all,
        }) => write(
            cli,
            "keyframe.remove",
            object(vec![
                ("clip", Some(json!(clip))),
                ("property", Some(json!(property))),
                ("at", some(at)),
                ("offset", some(offset)),
                ("all", Some(json!(all))),
            ]),
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
        Command::Timeline(TimelineCommand::Duplicate {
            width,
            height,
            fill,
            name,
            primary,
            timeline,
        }) => write(
            cli,
            "timeline.duplicate",
            object(vec![
                ("width", some(width)),
                ("height", some(height)),
                ("fit", fill.then_some(json!("fill"))),
                ("name", some(name)),
                ("primary", primary.then_some(json!(true))),
                ("timeline", some(timeline)),
            ]),
        ),
        Command::Timeline(TimelineCommand::RemoveRanges {
            ranges,
            tracks,
            no_ripple,
            timeline,
        }) => {
            let ranges = ranges
                .iter()
                .map(|range| {
                    let (start, end) = range.split_once("..").ok_or_else(|| {
                        ApiError::new(
                            codes::INVALID_ARGUMENTS,
                            format!("range {range:?} is not START..END"),
                        )
                        .with_hint(
                            "For example 3.4s..4.1s, 102f..123f or 00:00:03:12..00:00:04:03.",
                        )
                    })?;
                    let time = |text: &str| {
                        text.trim().parse::<TimeInput>().map_err(|e| {
                            ApiError::new(codes::INVALID_ARGUMENTS, format!("range {range:?}: {e}"))
                        })
                    };
                    Ok(json!({ "start": time(start)?, "end": time(end)? }))
                })
                .collect::<Result<Vec<Value>, ApiError>>()?;
            write(
                cli,
                "timeline.remove_ranges",
                object(vec![
                    ("ranges", Some(json!(ranges))),
                    ("tracks", some(tracks)),
                    ("ripple", no_ripple.then_some(json!(false))),
                    ("timeline", some(timeline)),
                ]),
            )
        }
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
        Command::Skills(SkillsCommand::List) => {
            let skills: Vec<Value> = fosscut_mcp::skills::SKILLS
                .iter()
                .map(|skill| json!({ "name": skill.name, "description": skill.description() }))
                .collect();
            Ok(Outcome::Query(
                "skills.list".into(),
                json!({ "skills": skills }),
            ))
        }
        Command::Skills(SkillsCommand::Install {
            target,
            user,
            dir,
            skill,
        }) => install_skills(cli, *target, *user, dir.as_deref(), skill.as_deref()),
        Command::Captions(command) => captions(cli, command),
        Command::Effect(command) => effect(cli, command),
        Command::Adjustment(AdjustmentCommand::Add {
            start,
            duration,
            effects,
            track,
        }) => {
            let effects: Vec<Value> = effects.iter().map(|t| json!({ "type": t })).collect();
            write(
                cli,
                "adjustment.add",
                object(vec![
                    ("start", some(&Some(start))),
                    ("duration", some(&Some(duration))),
                    ("effects", Some(json!(effects))),
                    ("track", some(track)),
                ]),
            )
        }
        Command::Look(LookCommand::Apply {
            look,
            clip,
            replace,
        }) => write(
            cli,
            "look.apply",
            json!({ "clip": clip, "look": look, "replace": replace }),
        ),
        Command::Color(command) => color(cli, command),
        Command::Transition(TransitionCommand::Set {
            clip,
            transition_type,
            duration,
            params,
        }) => write(
            cli,
            "transition.set",
            object(vec![
                ("clip", Some(json!(clip))),
                ("type", Some(json!(transition_type))),
                ("duration", some(duration)),
                ("params", Some(params_of(params)?)),
            ]),
        ),
        Command::Transition(TransitionCommand::Remove { clip }) => {
            write(cli, "transition.remove", json!({ "clip": clip }))
        }
        Command::Animation(AnimationCommand::Set {
            clip,
            entrance,
            exit,
            combo,
            clear,
        }) => {
            let animation = |text: &Option<String>| -> Result<Option<Value>, ApiError> {
                let Some(text) = text else {
                    return Ok(None);
                };
                Ok(Some(match text.split_once(':') {
                    Some((preset, duration)) => {
                        let duration: TimeInput =
                            duration.parse().map_err(|e: fosscut_time::TimeError| {
                                ApiError::new(codes::INVALID_ARGUMENTS, format!("{text}: {e}"))
                            })?;
                        json!({ "preset": preset, "duration": duration })
                    }
                    None => json!({ "preset": text }),
                }))
            };
            write(
                cli,
                "animation.set",
                object(vec![
                    ("clip", Some(json!(clip))),
                    ("in", animation(entrance)?),
                    ("out", animation(exit)?),
                    ("combo", animation(combo)?),
                    ("clear", Some(json!(clear))),
                ]),
            )
        }
        Command::Mask(command) => {
            let (name, mut args, settings) = match command {
                MaskCommand::Add {
                    clip,
                    shape,
                    settings,
                } => (
                    "mask.add",
                    json!({ "clip": clip, "shape": shape }),
                    settings,
                ),
                MaskCommand::Set {
                    clip,
                    mask,
                    settings,
                } => ("mask.set", json!({ "clip": clip, "mask": mask }), settings),
                MaskCommand::Remove { clip, mask } => (
                    "mask.remove",
                    json!({ "clip": clip, "mask": mask }),
                    &Vec::new(),
                ),
            };
            for text in settings {
                let (field, value) = property_value(text)?;
                args[field] = value;
            }
            write(cli, name, args)
        }
        Command::Models(ModelsCommand::List) => Ok(Outcome::Query(
            "models.list".into(),
            json!({
                "dir": fosscut_engine::models_dir().display().to_string(),
                "models": fosscut_engine::analysis::models(&fosscut_engine::models_dir()),
            }),
        )),
        Command::Models(ModelsCommand::Install { name, yes, from }) => {
            install_model(cli, name, *yes, from.as_deref())
        }
        Command::Mcp => {
            let project = bundle::bundle_path(cli.project.as_deref()).ok();
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| ApiError::new(codes::INTERNAL, format!("cannot start: {e}")))?;
            runtime
                .block_on(fosscut_mcp::serve_stdio(project))
                .map_err(|e| ApiError::new(codes::INTERNAL, format!("MCP session failed: {e}")))?;
            Ok(Outcome::Served)
        }
        Command::Host {
            events,
            exit_on_eof,
        } => host(cli, *events, *exit_on_eof),
        Command::Frame {
            time,
            out,
            width,
            timeline,
        } => frame(cli, time, out, *width, timeline.as_deref()),
        Command::ContactSheet {
            out,
            count,
            width,
            timeline,
        } => contact_sheet(cli, out, *count, *width, timeline.as_deref()),
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

/// `fosscut host`: serves the project until Ctrl-C (or, with `exit_on_eof`, until standard
/// input closes), printing a ready line and, with `events`, each committed change.
fn host(cli: &Cli, events: bool, exit_on_eof: bool) -> Result<Outcome, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let options = live::HostOptions::new("fosscut host", env!("CARGO_PKG_VERSION"));
    let host = live::Host::start(&path, options)?;
    let changes = host.subscribe();
    output::print_host_event(
        &json!({
            "event": "ready",
            "bundle": host.bundle(),
            "revision": host.revision(),
            "pid": std::process::id(),
        }),
        cli.json,
    );
    let (stop, stopped) = std::sync::mpsc::channel::<()>();
    if exit_on_eof {
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            let _ = stop.send(());
        });
    }
    loop {
        if stopped.try_recv().is_ok() {
            break;
        }
        match changes.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(change) if events => {
                let mut event = serde_json::to_value(&change).unwrap_or_default();
                event["event"] = json!("changed");
                output::print_host_event(&event, cli.json);
            }
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let revision = host.revision();
    host.close();
    output::print_host_event(
        &json!({ "event": "stopped", "revision": revision }),
        cli.json,
    );
    Ok(Outcome::Served)
}

fn execute(cli: &Cli, request: Request) -> Result<Outcome, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    Ok(Outcome::Write(bundle::execute(&path, request)?))
}

fn query(cli: &Cli, name: &str, args: Value) -> Result<Outcome, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    Ok(Outcome::Query(
        name.to_string(),
        bundle::query(&path, name, args)?,
    ))
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
    let project = bundle::snapshot(&bundle)?;
    let project = &project;
    let (timeline, format) = timeline_of(project, timeline)?;
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
        let plan = fosscut_engine::plan(project, &request).map_err(|e| engine_error(&e))?;
        return Ok(Outcome::ExportPlan(Box::new(plan)));
    }
    let mut renderer = renderer()?;
    let report = fosscut_engine::export(
        project,
        &request,
        &mut renderer,
        &mut progress_printer("exporting"),
    )
    .map_err(|e| engine_error(&e))?;
    Ok(Outcome::Export(Box::new(report)))
}

/// A timeline by id, or the primary timeline, with its format.
fn timeline_of<'a>(
    project: &'a Project,
    timeline: Option<&str>,
) -> Result<(Id, &'a TimelineFormat), ApiError> {
    let timeline = bundle::timeline(project, timeline)?;
    let format = &project.timelines[&timeline].format;
    Ok((timeline, format))
}

fn renderer() -> Result<fosscut_render::Renderer, ApiError> {
    fosscut_render::Renderer::headless().map_err(|e| ApiError::new(e.code(), e.to_string()))
}

/// Refuses to replace an existing file without --allow-destructive (ADR-0006, item 6).
fn guard_output(cli: &Cli, path: &Path) -> Result<(), ApiError> {
    if path.exists() && !cli.write.allow_destructive {
        return Err(ApiError::new(
            codes::GUARD_DESTRUCTIVE,
            format!("{} already exists", path.display()),
        )
        .with_hint("Pass --allow-destructive to overwrite it, or choose another path.")
        .with_details(json!({ "path": path.display().to_string() })));
    }
    Ok(())
}

/// Writes `image` as PNG unless this is a dry run.
fn write_png(cli: &Cli, path: &Path, image: &fosscut_render::Image) -> Result<(), ApiError> {
    if cli.write.dry_run {
        return Ok(());
    }
    let bytes = fosscut_engine::frame::png(image).map_err(|e| engine_error(&e))?;
    std::fs::write(path, bytes).map_err(|e| {
        ApiError::new(
            "fosscut.media.write_failed",
            format!("cannot write {}: {e}", path.display()),
        )
    })
}

/// `fosscut frame`: one frame of the timeline as a PNG file. A dry run renders and reports
/// but writes nothing.
fn frame(
    cli: &Cli,
    time: &TimeInput,
    out: &Path,
    width: Option<u32>,
    timeline: Option<&str>,
) -> Result<Outcome, ApiError> {
    let bundle = bundle::bundle_path(cli.project.as_deref())?;
    let project = bundle::snapshot(&bundle)?;
    let project = &project;
    let (timeline, format) = timeline_of(project, timeline)?;
    let time = time.resolve(format.rate)?;
    guard_output(cli, out)?;
    let still =
        fosscut_engine::frame::frame(project, &bundle, &timeline, time, width, &mut renderer()?)
            .map_err(|e| engine_error(&e))?;
    write_png(cli, out, &still.image)?;
    Ok(Outcome::Query(
        "frame.grab".into(),
        json!({
            "output": out.display().to_string(),
            "dry_run": cli.write.dry_run,
            "at": still.at,
            "width": still.image.width,
            "height": still.image.height,
        }),
    ))
}

/// `fosscut contact-sheet`: frames spread over the timeline, tiled into one PNG file.
fn contact_sheet(
    cli: &Cli,
    out: &Path,
    count: u32,
    width: u32,
    timeline: Option<&str>,
) -> Result<Outcome, ApiError> {
    let bundle = bundle::bundle_path(cli.project.as_deref())?;
    let project = bundle::snapshot(&bundle)?;
    let project = &project;
    let (timeline, _) = timeline_of(project, timeline)?;
    guard_output(cli, out)?;
    let sheet = fosscut_engine::frame::contact_sheet(
        project,
        &bundle,
        &timeline,
        count,
        width,
        &mut renderer()?,
    )
    .map_err(|e| engine_error(&e))?;
    write_png(cli, out, &sheet.image)?;
    Ok(Outcome::Query(
        "frame.contact_sheet".into(),
        json!({
            "output": out.display().to_string(),
            "dry_run": cli.write.dry_run,
            "width": sheet.image.width,
            "height": sheet.image.height,
            "columns": sheet.columns,
            "tiles": sheet.tiles,
        }),
    ))
}

/// `fosscut skills install`: copies built-in skills into an agent's skill folder. Nothing
/// is written when a dry run is asked for, or when a changed copy would be replaced without
/// --allow-destructive.
fn install_skills(
    cli: &Cli,
    target: Option<SkillTarget>,
    user: bool,
    dir: Option<&Path>,
    only: Option<&str>,
) -> Result<Outcome, ApiError> {
    let root = match (dir, target) {
        (Some(dir), _) => dir.to_path_buf(),
        (None, Some(target)) if user => home_dir()?.join(target.folder()),
        (None, Some(target)) => PathBuf::from(target.folder()),
        (None, None) => {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "choose where to install the skills",
            )
            .with_hint("Pass --target claude, --target codex, or --dir <folder>."));
        }
    };
    let skills: Vec<&fosscut_mcp::skills::Skill> = match only {
        Some(name) => vec![fosscut_mcp::skills::skill(name).ok_or_else(|| {
            let names: Vec<&str> = fosscut_mcp::skills::SKILLS.iter().map(|s| s.name).collect();
            ApiError::new(
                codes::INVALID_ARGUMENTS,
                format!("there is no skill {name}"),
            )
            .with_hint(format!("Skills: {}.", names.join(", ")))
        })?],
        None => fosscut_mcp::skills::SKILLS.iter().collect(),
    };
    let mut plan = Vec::new();
    for skill in &skills {
        let folder = root.join(skill.name);
        let mut status = "unchanged";
        for (path, text) in skill.files {
            match std::fs::read_to_string(folder.join(path)) {
                Ok(existing) if existing == *text => {}
                Ok(_) => status = "updated",
                Err(_) if status == "unchanged" => status = "installed",
                Err(_) => {}
            }
        }
        plan.push((skill, folder, status));
    }
    let changed: Vec<String> = plan
        .iter()
        .filter(|(_, _, status)| *status == "updated")
        .map(|(_, folder, _)| folder.display().to_string())
        .collect();
    if !changed.is_empty() && !cli.write.allow_destructive {
        return Err(ApiError::new(
            codes::GUARD_DESTRUCTIVE,
            format!("{} has local changes", changed.join(", ")),
        )
        .with_hint(
            "Pass --allow-destructive to replace them with this version of FossCut's skills.",
        )
        .with_details(json!({ "paths": changed })));
    }
    if !cli.write.dry_run {
        for (skill, folder, status) in &plan {
            if *status == "unchanged" {
                continue;
            }
            for (path, text) in skill.files {
                let file = folder.join(path);
                let written = file
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&file, text));
                written.map_err(|e| {
                    ApiError::new(
                        "fosscut.media.write_failed",
                        format!("cannot write {}: {e}", file.display()),
                    )
                })?;
            }
        }
    }
    let skills: Vec<Value> = plan
        .iter()
        .map(|(skill, folder, status)| {
            json!({ "skill": skill.name, "path": folder.display().to_string(), "status": status })
        })
        .collect();
    Ok(Outcome::Query(
        "skills.install".into(),
        json!({ "dry_run": cli.write.dry_run, "skills": skills }),
    ))
}

/// The user's home folder, from `HOME` (or `USERPROFILE` on Windows).
fn home_dir() -> Result<PathBuf, ApiError> {
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|var| std::env::var_os(var).filter(|v| !v.is_empty()))
        .map(PathBuf::from)
        .ok_or_else(|| {
            ApiError::new(codes::INVALID_ARGUMENTS, "cannot find your home folder")
                .with_hint("Set HOME, or install into a folder with --dir.")
        })
}

/// The preset an output path implies when none is given.
fn preset_for(output: &Path) -> Result<Preset, ApiError> {
    Preset::for_output(output).ok_or_else(|| {
        ApiError::new(
            "fosscut.export.codec_unavailable",
            "this machine cannot encode video",
        )
    })
}

/// `fosscut captions ...`: generate and import become `captions.add` with the global write
/// options; export writes an SRT file; style and edit run their commands.
/// Parameters from `NAME=VALUE` arguments.
fn params_of(values: &[String]) -> Result<Value, ApiError> {
    let mut params = Map::new();
    for text in values {
        let (name, value) = property_value(text)?;
        params.insert(name.to_string(), value);
    }
    Ok(Value::Object(params))
}

fn effect(cli: &Cli, command: &EffectCommand) -> Result<Outcome, ApiError> {
    match command {
        EffectCommand::Add {
            clip,
            effect_type,
            params,
            index,
        } => write(
            cli,
            "effect.add",
            object(vec![
                ("clip", Some(json!(clip))),
                ("type", Some(json!(effect_type))),
                ("params", Some(params_of(params)?)),
                ("index", some(index)),
            ]),
        ),
        EffectCommand::Set {
            clip,
            effect,
            params,
            reset,
            enabled,
        } => write(
            cli,
            "effect.set",
            object(vec![
                ("clip", Some(json!(clip))),
                ("effect", Some(json!(effect))),
                ("params", Some(params_of(params)?)),
                ("reset", Some(json!(reset))),
                ("enabled", some(enabled)),
            ]),
        ),
        EffectCommand::Remove { clip, effect } => write(
            cli,
            "effect.remove",
            json!({ "clip": clip, "effect": effect }),
        ),
        EffectCommand::Move {
            clip,
            effect,
            index,
        } => write(
            cli,
            "effect.move",
            json!({ "clip": clip, "effect": effect, "index": index }),
        ),
        EffectCommand::Copy { from, to, append } => write(
            cli,
            "effect.copy",
            json!({ "from": from, "to": to, "append": append }),
        ),
    }
}

/// `fosscut color auto|match`: measures on this machine, then adds the correction with
/// `effect.add` as one undoable step.
fn color(cli: &Cli, command: &ColorCommand) -> Result<Outcome, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let parse = |id: &str, what: &str| {
        Id::parse(id).map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("{what}: {e}")))
    };
    let (clip, reference) = match command {
        ColorCommand::Auto { clip } => (parse(clip, "clip")?, None),
        ColorCommand::Match { clip, reference } => {
            (parse(clip, "clip")?, Some(parse(reference, "reference")?))
        }
    };
    let mut renderer = renderer()?;
    let suggestion =
        fosscut_engine::color::suggest(&snapshot, &path, &clip, reference.as_ref(), &mut renderer)
            .map_err(|e| engine_error(&e))?;
    write(cli, "effect.add", suggestion.effect_add)
}

fn captions(cli: &Cli, command: &CaptionsCommand) -> Result<Outcome, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    match command {
        CaptionsCommand::Generate {
            preset,
            source_track,
            track,
            timeline,
            max_words,
            max_lines,
            model,
            language,
            min_pause_ms,
            replace,
        } => {
            let snapshot = bundle::snapshot(&path)?;
            let mut transcript = fosscut_engine::analysis::TranscriptOptions {
                model: model.clone(),
                language: language.clone(),
                ..Default::default()
            };
            if let Some(ms) = min_pause_ms {
                transcript.min_pause_ms = *ms;
            }
            let parse = |id: &Option<String>, what: &str| {
                id.as_deref()
                    .map(Id::parse)
                    .transpose()
                    .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("{what}: {e}")))
            };
            let generated = fosscut_engine::captions::generate(
                &snapshot,
                &fosscut_engine::captions::GenerateRequest {
                    bundle: path.clone(),
                    timeline: parse(timeline, "timeline")?,
                    source_track: parse(source_track, "source track")?,
                    transcript,
                    cache: fosscut_engine::cache_dir(),
                    models: fosscut_engine::models_dir(),
                },
            )
            .map_err(|e| engine_error(&e))?;
            let paging = object(vec![
                ("max_words", some(max_words)),
                ("max_lines", some(max_lines)),
            ]);
            write(
                cli,
                "captions.add",
                object(vec![
                    ("words", Some(json!(generated.words))),
                    ("timeline", Some(json!(generated.timeline))),
                    ("track", some(track)),
                    ("preset", some(preset)),
                    ("paging", Some(paging)),
                    ("replace", replace.then_some(json!(true))),
                ]),
            )
        }
        CaptionsCommand::Import {
            file,
            preset,
            track,
            timeline,
            replace,
        } => {
            let text = std::fs::read_to_string(file).map_err(|e| {
                ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    format!("cannot read {}: {e}", file.display()),
                )
            })?;
            let cues = fosscut_engine::captions::parse_srt(&text).map_err(|e| engine_error(&e))?;
            let snapshot = bundle::snapshot(&path)?;
            let (timeline_id, _) = timeline_of(&snapshot, timeline.as_deref())?;
            let rate = snapshot.timelines[&timeline_id].format.rate;
            write(
                cli,
                "captions.add",
                object(vec![
                    (
                        "words",
                        Some(json!(fosscut_engine::captions::cue_words(&cues, rate))),
                    ),
                    ("timeline", Some(json!(timeline_id))),
                    ("track", some(track)),
                    ("preset", some(preset)),
                    ("paging", Some(fosscut_engine::captions::cue_paging())),
                    ("replace", replace.then_some(json!(true))),
                ]),
            )
        }
        CaptionsCommand::Export {
            file,
            track,
            timeline,
        } => {
            let snapshot = bundle::snapshot(&path)?;
            let ids = |id: &Option<String>| id.as_deref().map(Id::parse).transpose();
            let (timeline, track) = (
                ids(timeline).map_err(|e| {
                    ApiError::new(codes::INVALID_ARGUMENTS, format!("timeline: {e}"))
                })?,
                ids(track)
                    .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("track: {e}")))?,
            );
            let srt =
                fosscut_engine::captions::export_srt(&snapshot, timeline.as_ref(), track.as_ref())
                    .map_err(|e| engine_error(&e))?;
            if file.exists() && !cli.write.allow_destructive {
                return Err(engine_error(&EngineError::OutputExists {
                    path: file.display().to_string(),
                }));
            }
            let cues = srt.matches(" --> ").count();
            if !cli.write.dry_run {
                std::fs::write(file, &srt).map_err(|e| {
                    ApiError::new(
                        "fosscut.media.write_failed",
                        format!("cannot write {}: {e}", file.display()),
                    )
                })?;
            }
            Ok(Outcome::Query(
                "captions.export".into(),
                json!({
                    "file": file.display().to_string(),
                    "cues": cues,
                    "bytes": srt.len(),
                    "dry_run": cli.write.dry_run,
                    "preview": fosscut_engine::captions::srt_preview(&srt, 5),
                }),
            ))
        }
        CaptionsCommand::Style {
            preset,
            style,
            track,
            timeline,
        } => {
            let style = style
                .as_deref()
                .map(|text| {
                    let text = read_input(text)?;
                    serde_json::from_str::<Value>(&text)
                        .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("style: {e}")))
                })
                .transpose()?;
            write(
                cli,
                "captions.style",
                object(vec![
                    ("preset", some(preset)),
                    ("style", style),
                    ("track", some(track)),
                    ("timeline", some(timeline)),
                ]),
            )
        }
        CaptionsCommand::Edit {
            clip,
            text,
            emphasis,
        } => write(
            cli,
            "captions.edit",
            object(vec![
                ("clip", Some(json!(clip))),
                ("text", some(&text.as_ref().map(|t| t.replace("\\n", "\n")))),
                ("emphasis", some(emphasis)),
            ]),
        ),
    }
}

/// `fosscut models install`: a download needs `--yes`; a local file does not.
fn install_model(
    cli: &Cli,
    name: &str,
    yes: bool,
    from: Option<&Path>,
) -> Result<Outcome, ApiError> {
    use fosscut_engine::analysis::ModelSource;
    let source = match from {
        Some(file) => ModelSource::File(file.to_path_buf()),
        None => ModelSource::Download(None),
    };
    if yes
        && from.is_none()
        && !cli.write.dry_run
        && let Some(model) = fosscut_engine::analysis::models(&fosscut_engine::models_dir())
            .into_iter()
            .find(|m| m.name == name && !m.installed)
    {
        eprintln!(
            "downloading {} ({:.1} MB, {}) from {}",
            model.name,
            model.size_bytes as f64 / 1e6,
            model.license,
            model.url
        );
    }
    let report = fosscut_engine::analysis::install_model(
        &fosscut_engine::models_dir(),
        name,
        &source,
        yes,
        cli.write.dry_run,
    )
    .map_err(|e| engine_error(&e))?;
    Ok(Outcome::Query("models.install".into(), json!(report)))
}

/// `fosscut media analyze`: cached results, or a measurement with progress on stderr.
fn analyze_media(
    cli: &Cli,
    asset: &str,
    names: &[String],
    options: fosscut_engine::analysis::Options,
    refresh: bool,
) -> Result<Outcome, ApiError> {
    let bundle = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&bundle)?;
    let asset = Id::parse(asset)
        .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("asset: {e}")))?;
    let request = fosscut_engine::analysis::AnalysisRequest {
        bundle: bundle.clone(),
        asset,
        analyzers: fosscut_engine::analysis::analyzers(names).map_err(|e| engine_error(&e))?,
        options,
        refresh,
        cache: fosscut_engine::cache_dir(),
        models: fosscut_engine::models_dir(),
    };
    let report =
        fosscut_engine::analysis::analyze(&snapshot, &request, &mut progress_printer("analyzing"))
            .map_err(|e| engine_error(&e))?;
    Ok(Outcome::Query("media.analysis".into(), json!(report)))
}

/// Progress on stderr: a live counter in a terminal, every tenth otherwise (for agents).
fn progress_printer(doing: &'static str) -> impl FnMut(Progress) -> bool {
    let terminal = std::io::stderr().is_terminal();
    let mut reported = 0;
    move |progress: Progress| {
        if terminal {
            eprint!("\r{doing} {}/{}", progress.done, progress.total);
            if progress.done == progress.total {
                eprintln!();
            }
        } else {
            let tenth = progress.done * 10 / progress.total.max(1);
            if tenth > reported {
                reported = tenth;
                eprintln!("{doing} {}%", tenth * 10);
            }
        }
        true
    }
}

fn engine_error(error: &EngineError) -> ApiError {
    ApiError::from(error)
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
