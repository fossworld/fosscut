//! The `fosscut` command-line interface (ADR-0002): every editing command and query,
//! with `--json` output for scripts and AI agents.

#![forbid(unsafe_code)]

mod args;
mod output;

use std::io::Read;
use std::process::ExitCode;

use args::{
    AdjustmentCommand, AnimationCommand, AudioCommand, BeatsCommand, CacheCommand, CaptionsCommand,
    Cli, ClipCommand, ClipsCommand, ColorCommand, Command, EffectCommand, FramingArgs,
    KeyframeCommand, LibraryCommand, LookCommand, MarkerCommand, MaskCommand, MediaCommand,
    ModelsCommand, MulticamCommand, PreferencesCommand, ProjectCommand, RevealArgs, ShortsCommand,
    SkillTarget, SkillsCommand, SubjectArgs, TextCommand, TimelineCommand, TrackCommand,
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

/// Stack for the thread that parses and runs commands.
const STACK: usize = 16 * 1024 * 1024;

fn main() -> ExitCode {
    // The command line's argument types are large, and Windows gives the main thread 1 MiB
    // of stack, so parsing and running happen on a thread with room.
    match std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(cli_main)
    {
        Ok(worker) => worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
        Err(_) => cli_main(),
    }
}

fn cli_main() -> ExitCode {
    let cli = Cli::parse();
    // Hardware decoding and encoding follow the preferences in every command; a broken
    // preferences file is reported by the commands that read it.
    if let Ok(preferences) = preferences() {
        preferences.apply_performance();
    }
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
            let rate = match rate {
                Some(rate) => rate_of(rate)?,
                None => rate_of(&preferences()?.timeline.frame_rate)?,
            };
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
        Command::Media(MediaCommand::List) => {
            let bundle = bundle::bundle_path(cli.project.as_deref())?;
            let mut listing = bundle::query(&bundle, "media.list", Value::Null)?;
            fosscut_engine::import::mark_missing(&mut listing, &bundle);
            Ok(Outcome::Query("media.list".into(), listing))
        }
        Command::Media(MediaCommand::Rename { asset, name }) => {
            write(cli, "media.rename", json!({ "asset": asset, "name": name }))
        }
        Command::Media(MediaCommand::Remove { asset, with_clips }) => write(
            cli,
            "media.remove",
            json!({ "asset": asset, "remove_clips": with_clips }),
        ),
        Command::Media(MediaCommand::Move { assets, folder }) => write(
            cli,
            "media.move",
            object(vec![
                ("assets", Some(json!(assets))),
                ("folder", some(folder)),
            ]),
        ),
        Command::Media(MediaCommand::Folder { action, folder, to }) => write(
            cli,
            "media.folder",
            object(vec![
                ("action", Some(json!(action))),
                ("folder", Some(json!(folder))),
                ("to", some(to)),
            ]),
        ),
        Command::Media(MediaCommand::Relink { asset, file }) => {
            let bundle = bundle::bundle_path(cli.project.as_deref())?;
            let asset = asset
                .parse()
                .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("{asset}: {e}")))?;
            let args = fosscut_engine::import::media_relink_args(&asset, file, &bundle)?;
            write(cli, "media.relink", args)
        }
        Command::Media(MediaCommand::Replace { asset, with }) => write(
            cli,
            "media.replace",
            json!({ "asset": asset, "with": with }),
        ),
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
            matte_model,
            matte_every,
            matte_size,
        }) => {
            let mut options = fosscut_engine::analysis::Options::default();
            if let Some(name) = matte_model {
                options.matte.model = serde_json::from_value(json!(name)).map_err(|_| {
                    ApiError::new(
                        codes::INVALID_ARGUMENTS,
                        format!("--matte-model {name}: person or subject"),
                    )
                })?;
            }
            if let Some(every) = matte_every {
                options.matte.every = *every;
            }
            if let Some(size) = matte_size {
                options.matte.size = *size;
            }
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
            compound,
            track,
            start,
            source_in,
            duration,
            timeline,
        }) => {
            // A still image is as long as the preferences say unless told otherwise.
            let duration = match (duration, asset) {
                (Some(duration), _) => Some(json!(duration)),
                (None, Some(asset)) if is_image(cli, asset)? => Some(json!({
                    "seconds": format!("{}", preferences()?.editing.image_duration)
                })),
                (None, _) => None,
            };
            write(
                cli,
                "clip.add",
                object(vec![
                    ("asset", some(asset)),
                    ("compound", some(compound)),
                    ("track", some(track)),
                    ("start", some(start)),
                    ("source_in", some(source_in)),
                    ("duration", duration),
                    ("timeline", some(timeline)),
                ]),
            )
        }
        Command::Clip(ClipCommand::Split { clip, at, unlinked }) => write(
            cli,
            "clip.split",
            json!({ "clip": clip, "at": at, "linked": !unlinked }),
        ),
        Command::Clip(ClipCommand::Trim {
            clip,
            start,
            end,
            ripple,
            unlinked,
        }) => write(
            cli,
            "clip.trim",
            object(vec![
                ("clip", Some(json!(clip))),
                ("start", some(start)),
                ("end", some(end)),
                ("ripple", ripple.then(|| json!(true))),
                ("linked", unlinked.then(|| json!(false))),
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
            flow,
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
                        [(*blend, "blend"), (*flow, "flow"), (*nearest, "nearest")]
                            .into_iter()
                            .find_map(|(on, mode)| on.then(|| json!(mode))),
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
        Command::Clip(ClipCommand::Move {
            clip,
            start,
            track,
            unlinked,
        }) => write(
            cli,
            "clip.move",
            object(vec![
                ("clip", Some(json!(clip))),
                ("start", some(start)),
                ("track", some(track)),
                ("linked", unlinked.then(|| json!(false))),
            ]),
        ),
        Command::Clip(ClipCommand::Remove {
            clip,
            ripple,
            unlinked,
        }) => write(
            cli,
            "clip.remove",
            json!({ "clip": clip, "ripple": ripple, "linked": !unlinked }),
        ),
        Command::Clip(ClipCommand::Compound { clips, name }) => write(
            cli,
            "clip.compound",
            object(vec![("clips", Some(json!(clips))), ("name", some(name))]),
        ),
        Command::Clip(ClipCommand::Decompose { clip }) => {
            write(cli, "clip.decompose", json!({ "clip": clip }))
        }
        Command::Clip(ClipCommand::Group { clips }) => {
            write(cli, "clip.group", json!({ "clips": clips }))
        }
        Command::Clip(ClipCommand::Ungroup { clips }) => {
            write(cli, "clip.ungroup", json!({ "clips": clips }))
        }
        Command::Clip(ClipCommand::ExtractAudio { clip }) => {
            write(cli, "clip.extract_audio", json!({ "clip": clip }))
        }
        Command::Clip(ClipCommand::RestoreAudio { clip }) => {
            write(cli, "clip.restore_audio", json!({ "clip": clip }))
        }
        Command::Clip(ClipCommand::Replace {
            clip,
            asset,
            source_in,
        }) => write(
            cli,
            "clip.replace",
            object(vec![
                ("clip", Some(json!(clip))),
                ("asset", Some(json!(asset))),
                ("source_in", some(source_in)),
            ]),
        ),
        Command::Clip(ClipCommand::Align {
            clip,
            horizontal,
            vertical,
        }) => write(
            cli,
            "clip.align",
            object(vec![
                ("clip", Some(json!(clip))),
                ("horizontal", some(horizontal)),
                ("vertical", some(vertical)),
            ]),
        ),
        Command::Clip(ClipCommand::CopyAttributes {
            from,
            to,
            attributes,
        }) => write(
            cli,
            "clip.copy_attributes",
            json!({ "from": from, "to": to, "attributes": attributes }),
        ),
        Command::Clips(ClipsCommand::Copy { clips }) => {
            query(cli, "clips.copy", json!({ "clips": clips }))
        }
        Command::Clips(ClipsCommand::Paste { at, from, track }) => {
            let copied: Value = serde_json::from_str(&read_input(from)?)
                .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("--from: {e}")))?;
            // Accept the query's JSON output as is, or just its result.
            let clips = copied
                .get("result")
                .unwrap_or(&copied)
                .get("clips")
                .cloned()
                .ok_or_else(|| {
                    ApiError::new(
                        codes::INVALID_ARGUMENTS,
                        "--from: expected the output of `fosscut clips copy --json`",
                    )
                })?;
            write(
                cli,
                "clips.paste",
                object(vec![
                    ("clips", Some(clips)),
                    ("at", Some(json!(at))),
                    ("track", some(track)),
                ]),
            )
        }
        Command::Marker(MarkerCommand::Add { at, name, color }) => write(
            cli,
            "marker.add",
            object(vec![
                ("at", Some(json!(at))),
                ("name", some(name)),
                ("color", some(color)),
            ]),
        ),
        Command::Marker(MarkerCommand::Set {
            marker,
            at,
            name,
            color,
        }) => write(
            cli,
            "marker.set",
            object(vec![
                ("marker", Some(json!(marker))),
                ("at", some(at)),
                ("name", some(name)),
                ("color", some(color)),
            ]),
        ),
        Command::Marker(MarkerCommand::Remove { marker }) => {
            write(cli, "marker.remove", json!({ "marker": marker }))
        }
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
        Command::Text(command) => text(cli, command),
        Command::Library(command) => library(cli, command),
        Command::Multicam(command) => multicam(cli, command),
        Command::Clip(ClipCommand::Canvas {
            clips,
            blur,
            color,
            none,
        }) => {
            let fill = match (blur, color, none) {
                (Some(amount), _, _) => json!({ "kind": "blur", "amount": amount }),
                (None, Some(color), _) => json!({ "kind": "color", "color": color }),
                (None, None, true) => Value::Null,
                (None, None, false) => {
                    return Err(ApiError::new(
                        codes::INVALID_ARGUMENTS,
                        "give --blur, --color or --none",
                    ));
                }
            };
            write(cli, "clip.canvas", json!({ "clips": clips, "fill": fill }))
        }
        Command::Preferences { command } => preferences_command(command.as_ref()),
        Command::Speak {
            text,
            voice,
            speed,
            at,
            track,
            voices,
        } => {
            if *voices {
                return Ok(Outcome::Query(
                    "voices".into(),
                    json!({ "voices": fosscut_engine::voice::voices(&fosscut_engine::models_dir()) }),
                ));
            }
            let path = bundle::bundle_path(cli.project.as_deref())?;
            let snapshot = bundle::snapshot(&path)?;
            let spoken = fosscut_engine::voice::speak(
                &snapshot,
                &fosscut_engine::voice::SpeakRequest {
                    bundle: path.clone(),
                    models: fosscut_engine::models_dir(),
                    text: text.clone().unwrap_or_default(),
                    voice: voice.clone(),
                    speed: *speed,
                    language: None,
                    timeline: None,
                    track: track.as_deref().map(|t| parse_id(t, "track")).transpose()?,
                    start: at.as_ref().map(|t| json!(t)),
                },
            )
            .map_err(|e| engine_error(&e))?;
            let commands = spoken.calls.clone();
            let mut report = json!(spoken);
            if let Some(object) = report.as_object_mut() {
                object.remove("calls");
            }
            apply_at(
                cli,
                &path,
                "speak",
                "speech",
                report,
                commands,
                spoken.revision,
            )
        }
        Command::Cache(command) => cache_command(command),
        Command::Media(MediaCommand::Proxy { asset, size }) => {
            let path = bundle::bundle_path(cli.project.as_deref())?;
            let snapshot = bundle::snapshot(&path)?;
            let size = match size {
                Some(size) => {
                    fosscut_engine::proxy::ProxySize::parse(size).map_err(|e| engine_error(&e))?
                }
                None => preferences()?.proxy_size(),
            };
            let report = fosscut_engine::proxy::make_proxy(
                &snapshot,
                &path,
                &parse_id(asset, "asset")?,
                size,
                &fosscut_engine::cache_dir(),
                &mut progress_printer("making a proxy"),
            )
            .map_err(|e| engine_error(&e))?;
            Ok(Outcome::Query("media.proxy".into(), json!(report)))
        }
        Command::Media(MediaCommand::Remux { asset }) => {
            let path = bundle::bundle_path(cli.project.as_deref())?;
            let snapshot = bundle::snapshot(&path)?;
            let report = fosscut_engine::remux::remux(
                &snapshot,
                &path,
                &parse_id(asset, "asset")?,
                &fosscut_engine::cache_dir(),
                &mut progress_printer("remuxing"),
            )
            .map_err(|e| engine_error(&e))?;
            Ok(Outcome::Query("media.remux".into(), json!(report)))
        }
        Command::Update { from } => {
            let found = fosscut_engine::updates::check(env!("CARGO_PKG_VERSION"), from.as_deref())
                .map_err(|e| engine_error(&e))?;
            Ok(Outcome::Query("update.check".into(), json!(found)))
        }
        Command::Record {
            what,
            list,
            device,
            microphone,
            duration,
            out,
            size,
            fps,
            at,
            no_import,
            input,
            microphone_input,
        } => {
            if *list {
                let devices = fosscut_engine::record::devices().map_err(|e| engine_error(&e))?;
                return Ok(Outcome::Query(
                    "record.devices".into(),
                    json!({ "devices": devices }),
                ));
            }
            record(
                cli,
                &Recording {
                    what: what.as_deref().unwrap_or_default(),
                    device: device.clone(),
                    microphone: microphone.clone(),
                    duration: duration.as_ref(),
                    out: out.clone(),
                    size: size.as_deref(),
                    fps: fps.as_deref(),
                    at: at.as_ref(),
                    import: !no_import,
                    input: input.as_deref(),
                    microphone_input: microphone_input.as_deref(),
                },
            )
        }
        Command::Clip(ClipCommand::SplitScenes { clip }) => split_scenes(cli, clip),
        Command::Clip(ClipCommand::Stabilize { clip, strength }) => stabilize(cli, clip, *strength),
        Command::Clip(ClipCommand::Track {
            clip,
            region,
            at,
            mask,
            follow,
        }) => track_region(
            cli,
            clip,
            region,
            at.as_ref(),
            mask.as_deref(),
            follow.as_deref(),
        ),
        Command::Clip(ClipCommand::Reframe {
            clip,
            subject,
            framing,
        }) => reframe(cli, clip, None, subject, framing),
        Command::Clip(ClipCommand::CameraTrack {
            clip,
            zoom,
            subject,
            shots,
        }) => {
            let framing = FramingArgs {
                one_shot: !shots,
                max_zoom: None,
                max_speed: None,
            };
            reframe(cli, clip, Some(*zoom), subject, &framing)
        }
        Command::Timeline(TimelineCommand::Vertical {
            timeline,
            name,
            width,
            height,
            follow,
            framing,
        }) => vertical(
            cli,
            timeline.as_deref(),
            name.clone(),
            (*width, *height),
            follow,
            framing,
        ),
        Command::Beats(BeatsCommand::Markers { music, every, bpm }) => {
            beat_markers(cli, music, *every, *bpm)
        }
        Command::Beats(BeatsCommand::Cut {
            track,
            music,
            every,
            tolerance,
            bpm,
        }) => beat_cuts(cli, track, music, *every, tolerance.as_ref(), *bpm),
        Command::Shorts(ShortsCommand::Candidates {
            clip,
            min_seconds,
            max_seconds,
            count,
        }) => short_candidates(cli, clip, *min_seconds, *max_seconds, *count),
        Command::Shorts(ShortsCommand::Make {
            clip,
            from,
            to,
            name,
            width,
            height,
            follow,
            framing,
            no_captions,
            caption_preset,
            title,
        }) => make_short(
            cli,
            clip,
            (from, to),
            ShortLook {
                name: name.clone(),
                size: (*width, *height),
                follow,
                framing,
                captions: !no_captions,
                caption_preset: caption_preset.clone(),
                title: title.clone(),
            },
        ),
        Command::Audio(AudioCommand::Normalize {
            clips,
            target,
            ceiling,
            no_limit,
        }) => normalize(cli, clips, *target, *ceiling, *no_limit),
        Command::Timeline(TimelineCommand::Read { detail, timeline }) => query(
            cli,
            "timeline.read",
            object(vec![
                ("detail", Some(json!(detail.as_str()))),
                ("timeline", some(timeline)),
            ]),
        ),
        Command::Timeline(TimelineCommand::Add {
            name,
            assets,
            width,
            height,
            rate,
            primary,
        }) => write(
            cli,
            "timeline.add",
            object(vec![
                ("name", some(name)),
                ("assets", Some(json!(assets))),
                ("width", some(width)),
                ("height", some(height)),
                ("rate", some(rate)),
                ("primary", Some(json!(primary))),
            ]),
        ),
        Command::Timeline(TimelineCommand::Rename { name, timeline }) => write(
            cli,
            "timeline.rename",
            object(vec![
                ("name", Some(json!(name))),
                ("timeline", some(timeline)),
            ]),
        ),
        Command::Timeline(TimelineCommand::Primary { timeline }) => {
            write(cli, "timeline.set_primary", json!({ "timeline": timeline }))
        }
        Command::Timeline(TimelineCommand::Remove { timeline }) => {
            write(cli, "timeline.remove", json!({ "timeline": timeline }))
        }
        Command::Timeline(TimelineCommand::Split { at, tracks }) => {
            write(cli, "timeline.split", json!({ "at": at, "tracks": tracks }))
        }
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
            resolution,
            fps,
            bitrate,
            audio_bitrate,
            range,
            cover,
            subtitles,
        } => export(
            cli,
            output.as_deref(),
            preset.as_deref(),
            timeline.as_deref(),
            *presets,
            &ExportChoices {
                resolution: resolution.clone(),
                fps: fps.clone(),
                bitrate: bitrate.clone(),
                audio_bitrate: audio_bitrate.clone(),
                range: range.clone(),
                cover: cover.clone(),
                subtitles: subtitles.clone(),
            },
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
        Command::Audio(AudioCommand::Voice {
            preset,
            clip,
            replace,
        }) => write(
            cli,
            "voice.apply",
            json!({ "clip": clip, "preset": preset, "replace": replace }),
        ),
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
            clean_caches();
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
        } => {
            clean_caches();
            host(cli, *events, *exit_on_eof)
        }
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
/// The export options of `fosscut export`, as typed.
struct ExportChoices {
    resolution: Option<String>,
    fps: Option<String>,
    bitrate: Option<String>,
    audio_bitrate: Option<String>,
    range: Option<String>,
    cover: Option<TimeInput>,
    subtitles: Option<String>,
}

fn export(
    cli: &Cli,
    output: Option<&Path>,
    preset: Option<&str>,
    timeline: Option<&str>,
    list: bool,
    choices: &ExportChoices,
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
    let failed = |e: EngineError| engine_error(&e);
    let choice = fosscut_engine::SettingsChoice {
        resolution: choices
            .resolution
            .as_deref()
            .map(fosscut_engine::resolution)
            .transpose()
            .map_err(failed)?,
        rate: choices
            .fps
            .as_deref()
            .map(fosscut_engine::frame_rate)
            .transpose()
            .map_err(failed)?,
        video_bit_rate: choices
            .bitrate
            .as_deref()
            .map(fosscut_engine::bit_rate)
            .transpose()
            .map_err(failed)?,
        audio_bit_rate: choices
            .audio_bitrate
            .as_deref()
            .map(fosscut_engine::bit_rate)
            .transpose()
            .map_err(failed)?,
    };
    let rate = format.rate;
    let time = |input: &TimeInput| {
        input
            .resolve(rate)
            .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("time: {e}")))
    };
    let range = match choices.range.as_deref() {
        Some(text) => {
            let (from, to) = text.split_once("..").ok_or_else(|| {
                ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    format!("range {text}: expected START..END, for example 2s..10s"),
                )
            })?;
            let parse = |t: &str| {
                t.parse::<TimeInput>().map_err(|e| {
                    ApiError::new(codes::INVALID_ARGUMENTS, format!("range {text}: {e}"))
                })
            };
            Some((time(&parse(from)?)?, time(&parse(to)?)?))
        }
        None => None,
    };
    let options = fosscut_engine::ExportOptions {
        range,
        cover: choices.cover.as_ref().map(time).transpose()?,
        subtitles: choices
            .subtitles
            .as_deref()
            .map(fosscut_engine::SubtitleFormat::named)
            .transpose()
            .map_err(failed)?,
    };
    let request = ExportRequest {
        bundle: bundle.clone(),
        timeline,
        settings: fosscut_engine::settings_for(preset, format, &choice),
        output: output.to_path_buf(),
        allow_overwrite: cli.write.allow_destructive,
        expected_revision: cli.write.expected_revision,
        request_id: cli.write.request_id.clone(),
        cache: fosscut_engine::cache_dir(),
        options,
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

/// JSON given inline, as `@file` or `-` for stdin, for option `what`.
fn json_input(text: &Option<String>, what: &str) -> Result<Option<Value>, ApiError> {
    text.as_deref()
        .map(|text| {
            let text = read_input(text)?;
            serde_json::from_str::<Value>(&text)
                .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("{what}: {e}")))
        })
        .transpose()
}

/// The `reveal` argument of `text.add` and `text.set`, when one is asked for.
fn reveal_of(reveal: &RevealArgs) -> Option<Value> {
    let unit = reveal.reveal.as_ref()?;
    Some(object(vec![
        ("unit", Some(json!(unit))),
        ("motion", some(&reveal.reveal_motion)),
        (
            "duration",
            Some(json!(
                reveal
                    .reveal_duration
                    .clone()
                    .unwrap_or_else(|| { TimeInput::Seconds("1".into()) })
            )),
        ),
        ("exit", reveal.reveal_exit.then_some(json!(true))),
    ]))
}

fn text(cli: &Cli, command: &TextCommand) -> Result<Outcome, ApiError> {
    let lines = |text: &str| text.replace("\\n", "\n");
    match command {
        TextCommand::Add {
            text,
            track,
            start,
            duration,
            color,
            size,
            font,
            preset,
            style,
            spans,
            reveal,
            timeline,
        } => {
            // Single options are merged over the style patch.
            let mut patch = json_input(style, "style")?.unwrap_or_else(|| json!({}));
            for (key, value) in [
                ("color", some(color)),
                ("size", some(size)),
                ("font", some(font)),
            ] {
                if let Some(value) = value {
                    patch[key] = value;
                }
            }
            let patch = (patch != json!({})).then_some(patch);
            write(
                cli,
                "text.add",
                object(vec![
                    ("text", Some(json!(lines(text)))),
                    ("track", some(track)),
                    ("start", some(start)),
                    ("duration", some(duration)),
                    ("preset", some(preset)),
                    ("style", patch),
                    ("spans", json_input(spans, "spans")?),
                    ("reveal", reveal_of(reveal)),
                    ("timeline", some(timeline)),
                ]),
            )
        }
        TextCommand::Set {
            clip,
            text,
            preset,
            style,
            spans,
            reveal,
            no_reveal,
        } => {
            let reveal = if *no_reveal {
                Some(Value::Null)
            } else {
                reveal_of(reveal)
            };
            let mut args = object(vec![
                ("clip", Some(json!(clip))),
                ("text", text.as_deref().map(|t| json!(lines(t)))),
                ("preset", some(preset)),
                ("style", json_input(style, "style")?),
                ("spans", json_input(spans, "spans")?),
            ]);
            if let Some(reveal) = reveal {
                args["reveal"] = reveal;
            }
            write(cli, "text.set", args)
        }
        TextCommand::SavePreset {
            name,
            clip,
            style,
            replace,
        } => write(
            cli,
            "text.preset_save",
            object(vec![
                ("name", Some(json!(name))),
                ("clip", some(clip)),
                ("style", json_input(style, "style")?),
                ("replace", replace.then_some(json!(true))),
            ]),
        ),
        TextCommand::RemovePreset { name } => {
            write(cli, "text.preset_remove", json!({ "name": name }))
        }
        TextCommand::Template {
            template,
            lines: texts,
            track,
            start,
            duration,
            style,
            timeline,
        } => write(
            cli,
            "text.template",
            object(vec![
                ("template", Some(json!(template))),
                (
                    "lines",
                    (!texts.is_empty())
                        .then(|| json!(texts.iter().map(|t| lines(t)).collect::<Vec<_>>())),
                ),
                ("track", some(track)),
                ("start", some(start)),
                ("duration", some(duration)),
                ("style", json_input(style, "style")?),
                ("timeline", some(timeline)),
            ]),
        ),
    }
}

/// `fosscut library list` and `fosscut library add`.
fn library(cli: &Cli, command: &LibraryCommand) -> Result<Outcome, ApiError> {
    use fosscut_engine::library;
    match command {
        LibraryCommand::List { kind, search } => Ok(Outcome::Query(
            "library.list".into(),
            json!({ "items": library::search(kind.as_deref(), search.as_deref()) }),
        )),
        LibraryCommand::Add {
            name,
            kind,
            at,
            duration,
            track,
            clip,
            lines,
            media,
        } => {
            let item = library::find(name, kind.as_deref()).map_err(|e| engine_error(&e))?;
            if matches!(item.kind, "sound" | "music") {
                // Imported first (or found again), then placed.
                let path = bundle::bundle_path(cli.project.as_deref())?;
                let media_add =
                    library::sound_media(&path, &item.name).map_err(|e| engine_error(&e))?;
                // The import is the first write and takes the revision check and dry run;
                // placing follows on the revision it made.
                let mut import = Request::single("media.add", media_add);
                apply_options(cli, &mut import, true);
                import.request_id = None;
                let imported = bundle::execute(&path, import)?;
                if cli.write.dry_run {
                    return Ok(Outcome::Write(imported));
                }
                let asset = imported.results[0]["asset"].clone();
                let placed = object(vec![
                    ("asset", Some(asset)),
                    ("start", some(at)),
                    ("duration", some(duration)),
                    ("track", some(track)),
                    (
                        "free_track",
                        (at.is_some() && track.is_none()).then_some(json!(true)),
                    ),
                ]);
                let mut request = Request::single("clip.add", placed);
                apply_options(cli, &mut request, true);
                request.expected_revision = Some(imported.revision);
                return execute(cli, request);
            }
            let placement = library::Placement {
                start: at.as_ref().map(|t| json!(t)),
                duration: duration.as_ref().map(|t| json!(t)),
                track: track.clone(),
                clip: clip.clone(),
                lines: lines.iter().map(|l| l.replace("\\n", "\n")).collect(),
                media: media.clone(),
            };
            let call = library::command_for(&item, &placement).map_err(|e| engine_error(&e))?;
            write(
                cli,
                call["command"].as_str().unwrap_or_default(),
                call["args"].clone(),
            )
        }
    }
}

/// `fosscut audio normalize`: measures the clips and sets their volumes in one change.
fn normalize(
    cli: &Cli,
    clips: &[String],
    target: Option<f64>,
    ceiling: Option<f64>,
    no_limit: bool,
) -> Result<Outcome, ApiError> {
    use fosscut_engine::loudness;
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let ids = clips
        .iter()
        .map(|c| {
            Id::parse(c).map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("clip: {e}")))
        })
        .collect::<Result<Vec<Id>, ApiError>>()?;
    let measured = loudness::normalize(
        &snapshot,
        &path,
        &ids,
        match target {
            Some(target) => target,
            None => preferences()?.editing.loudness_target,
        },
        ceiling.unwrap_or(loudness::DEFAULT_CEILING_DBFS),
        !no_limit,
    )
    .map_err(|e| engine_error(&e))?;
    let response = if measured.commands.is_empty() {
        None
    } else {
        let mut request: Request = serde_json::from_value(json!({ "commands": measured.commands }))
            .map_err(|e| ApiError::new(codes::REQUEST_INVALID, e.to_string()))?;
        apply_options(cli, &mut request, true);
        Some(bundle::execute(&path, request)?)
    };
    Ok(Outcome::Query(
        "audio.normalize".to_string(),
        json!({ "normalization": measured, "response": response }),
    ))
}

/// Like [`apply_measured`], for calls that predict ids from the project at `revision`
/// (imports followed by placements): they apply only to that revision, unless the command
/// line gave another one.
fn apply_at(
    cli: &Cli,
    path: &Path,
    name: &str,
    key: &str,
    report: Value,
    commands: Vec<Value>,
    revision: u64,
) -> Result<Outcome, ApiError> {
    let response = if commands.is_empty() {
        None
    } else {
        let mut request: Request = serde_json::from_value(json!({ "commands": commands }))
            .map_err(|e| ApiError::new(codes::REQUEST_INVALID, e.to_string()))?;
        apply_options(cli, &mut request, true);
        request.expected_revision = request.expected_revision.or(Some(revision));
        Some(bundle::execute(path, request)?)
    };
    Ok(Outcome::Query(
        name.to_string(),
        json!({ key: report, "response": response }),
    ))
}

/// Applies the calls a measurement proposed in one change (none when it proposed none)
/// and reports the measurement under `key` with the response.
fn apply_measured(
    cli: &Cli,
    path: &Path,
    name: &str,
    key: &str,
    report: Value,
    commands: Vec<Value>,
) -> Result<Outcome, ApiError> {
    let response = if commands.is_empty() {
        None
    } else {
        let mut request: Request = serde_json::from_value(json!({ "commands": commands }))
            .map_err(|e| ApiError::new(codes::REQUEST_INVALID, e.to_string()))?;
        apply_options(cli, &mut request, true);
        Some(bundle::execute(path, request)?)
    };
    Ok(Outcome::Query(
        name.to_string(),
        json!({ key: report, "response": response }),
    ))
}

fn parse_id(text: &str, what: &str) -> Result<Id, ApiError> {
    Id::parse(text).map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("{what}: {e}")))
}

/// `fosscut clip split-scenes`: measures the shot cuts and splits the clip at them.
fn split_scenes(cli: &Cli, clip: &str) -> Result<Outcome, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let found = fosscut_engine::scenes::split_at_scenes(
        &snapshot,
        path.clone(),
        &parse_id(clip, "clip")?,
        fosscut_engine::cache_dir(),
        fosscut_engine::models_dir(),
        &mut progress_printer("finding scenes"),
    )
    .map_err(|e| engine_error(&e))?;
    let commands = found.commands.clone();
    apply_measured(
        cli,
        &path,
        "clip.split_scenes",
        "scenes",
        json!(found),
        commands,
    )
}

/// `fosscut clip stabilize`: measures the camera's motion and keys the clip to steady it.
fn stabilize(cli: &Cli, clip: &str, strength: f64) -> Result<Outcome, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let plan = fosscut_engine::motion::stabilize(
        &snapshot,
        &path,
        &parse_id(clip, "clip")?,
        strength,
        &mut progress_printer("measuring motion"),
    )
    .map_err(|e| engine_error(&e))?;
    let commands = plan.commands.clone();
    apply_measured(
        cli,
        &path,
        "clip.stabilize",
        "motion",
        motion_report(&plan),
        commands,
    )
}

/// Like [`apply_measured`], at the revision the calls were planned at unless
/// --expected-revision says otherwise: the ids a plan reports are the ones its calls make
/// only there.
fn apply_planned(
    cli: &Cli,
    path: &Path,
    (name, key): (&str, &str),
    revision: u64,
    report: Value,
    commands: Vec<Value>,
) -> Result<Outcome, ApiError> {
    let mut request: Request = serde_json::from_value(json!({ "commands": commands }))
        .map_err(|e| ApiError::new(codes::REQUEST_INVALID, e.to_string()))?;
    request.expected_revision = Some(revision);
    apply_options(cli, &mut request, true);
    let response = bundle::execute(path, request)?;
    Ok(Outcome::Query(
        name.to_string(),
        json!({ key: report, "response": response }),
    ))
}

/// The timeline rate of the timeline holding `clip`.
fn clip_rate(project: &Project, clip: &Id) -> Result<Rate, ApiError> {
    project
        .timelines
        .values()
        .find(|t| {
            t.tracks
                .iter()
                .any(|tr| tr.clips.iter().any(|c| &c.id == clip))
        })
        .map(|t| t.format.rate)
        .ok_or_else(|| ApiError::new(codes::CLIP_NOT_FOUND, format!("clip {clip} does not exist")))
}

fn resolve_at(input: &TimeInput, rate: Rate, what: &str) -> Result<fosscut_time::Flicks, ApiError> {
    input
        .resolve(rate)
        .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("{what}: {e}")))
}

/// What `--follow` names, for planners that cannot follow a region.
fn followed(name: &str) -> Result<fosscut_engine::reframe::SubjectSource, ApiError> {
    use fosscut_engine::reframe::SubjectSource;
    match name {
        "person" => Ok(SubjectSource::Person),
        "any" => Ok(SubjectSource::Subject),
        other => Err(ApiError::new(
            codes::INVALID_ARGUMENTS,
            format!("--follow {other}: use person or any"),
        )),
    }
}

fn framing_options(framing: &FramingArgs) -> fosscut_engine::reframe::FramingOptions {
    let defaults = fosscut_engine::reframe::FramingOptions::default();
    fosscut_engine::reframe::FramingOptions {
        max_zoom: framing.max_zoom.unwrap_or(defaults.max_zoom),
        max_speed: framing.max_speed.unwrap_or(defaults.max_speed),
        ..defaults
    }
}

fn reframe_request(
    path: &Path,
    source: fosscut_engine::reframe::SubjectSource,
    framing: &FramingArgs,
) -> fosscut_engine::reframe::ReframeRequest {
    fosscut_engine::reframe::ReframeRequest {
        bundle: path.to_path_buf(),
        cache: fosscut_engine::cache_dir(),
        source,
        framing: framing_options(framing),
        shots: !framing.one_shot,
    }
}

/// `fosscut clip reframe` and `fosscut clip camera-track` (with `zoom`).
fn reframe(
    cli: &Cli,
    clip: &str,
    zoom: Option<f64>,
    subject: &SubjectArgs,
    framing: &FramingArgs,
) -> Result<Outcome, ApiError> {
    use fosscut_engine::reframe::{self, Region, SubjectSource};
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let clip = parse_id(clip, "clip")?;
    let source = match &subject.region {
        Some(region) => {
            let [x, y, width, height] = region[..] else {
                return Err(ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    "--region takes four numbers: X,Y,WIDTH,HEIGHT",
                ));
            };
            let rate = clip_rate(&snapshot, &clip)?;
            SubjectSource::Region {
                region: Region {
                    x,
                    y,
                    width,
                    height,
                },
                at: subject
                    .at
                    .as_ref()
                    .map(|at| resolve_at(at, rate, "--at"))
                    .transpose()?,
            }
        }
        None => followed(&subject.follow)?,
    };
    let request = reframe_request(&path, source, framing);
    let mut progress = progress_printer("framing");
    let (name, plan) = match zoom {
        Some(zoom) => (
            "clip.camera_track",
            reframe::camera_track(&snapshot, &clip, zoom, &request, &mut progress),
        ),
        None => (
            "clip.reframe",
            reframe::reframe(&snapshot, &clip, &request, &mut progress),
        ),
    };
    let plan = plan.map_err(|e| engine_error(&e))?;
    let commands = plan.commands.clone();
    apply_measured(cli, &path, name, "framing", motion_report(&plan), commands)
}

/// `fosscut timeline vertical`.
fn vertical(
    cli: &Cli,
    timeline: Option<&str>,
    name: Option<String>,
    (width, height): (u32, u32),
    follow: &str,
    framing: &FramingArgs,
) -> Result<Outcome, ApiError> {
    use fosscut_engine::reframe::{self, VerticalRequest};
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let timeline = timeline.map(|t| parse_id(t, "timeline")).transpose()?;
    let plan = reframe::vertical(
        &snapshot,
        timeline.as_ref(),
        &VerticalRequest {
            reframe: reframe_request(&path, followed(follow)?, framing),
            width,
            height,
            name,
        },
        &mut progress_printer("framing"),
    )
    .map_err(|e| engine_error(&e))?;
    let commands = plan.commands.clone();
    apply_planned(
        cli,
        &path,
        ("timeline.vertical", "vertical"),
        snapshot.manifest.revision,
        motion_report(&plan),
        commands,
    )
}

fn beat_context(path: &Path, bpm: Option<f64>) -> fosscut_engine::beats::BeatContext {
    use fosscut_engine::beats::{BeatContext, BeatOptions};
    BeatContext {
        bundle: path.to_path_buf(),
        cache: fosscut_engine::cache_dir(),
        models: fosscut_engine::models_dir(),
        options: BeatOptions {
            bpm,
            ..BeatOptions::default()
        },
    }
}

/// `fosscut beats markers`.
fn beat_markers(cli: &Cli, music: &str, every: u32, bpm: Option<f64>) -> Result<Outcome, ApiError> {
    use fosscut_engine::beats::{self, BeatSource};
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let found = beats::markers(
        &snapshot,
        &beat_context(&path, bpm),
        &BeatSource::Clip(parse_id(music, "music")?),
        every,
        &mut progress_printer("finding beats"),
    )
    .map_err(|e| engine_error(&e))?;
    let commands = found.commands.clone();
    apply_measured(
        cli,
        &path,
        "beats.markers",
        "beats",
        motion_report(&found),
        commands,
    )
}

/// `fosscut beats cut`.
fn beat_cuts(
    cli: &Cli,
    track: &str,
    music: &str,
    every: u32,
    tolerance: Option<&TimeInput>,
    bpm: Option<f64>,
) -> Result<Outcome, ApiError> {
    use fosscut_engine::beats::{self, CutOptions};
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let music = parse_id(music, "music")?;
    let rate = clip_rate(&snapshot, &music)?;
    let found = beats::cut_to_beats(
        &snapshot,
        &beat_context(&path, bpm),
        &parse_id(track, "track")?,
        &music,
        &CutOptions {
            every,
            tolerance: tolerance
                .map(|t| resolve_at(t, rate, "--tolerance"))
                .transpose()?,
            ..CutOptions::default()
        },
        &mut progress_printer("finding beats"),
    )
    .map_err(|e| engine_error(&e))?;
    let commands = found.commands.clone();
    apply_measured(
        cli,
        &path,
        "beats.cut",
        "beats",
        motion_report(&found),
        commands,
    )
}

/// `fosscut shorts candidates`.
fn short_candidates(
    cli: &Cli,
    clip: &str,
    min_seconds: f64,
    max_seconds: f64,
    count: u32,
) -> Result<Outcome, ApiError> {
    use fosscut_engine::shorts::{self, HighlightOptions, ShortsContext};
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let found = shorts::candidates(
        &snapshot,
        &ShortsContext {
            bundle: path.clone(),
            cache: fosscut_engine::cache_dir(),
        },
        &parse_id(clip, "clip")?,
        &HighlightOptions {
            min_seconds,
            max_seconds,
            count,
            ..HighlightOptions::default()
        },
    )
    .map_err(|e| engine_error(&e))?;
    Ok(Outcome::Query(
        "shorts.candidates".to_string(),
        json!(found),
    ))
}

/// How `fosscut shorts make` builds a short.
struct ShortLook<'a> {
    name: Option<String>,
    size: (u32, u32),
    follow: &'a str,
    framing: &'a FramingArgs,
    captions: bool,
    caption_preset: Option<String>,
    title: Option<String>,
}

/// `fosscut shorts make`.
fn make_short(
    cli: &Cli,
    clip: &str,
    (from, to): (&TimeInput, &TimeInput),
    look: ShortLook<'_>,
) -> Result<Outcome, ApiError> {
    use fosscut_engine::shorts::{self, ShortRequest, ShortSource, ShortsContext};
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let clip = parse_id(clip, "clip")?;
    let rate = clip_rate(&snapshot, &clip)?;
    let mut request = ShortRequest::new(
        ShortSource::Clip(clip),
        resolve_at(from, rate, "--from")?,
        resolve_at(to, rate, "--to")?,
    );
    request.name = look.name;
    (request.width, request.height) = look.size;
    request.follow = match look.follow {
        "centre" | "center" => None,
        other => Some(followed(other)?),
    };
    request.framing = framing_options(look.framing);
    request.shots = !look.framing.one_shot;
    request.captions = look.captions;
    request.caption_preset = look.caption_preset;
    request.title = look.title;
    let plan = shorts::make_short(
        &snapshot,
        &ShortsContext {
            bundle: path.clone(),
            cache: fosscut_engine::cache_dir(),
        },
        &request,
    )
    .map_err(|e| engine_error(&e))?;
    let commands = plan.commands.clone();
    apply_planned(
        cli,
        &path,
        ("shorts.make", "short"),
        snapshot.manifest.revision,
        motion_report(&plan),
        commands,
    )
}

/// A plan without its calls, which the response already shows applied.
fn motion_report(plan: &impl serde::Serialize) -> Value {
    let mut report = json!(plan);
    if let Some(object) = report.as_object_mut() {
        object.remove("commands");
    }
    report
}

/// `fosscut clip track`: follows a region and keys a mask or another clip to it.
fn track_region(
    cli: &Cli,
    clip: &str,
    region: &[f64],
    at: Option<&TimeInput>,
    mask: Option<&str>,
    follow: Option<&str>,
) -> Result<Outcome, ApiError> {
    use fosscut_engine::motion::{TrackTarget, track};
    let [x, y, width, height] = region else {
        return Err(ApiError::new(
            codes::INVALID_ARGUMENTS,
            "--region takes four numbers: X,Y,WIDTH,HEIGHT",
        ));
    };
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    let id = parse_id(clip, "clip")?;
    let target = match (mask, follow) {
        (Some(mask), _) => TrackTarget::Mask(parse_id(mask, "mask")?),
        (None, Some(other)) => TrackTarget::Clip(parse_id(other, "follow")?),
        (None, None) => {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "give --mask or --follow",
            ));
        }
    };
    let at = match at {
        Some(at) => resolve_time(&snapshot, &id, at)?,
        None => clip_start(&snapshot, &id)?,
    };
    let plan = track(
        &snapshot,
        &path,
        &id,
        fosscut_engine::motion::Region {
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        },
        at,
        &target,
        &mut progress_printer("tracking"),
    )
    .map_err(|e| engine_error(&e))?;
    let commands = plan.commands.clone();
    apply_measured(
        cli,
        &path,
        "clip.track",
        "motion",
        motion_report(&plan),
        commands,
    )
}

/// The timeline containing `clip`.
fn clip_timeline<'a>(
    project: &'a Project,
    clip: &Id,
) -> Result<(&'a fosscut_model::Timeline, &'a fosscut_model::Clip), ApiError> {
    project
        .timelines
        .values()
        .find_map(|t| {
            t.tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|c| &c.id == clip)
                .map(|c| (t, c))
        })
        .ok_or_else(|| ApiError::new(codes::CLIP_NOT_FOUND, format!("clip {clip} does not exist")))
}

fn clip_start(project: &Project, clip: &Id) -> Result<fosscut_time::Flicks, ApiError> {
    clip_timeline(project, clip).map(|(_, c)| c.start)
}

/// `at` in flicks, at the rate of the timeline holding `clip`.
fn resolve_time(
    project: &Project,
    clip: &Id,
    at: &TimeInput,
) -> Result<fosscut_time::Flicks, ApiError> {
    let (timeline, _) = clip_timeline(project, clip)?;
    at.resolve(timeline.format.rate)
        .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("--at: {e}")))
}

/// `fosscut multicam ...`
fn multicam(cli: &Cli, command: &MulticamCommand) -> Result<Outcome, ApiError> {
    match command {
        MulticamCommand::Create {
            angles,
            offsets,
            max_offset,
            no_sync,
            sound,
            name,
            start,
            track,
            timeline,
        } => {
            let path = bundle::bundle_path(cli.project.as_deref())?;
            let mut measured = None;
            let offsets: Vec<Value> = if !offsets.is_empty() {
                offsets.iter().map(|o| json!(o)).collect()
            } else if *no_sync {
                Vec::new()
            } else {
                let snapshot = bundle::snapshot(&path)?;
                let ids = angles
                    .iter()
                    .map(|a| parse_id(a, "angle"))
                    .collect::<Result<Vec<_>, _>>()?;
                let found = fosscut_engine::motion::sync_offsets(
                    &snapshot,
                    &path,
                    &ids,
                    max_offset.unwrap_or(0.0),
                )
                .map_err(|e| engine_error(&e))?;
                let offsets = found
                    .offsets
                    .iter()
                    .map(|o| json!({ "flicks": o.get() }))
                    .collect();
                measured = Some(found);
                offsets
            };
            let args = object(vec![
                ("angles", Some(json!(angles))),
                ("offsets", (!offsets.is_empty()).then(|| json!(offsets))),
                ("sound", sound.map(|s| json!(s))),
                ("name", some(name)),
                ("start", some(start)),
                ("track", some(track)),
                ("timeline", some(timeline)),
            ]);
            let command = json!({ "command": "multicam.create", "args": args });
            apply_measured(
                cli,
                &path,
                "multicam.create",
                "sync",
                json!(measured),
                vec![command],
            )
        }
        MulticamCommand::Switch { clip, at, angle } => {
            let angle = angle
                .parse::<usize>()
                .map_or_else(|_| json!(angle), |n| json!(n));
            write(
                cli,
                "multicam.switch",
                json!({ "clip": clip, "at": at, "angle": angle }),
            )
        }
        MulticamCommand::Sync { angles, max_offset } => {
            let path = bundle::bundle_path(cli.project.as_deref())?;
            let snapshot = bundle::snapshot(&path)?;
            let ids = angles
                .iter()
                .map(|a| parse_id(a, "angle"))
                .collect::<Result<Vec<_>, _>>()?;
            let found = fosscut_engine::motion::sync_offsets(
                &snapshot,
                &path,
                &ids,
                max_offset.unwrap_or(0.0),
            )
            .map_err(|e| engine_error(&e))?;
            Ok(Outcome::Query("multicam.sync".into(), json!(found)))
        }
    }
}

/// A frame rate as people write it: a whole number, a ratio such as 30000/1001, or the
/// usual decimal shorthands 23.976, 29.97 and 59.94.
fn rate_of(text: &str) -> Result<Rate, ApiError> {
    let exact = match text.trim() {
        "23.976" | "23.98" => "24000/1001",
        "29.97" => "30000/1001",
        "59.94" => "60000/1001",
        other => other,
    };
    Ok(exact.parse::<Rate>()?)
}

/// Whether `asset` of the project is a still image.
fn is_image(cli: &Cli, asset: &str) -> Result<bool, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let snapshot = bundle::snapshot(&path)?;
    Ok(Id::parse(asset)
        .ok()
        .and_then(|id| snapshot.media.get(&id).map(|a| a.kind))
        == Some(fosscut_model::AssetKind::Image))
}

/// The user's preferences, or the defaults when there is no file yet.
fn preferences() -> Result<fosscut_engine::preferences::Preferences, ApiError> {
    fosscut_engine::preferences::Preferences::load(&fosscut_engine::preferences::path())
        .map_err(|e| engine_error(&e))
}

/// Cleans the caches by the preferences, as a host does when it starts (ADR-0032 item
/// 5); cleaning only saves space, so a failure does not stop the host.
fn clean_caches() {
    if let Ok(preferences) = preferences() {
        preferences
            .cache_policy(fosscut_engine::cache_dir())
            .enforce();
    }
}

/// `fosscut cache ...`
fn cache_command(command: &CacheCommand) -> Result<Outcome, ApiError> {
    use fosscut_engine::cache::{self, CacheKind};
    let dir = fosscut_engine::cache_dir();
    let value = match command {
        CacheCommand::Info => json!({ "dir": dir, "caches": cache::cache_info(&dir) }),
        CacheCommand::Clear { kinds } => {
            let kinds = kinds
                .iter()
                .map(|kind| CacheKind::parse(kind).map_err(|e| engine_error(&e)))
                .collect::<Result<Vec<_>, _>>()?;
            json!({ "freed_bytes": cache::clear(&dir, &kinds) })
        }
        CacheCommand::Clean => json!(preferences()?.cache_policy(dir).enforce()),
    };
    Ok(Outcome::Query("cache".into(), value))
}

/// `fosscut preferences ...`
fn preferences_command(command: Option<&PreferencesCommand>) -> Result<Outcome, ApiError> {
    let file = fosscut_engine::preferences::path();
    let mut preferences = preferences()?;
    let save = |preferences: &fosscut_engine::preferences::Preferences| {
        preferences.save(&file).map_err(|e| engine_error(&e))
    };
    let value = match command {
        None => preferences.get("").map_err(|e| engine_error(&e))?,
        Some(PreferencesCommand::Get { key }) => {
            preferences.get(key).map_err(|e| engine_error(&e))?
        }
        Some(PreferencesCommand::Set { key, value }) => {
            preferences.set(key, value).map_err(|e| engine_error(&e))?;
            save(&preferences)?;
            json!({ key.as_str(): preferences.get(key).ok() })
        }
        Some(PreferencesCommand::Reset { key }) => {
            preferences.set(key, "null").map_err(|e| engine_error(&e))?;
            save(&preferences)?;
            json!({ key.as_str(): preferences.get(key).ok() })
        }
        Some(PreferencesCommand::Path) => json!({ "path": file }),
    };
    Ok(Outcome::Query("preferences".into(), value))
}

/// What `fosscut record` was asked for.
struct Recording<'a> {
    what: &'a str,
    device: Option<String>,
    microphone: Option<String>,
    duration: Option<&'a TimeInput>,
    out: Option<PathBuf>,
    size: Option<&'a str>,
    fps: Option<&'a str>,
    at: Option<&'a TimeInput>,
    import: bool,
    input: Option<&'a str>,
    microphone_input: Option<&'a str>,
}

/// `FORMAT:INPUT` as a capture input.
fn capture_input(text: &str) -> Result<fosscut_engine::record::CaptureInput, ApiError> {
    let (format, input) = text.split_once(':').ok_or_else(|| {
        ApiError::new(
            codes::INVALID_ARGUMENTS,
            format!("{text}: give an input as FORMAT:INPUT, for example lavfi:sine"),
        )
    })?;
    Ok(fosscut_engine::record::CaptureInput {
        format: format.to_string(),
        input: input.to_string(),
        options: Vec::new(),
    })
}

/// `fosscut record`: records into the bundle's `recordings/` folder (or `--out`), then
/// imports the file and, with `--at`, places it.
fn record(cli: &Cli, asked: &Recording<'_>) -> Result<Outcome, ApiError> {
    use fosscut_engine::record::{self, RecordKind, RecordRequest};
    let what = RecordKind::parse(asked.what).ok_or_else(|| {
        ApiError::new(
            codes::INVALID_ARGUMENTS,
            format!("{}: record microphone, camera or screen", asked.what),
        )
    })?;
    let bundle = if asked.import || asked.out.is_none() {
        Some(bundle::bundle_path(cli.project.as_deref())?)
    } else {
        None
    };
    let output = match (&asked.out, &bundle) {
        (Some(out), _) => out.clone(),
        (None, Some(bundle)) => record::default_output(bundle, what, std::time::SystemTime::now()),
        (None, None) => {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "give --out, or a project to record into",
            ));
        }
    };
    let rate = Rate::FPS_30;
    let mut request = RecordRequest::new(what, output);
    request.device = asked.device.clone();
    request.microphone = asked.microphone.clone();
    request.input = asked.input.map(capture_input).transpose()?;
    request.microphone_input = asked.microphone_input.map(capture_input).transpose()?;
    request.duration = asked
        .duration
        .map(|d| d.resolve(rate))
        .transpose()
        .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("--duration: {e}")))?;
    request.rate = asked
        .fps
        .map(|fps| fosscut_engine::frame_rate(fps).map_err(|e| engine_error(&e)))
        .transpose()?;
    request.size = asked
        .size
        .map(|size| {
            size.split_once('x')
                .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
                .ok_or_else(|| {
                    ApiError::new(
                        codes::INVALID_ARGUMENTS,
                        format!("--size {size}: give WIDTHxHEIGHT, for example 1280x720"),
                    )
                })
        })
        .transpose()?;
    request.allow_overwrite = cli.write.allow_destructive;
    let plan = record::plan(&request).map_err(|e| engine_error(&e))?;
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Enter, or standard input closing, stops a recording; with a duration and no
    // terminal, standard input is left alone so a closed one does not stop it at once.
    if request.duration.is_none() || std::io::stdin().is_terminal() {
        let stopper = std::sync::Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            stopper.store(true, std::sync::atomic::Ordering::Relaxed);
        });
    }
    let terminal = std::io::stderr().is_terminal();
    eprintln!(
        "recording {}{} to {}; press Enter to stop",
        plan.device.as_deref().unwrap_or(what.name()),
        plan.microphone
            .as_deref()
            .map(|m| format!(" with {m}"))
            .unwrap_or_default(),
        plan.recording.output.display()
    );
    let mut shown = -1_i64;
    let report = record::record(&request, &stop, &mut |recorded| {
        let seconds = recorded.get() / 705_600_000;
        if seconds != shown {
            shown = seconds;
            if terminal {
                eprint!("\rrecorded {seconds} s");
            }
        }
    })
    .map_err(|e| engine_error(&e))?;
    if terminal {
        eprintln!();
    }
    let mut result = json!({ "recording": report });
    if let (true, Some(bundle)) = (asked.import, &bundle) {
        let file = PathBuf::from(&report.output);
        let mut add = Request::single(
            "media.add",
            fosscut_engine::import::media_add_args(&file, None, bundle)?,
        );
        apply_options(cli, &mut add, true);
        let imported = bundle::execute(bundle, add)?;
        let asset = imported
            .results
            .first()
            .and_then(|r| r["asset"].as_str().map(str::to_string));
        result["import"] = json!(imported);
        if let (Some(at), Some(asset)) = (asked.at, asset) {
            let mut place = Request::single(
                "clip.add",
                json!({ "asset": asset, "start": at, "free_track": true }),
            );
            apply_options(cli, &mut place, true);
            place.expected_revision = None;
            place.request_id = None;
            result["placed"] = json!(bundle::execute(bundle, place)?);
        }
    }
    Ok(Outcome::Query("record".into(), result))
}

fn captions(cli: &Cli, command: &CaptionsCommand) -> Result<Outcome, ApiError> {
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let optional_id =
        |text: &Option<String>, what: &str| text.as_deref().map(|t| parse_id(t, what)).transpose();
    match command {
        CaptionsCommand::Translate {
            to,
            mode,
            track,
            timeline,
        } => {
            let snapshot = bundle::snapshot(&path)?;
            let pair = match to.as_str() {
                "hu" => "en>hu",
                "en" => "hu>en",
                other => {
                    return Err(ApiError::new(
                        codes::INVALID_ARGUMENTS,
                        format!("--to {other}: translations go into hu or en"),
                    ));
                }
            };
            let mode = serde_json::from_value(json!(mode)).map_err(|_| {
                ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    format!("--mode {mode}: bilingual or replace"),
                )
            })?;
            let found = fosscut_engine::captions::translate(
                &snapshot,
                &fosscut_engine::captions::TranslateRequest {
                    models: fosscut_engine::models_dir(),
                    timeline: optional_id(timeline, "timeline")?,
                    track: optional_id(track, "track")?,
                    pair: pair.into(),
                    mode,
                },
            )
            .map_err(|e| engine_error(&e))?;
            let commands = found.calls.clone();
            let mut report = json!(found);
            if let Some(object) = report.as_object_mut() {
                object.remove("calls");
            }
            apply_at(
                cli,
                &path,
                "captions.translate",
                "translation",
                report,
                commands,
                found.revision,
            )
        }
        CaptionsCommand::Dub {
            voice,
            speed,
            translation,
            track,
            audio_track,
            timeline,
        } => {
            use fosscut_engine::voice::{DubRequest, DubText, dub};
            let snapshot = bundle::snapshot(&path)?;
            let found = dub(
                &snapshot,
                &DubRequest {
                    bundle: path.clone(),
                    models: fosscut_engine::models_dir(),
                    timeline: optional_id(timeline, "timeline")?,
                    track: optional_id(track, "track")?,
                    voice: voice.clone(),
                    speed: *speed,
                    language: None,
                    text: if *translation {
                        DubText::Translation
                    } else {
                        DubText::Words
                    },
                    audio_track: optional_id(audio_track, "audio track")?,
                },
            )
            .map_err(|e| engine_error(&e))?;
            let commands = found.calls.clone();
            let mut report = json!(found);
            if let Some(object) = report.as_object_mut() {
                object.remove("calls");
            }
            apply_at(
                cli,
                &path,
                "captions.dub",
                "dub",
                report,
                commands,
                found.revision,
            )
        }
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
            skip_fillers,
            lyrics,
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
                    skip_fillers: *skip_fillers,
                    lyrics: *lyrics,
                },
            )
            .map_err(|e| engine_error(&e))?;
            let mut paging = if *lyrics {
                fosscut_engine::captions::lyrics_paging()
            } else {
                json!({})
            };
            for (key, value) in [
                ("max_words", some(max_words)),
                ("max_lines", some(max_lines)),
            ] {
                if let Some(value) = value {
                    paging[key] = value;
                }
            }
            let preset = preset
                .clone()
                .or_else(|| lyrics.then(|| "karaoke".to_string()));
            write(
                cli,
                "captions.add",
                object(vec![
                    ("words", Some(json!(generated.words))),
                    ("timeline", Some(json!(generated.timeline))),
                    ("track", some(track)),
                    ("preset", some(&preset)),
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
            subtitles,
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
            let text = fosscut_engine::captions::SrtText::named(subtitles)
                .map_err(|e| engine_error(&e))?;
            let srt = fosscut_engine::captions::export_srt(
                &snapshot,
                timeline.as_ref(),
                track.as_ref(),
                text,
            )
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
            translation,
        } => write(
            cli,
            "captions.edit",
            object(vec![
                ("clip", Some(json!(clip))),
                ("text", some(&text.as_ref().map(|t| t.replace("\\n", "\n")))),
                ("emphasis", some(emphasis)),
                ("translation", some(translation)),
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
