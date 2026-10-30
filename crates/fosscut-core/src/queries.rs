//! Built-in read-only queries.

use fosscut_model::{ClipContent, Id, Project, Timeline};
use fosscut_time::{Rate, TimeReport};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::history::stacks;
use crate::{API_VERSION, ApiError, LogEntry, Registry, Request, codes, error_catalogue};

/// What a query can read.
pub(crate) struct QueryContext<'a> {
    pub project: &'a Project,
    pub log: &'a [LogEntry],
    pub registry: &'a Registry,
}

fn parse<A: for<'de> Deserialize<'de>>(name: &str, args: Value) -> Result<A, ApiError> {
    let args = if args.is_null() {
        Value::Object(Default::default())
    } else {
        args
    };
    serde_json::from_value(args)
        .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("{name}: {e}")))
}

/// How much `timeline.read` and `capabilities` return.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Detail {
    /// Names and counts only.
    Outline,
    /// The usual working view.
    #[default]
    Normal,
    /// Everything, including styles, transforms and schemas.
    Full,
}

/// Arguments of `capabilities`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapabilitiesArgs {
    /// `outline` lists names and summaries; `normal` and `full` include JSON Schemas.
    #[serde(default)]
    detail: Detail,
    /// Only these commands and queries, each with its JSON Schemas, for example
    /// `["clip.add", "text.add"]`: far smaller than every schema at once.
    #[serde(default)]
    names: Vec<String>,
}

/// Arguments of `project.info` and `media.list`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoArgs {}

/// Arguments of `timeline.read`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TimelineReadArgs {
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    timeline: Option<Id>,
    /// `outline`: tracks only; `normal`: clips with times; `full`: also styles and transforms.
    #[serde(default)]
    detail: Detail,
}

/// Arguments of `history.list`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryArgs {
    /// Number of most recent entries to return (default 20).
    #[serde(default)]
    limit: Option<usize>,
}

pub(crate) fn register(registry: &mut Registry) {
    registry.register_query::<CapabilitiesArgs>(
        "capabilities",
        "Commands, queries, their JSON Schemas, the request envelope, time formats and error codes.",
        capabilities,
    );
    registry.register_query::<NoArgs>(
        "project.info",
        "Project name, revision, timelines and undo state.",
        project_info,
    );
    registry.register_query::<NoArgs>(
        "media.list",
        "Assets in the media table with durations and stream details.",
        media_list,
    );
    registry.register_query::<TimelineReadArgs>(
        "timeline.read",
        "Agent-oriented view of a timeline: tracks and clips with frames, timecode and seconds.",
        timeline_read,
    );
    registry.register_query::<ClipsCopyArgs>(
        "clips.copy",
        "Copy clips as documents (media, effects, masks, keyframes, groups) to pass to clips.paste, in this project or another with the same media.",
        clips_copy,
    );
    registry.register_query::<HistoryArgs>(
        "history.list",
        "Most recent operation log entries, newest first.",
        history_list,
    );
}

fn capabilities(ctx: &QueryContext<'_>, args: Value) -> Result<Value, ApiError> {
    let args: CapabilitiesArgs = parse("capabilities", args)?;
    let unknown: Vec<&String> = args
        .names
        .iter()
        .filter(|n| ctx.registry.command(n).is_none() && ctx.registry.query(n).is_none())
        .collect();
    if !unknown.is_empty() {
        let shown: Vec<&str> = unknown.iter().map(|n| n.as_str()).collect();
        return Err(ApiError::new(
            codes::INVALID_ARGUMENTS,
            format!("no command or query named {}", shown.join(", ")),
        )
        .with_hint("capabilities with detail outline lists every name.")
        .with_details(json!({ "unknown": shown })));
    }
    // Named lookups always carry schemas and leave out everything else.
    let named = !args.names.is_empty();
    let wanted = |name: &str| !named || args.names.iter().any(|n| n == name);
    let brief = !named && args.detail == Detail::Outline;
    let commands: Vec<Value> = ctx
        .registry
        .commands()
        .filter(|c| wanted(c.name))
        .map(|c| {
            if brief {
                json!({ "name": c.name, "summary": c.summary })
            } else {
                json!({ "name": c.name, "summary": c.summary, "input_schema": c.input_schema, "output_schema": c.output_schema })
            }
        })
        .collect();
    let queries: Vec<Value> = ctx
        .registry
        .queries()
        .filter(|q| wanted(q.name))
        .map(|q| {
            if brief {
                json!({ "name": q.name, "summary": q.summary })
            } else {
                json!({ "name": q.name, "summary": q.summary, "input_schema": q.input_schema })
            }
        })
        .collect();
    let mut result = json!({
        "api_version": API_VERSION,
        "format_version": fosscut_model::FORMAT_VERSION,
        "commands": commands,
        "queries": queries,
    });
    if !brief && !named {
        result["request_schema"] =
            serde_json::to_value(schemars::schema_for!(Request)).unwrap_or_default();
        result["time_input_schema"] =
            serde_json::to_value(schemars::schema_for!(fosscut_time::TimeInput))
                .unwrap_or_default();
        result["errors"] = serde_json::to_value(error_catalogue()).unwrap_or_default();
    }
    if !named {
        let presets: Vec<Value> = crate::presets::caption_presets()
            .into_iter()
            .map(|p| {
                if brief {
                    json!({ "name": p.name, "description": p.description })
                } else {
                    json!(p)
                }
            })
            .collect();
        result["captions"] = json!({
            "presets": presets,
            "animators": crate::presets::caption_animators(brief),
        });
        // The closed property vocabulary (ADR-0011, item 3): what clip.set and
        // keyframe.set take.
        let properties: Vec<Value> = fosscut_model::clip_properties()
            .iter()
            .map(|p| {
                if brief {
                    json!({ "path": p.path, "animatable": p.animatable })
                } else {
                    json!(p)
                }
            })
            .collect();
        result["properties"] = json!(properties);
        // Effect types with their parameter schemas (ADR-0011, item 1), and the looks.
        let effects: Vec<Value> = fosscut_model::effect_types()
            .iter()
            .map(|e| {
                if brief {
                    json!({ "type": e.name, "title": e.title, "category": e.category, "summary": e.summary })
                } else {
                    json!(e)
                }
            })
            .collect();
        result["effects"] = json!(effects);
        let looks: Vec<Value> = crate::looks::looks()
            .into_iter()
            .map(|l| {
                if brief {
                    json!({ "name": l.name, "title": l.title, "description": l.description })
                } else {
                    json!(l)
                }
            })
            .collect();
        result["looks"] = json!(looks);
        let motion = crate::commands::motion_capabilities(brief);
        result["transitions"] = motion["transitions"].clone();
        result["animations"] = motion["animations"].clone();
        result["speed_presets"] = crate::commands::speed_capabilities(brief);
    }
    Ok(result)
}

fn primary_rate(project: &Project) -> Rate {
    project
        .primary_timeline()
        .map(|t| t.format.rate)
        .unwrap_or(Rate::FPS_30)
}

fn project_info(ctx: &QueryContext<'_>, args: Value) -> Result<Value, ApiError> {
    let _: NoArgs = parse("project.info", args)?;
    let project = ctx.project;
    let (undo, redo) = stacks(ctx.log);
    let timelines: Vec<Value> = project
        .manifest
        .timelines
        .iter()
        .filter_map(|entry| project.timelines.get(&entry.id))
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "format": t.format,
                "duration": TimeReport::new(t.duration(), t.format.rate),
                "tracks": t.tracks.len(),
                "clips": t.tracks.iter().map(|tr| tr.clips.len()).sum::<usize>(),
            })
        })
        .collect();
    Ok(json!({
        "id": project.manifest.id,
        "name": project.manifest.name,
        "format_version": project.manifest.format_version,
        "revision": project.manifest.revision,
        "primary_timeline": project.manifest.primary_timeline,
        "timelines": timelines,
        "assets": project.media.assets.len(),
        "can_undo": !undo.is_empty(),
        "can_redo": !redo.is_empty(),
    }))
}

fn media_list(ctx: &QueryContext<'_>, args: Value) -> Result<Value, ApiError> {
    let _: NoArgs = parse("media.list", args)?;
    let project = ctx.project;
    let rate = primary_rate(project);
    let assets: Vec<Value> = project
        .media
        .assets
        .iter()
        .map(|asset| {
            let used_by: Vec<&Id> = project
                .timelines
                .values()
                .flat_map(|t| t.tracks.iter().flat_map(|tr| tr.clips.iter()))
                .filter(|c| c.content.asset() == Some(&asset.id))
                .map(|c| &c.id)
                .collect();
            json!({
                "id": asset.id,
                "kind": asset.kind,
                "name": asset.name,
                "path": asset.path,
                "duration": asset.info.duration.map(|d| TimeReport::new(d, rate)),
                "video": asset.info.video,
                "audio": asset.info.audio,
                "used_by": used_by,
            })
        })
        .collect();
    Ok(json!({ "assets": assets }))
}

fn clip_view(
    project: &Project,
    timeline: &Timeline,
    clip: &fosscut_model::Clip,
    detail: Detail,
) -> Value {
    let rate = timeline.format.rate;
    let mut view = json!({
        "id": clip.id,
        "kind": clip.content.kind_name(),
        "start": TimeReport::new(clip.start, rate),
        "end": TimeReport::new(clip.end(), rate),
        "duration": TimeReport::new(clip.duration, rate),
    });
    match &clip.content {
        ClipContent::Video { asset, source_in } | ClipContent::Audio { asset, source_in } => {
            view["asset"] = json!(asset);
            view["asset_name"] = json!(project.media.get(asset).map(|a| a.name.clone()));
            view["source_in"] = json!(TimeReport::new(*source_in, rate));
            // Speed changes how much media the clip plays (ADR-0023 item 10).
            if let Some(speed) = &clip.speed {
                let mut shown = json!(speed);
                if !speed.curve.is_empty() {
                    shown["start_rate"] = json!(speed.rate_at(0.0));
                    shown["end_rate"] = json!(speed.rate_at(1.0));
                }
                view["speed"] = shown;
                view["source_used"] = json!(TimeReport::new(clip.source_length(), rate));
            }
        }
        ClipContent::Image { asset } => {
            view["asset"] = json!(asset);
            view["asset_name"] = json!(project.media.get(asset).map(|a| a.name.clone()));
        }
        ClipContent::Text(text) => {
            view["text"] = json!(text.text);
            if detail == Detail::Full {
                view["style"] = json!(text.style);
            }
        }
        ClipContent::Adjustment => {}
        ClipContent::Caption(caption) => {
            view["text"] = json!(caption.text());
            if detail == Detail::Full {
                // Word times on the timeline, so agents never add the clip's start.
                view["lines"] = caption
                    .lines
                    .iter()
                    .map(|line| {
                        line.words
                            .iter()
                            .map(|word| {
                                let mut w = json!({
                                    "text": word.text,
                                    "start": TimeReport::new(clip.start + word.start, rate),
                                    "end": TimeReport::new(clip.start + word.end, rate),
                                });
                                if word.emphasis {
                                    w["emphasis"] = json!(true);
                                }
                                w
                            })
                            .collect::<Value>()
                    })
                    .collect();
            } else {
                view["words"] = json!(caption.words().count());
            }
        }
    }
    if let ClipContent::Adjustment = clip.content {
        view["text"] = json!("Adjustment");
    }
    if detail == Detail::Full
        && let Some(video) = clip
            .content
            .asset()
            .and_then(|a| project.media.get(a))
            .and_then(|a| a.info.video.as_ref())
    {
        // The picture as displayed, after its rotation: what masks and placement measure.
        let turned = video.rotation.rem_euclid(180) == 90;
        view["picture"] = if turned {
            json!([video.height, video.width])
        } else {
            json!([video.width, video.height])
        };
    }
    if !clip.enabled {
        view["enabled"] = json!(false);
    }
    if let Some(group) = &clip.group {
        view["group"] = json!(group);
    }
    if clip.audio.detached {
        view["audio_detached"] = json!(true);
    }
    if let Some(t) = &clip.transition {
        view["transition"] = json!({
            "type": t.transition_type,
            "duration": TimeReport::new(t.duration, rate),
            "params": t.params,
        });
    }
    if !clip.animations.is_empty() && detail == Detail::Full {
        view["animations"] = json!(clip.animations);
    }
    if !clip.masks.is_empty() {
        view["masks"] = clip
            .masks
            .iter()
            .map(|m| {
                let mut mask = json!(m);
                if detail != Detail::Full {
                    return json!({ "id": m.id, "shape": m.shape });
                }
                if !m.keyframes.is_empty() {
                    let reports: serde_json::Map<String, Value> = m
                        .keyframes
                        .iter()
                        .map(|(name, keys)| {
                            let keys: Vec<Value> = keys
                                .iter()
                                .map(|k| {
                                    json!({
                                        "at": TimeReport::new(clip.start + k.time, rate),
                                        "offset": TimeReport::new(k.time, rate),
                                        "value": k.value,
                                        "interpolation": k.interpolation,
                                    })
                                })
                                .collect();
                            (name.clone(), json!(keys))
                        })
                        .collect();
                    mask["keyframes"] = Value::Object(reports);
                }
                mask
            })
            .collect();
    }
    if !clip.effects.is_empty() {
        view["effects"] = clip
            .effects
            .iter()
            .map(|e| {
                let mut effect = json!({ "id": e.id, "type": e.effect_type });
                if !e.enabled {
                    effect["enabled"] = json!(false);
                }
                if detail == Detail::Full {
                    effect["version"] = json!(e.version);
                    effect["params"] = json!(e.params);
                    if !e.keyframes.is_empty() {
                        effect["keyframes"] =
                            json!(crate::commands::effect_keyframe_reports(clip, e, rate));
                    }
                }
                effect
            })
            .collect();
    }
    if detail == Detail::Full {
        view["transform"] = json!(clip.transform);
        view["crop"] = json!(clip.crop);
        view["opacity"] = json!(clip.opacity);
        view["blend"] = json!(clip.blend);
        view["volume"] = json!(clip.volume);
        view["audio"] = json!(clip.audio);
        if !clip.keyframes.is_empty() {
            view["keyframes"] = json!(crate::commands::keyframe_reports(clip, rate));
        }
    }
    view
}

fn timeline_read(ctx: &QueryContext<'_>, args: Value) -> Result<Value, ApiError> {
    let args: TimelineReadArgs = parse("timeline.read", args)?;
    let project = ctx.project;
    let id = args
        .timeline
        .unwrap_or_else(|| project.manifest.primary_timeline.clone());
    let timeline = project.timelines.get(&id).ok_or_else(|| {
        ApiError::new(
            codes::TIMELINE_NOT_FOUND,
            format!("timeline {id} does not exist"),
        )
    })?;
    let rate = timeline.format.rate;
    let tracks: Vec<Value> = timeline
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let mut view = json!({
                "id": track.id,
                "kind": track.kind,
                "index": index,
                "clip_count": track.clips.len(),
                "end": TimeReport::new(track.end(), rate),
            });
            if let Some(name) = &track.name {
                view["name"] = json!(name);
            }
            for (flag, value) in [
                ("muted", track.muted),
                ("hidden", track.hidden),
                ("locked", track.locked),
            ] {
                if value {
                    view[flag] = json!(true);
                }
            }
            if args.detail == Detail::Full && track.kind == fosscut_model::TrackKind::Caption {
                view["caption_style"] = json!(track.caption_style.clone().unwrap_or_default());
            }
            if args.detail != Detail::Outline {
                view["clips"] = track
                    .clips
                    .iter()
                    .map(|c| clip_view(project, timeline, c, args.detail))
                    .collect();
            }
            view
        })
        .collect();
    let markers: Vec<Value> = timeline
        .markers
        .iter()
        .map(|m| {
            let mut marker = json!({ "id": m.id, "at": TimeReport::new(m.at, rate) });
            if let Some(name) = &m.name {
                marker["name"] = json!(name);
            }
            if let Some(color) = &m.color {
                marker["color"] = json!(color);
            }
            marker
        })
        .collect();
    let mut view = json!({
        "timeline": timeline.id,
        "name": timeline.name,
        "revision": project.manifest.revision,
        "format": timeline.format,
        "duration": TimeReport::new(timeline.duration(), rate),
        "tracks": tracks,
    });
    if !markers.is_empty() {
        view["markers"] = json!(markers);
    }
    Ok(view)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClipsCopyArgs {
    /// The clips to copy, from one or more tracks.
    clips: Vec<Id>,
}

/// The clipboard: clips as stored, for `clips.paste` here or in another project.
fn clips_copy(ctx: &QueryContext<'_>, args: Value) -> Result<Value, ApiError> {
    let args: ClipsCopyArgs = parse("clips.copy", args)?;
    if args.clips.is_empty() {
        return Err(ApiError::new(
            codes::INVALID_ARGUMENTS,
            "clips.copy: give the clips to copy",
        ));
    }
    let clips = crate::commands::copied(ctx.project, &args.clips)?;
    Ok(json!({ "clips": clips }))
}

fn history_list(ctx: &QueryContext<'_>, args: Value) -> Result<Value, ApiError> {
    let args: HistoryArgs = parse("history.list", args)?;
    let (_, redo) = stacks(ctx.log);
    let entries: Vec<Value> = ctx
        .log
        .iter()
        .rev()
        .take(args.limit.unwrap_or(20))
        .map(|e| {
            json!({
                "seq": e.seq,
                "kind": e.kind,
                "revision_before": e.revision_before,
                "revision_after": e.revision_after,
                "actor": e.actor,
                "reason": e.reason,
                "request_id": e.request_id,
                "commands": e.commands.iter().map(|c| c.command.clone()).collect::<Vec<_>>(),
                "target_seq": e.target_seq,
                "at_ms": e.at_ms,
                "changes": e.changes,
                "undone": redo.contains(&e.seq),
            })
        })
        .collect();
    Ok(json!({ "entries": entries }))
}
