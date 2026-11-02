//! `fosscut shape ...`, `fosscut clip follow` and `fosscut vector ...` (ADR-0036): shapes
//! with strokes, trims and SVG path data, clips moving along a shape's path, and SVG
//! drawings split into editable shape clips.

use clap::{Args, Subcommand};
use fosscut_core::{ApiError, CommandCall, Request, codes};
use fosscut_engine::project as bundle;
use fosscut_model::{ClipContent, Id};
use fosscut_time::TimeInput;
use serde_json::{Map, Value, json};

use crate::args::Cli;
use crate::{Outcome, apply_options, execute, object, some, write};

fn parse_time(text: &str) -> Result<TimeInput, String> {
    text.parse()
        .map_err(|e: fosscut_time::TimeError| e.to_string())
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// `fosscut shape ...`
#[derive(Debug, Subcommand)]
pub enum ShapeCommand {
    /// Draw a shape: a rectangle, ellipse, star or heart, a line or outline from SVG path
    /// data, or a library item, with a fill, a stroke (caps, joins, dashes) and trims.
    Add(Box<ShapeAddArgs>),
    /// Change a shape clip: its fill, stroke and trims, or any field by a JSON merge patch.
    Set(Box<ShapeSetArgs>),
}

/// What `fosscut shape set` changes.
#[derive(Debug, Args)]
pub struct ShapeSetArgs {
    /// The shape clip.
    #[arg(long)]
    clip: String,
    #[command(flatten)]
    look: Look,
    /// Any other change as a JSON merge patch of the shape, for example
    /// '{"roundness": 0.5}'.
    #[arg(long)]
    patch: Option<String>,
}

/// What `fosscut shape add` draws.
#[derive(Debug, Args)]
pub struct ShapeAddArgs {
    /// A library item to start from (fosscut library list --kind shape).
    #[arg(long)]
    item: Option<String>,
    /// rectangle, ellipse, star, heart or path.
    #[arg(long)]
    shape: Option<String>,
    /// SVG path data in canvas pixels, for example "M 200 800 C 600 200 1300 1000 1700
    /// 300": a path shape sized and placed where the data draws it. Open paths are lines
    /// without fill.
    #[arg(long, allow_hyphen_values = true)]
    path_data: Option<String>,
    /// Width and height as fractions of the canvas, for example 0.3,0.2.
    #[arg(long, value_delimiter = ',')]
    size: Option<Vec<f64>>,
    /// Corner rounding of a rectangle, 0 to 1.
    #[arg(long)]
    roundness: Option<f64>,
    /// Leave a path open: a line from its first point to its last.
    #[arg(long)]
    open: bool,
    #[command(flatten)]
    look: Look,
    /// Where its centre is, as fractions of the canvas from its centre, for example
    /// 0.2,-0.1.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    position: Option<Vec<f64>>,
    /// Video track id; a graphics track above the footage when omitted.
    #[arg(long)]
    track: Option<String>,
    /// Timeline id; the primary timeline when omitted.
    #[arg(long)]
    timeline: Option<String>,
    /// Timeline position; 0 when omitted.
    #[arg(long, value_parser = parse_time)]
    start: Option<TimeInput>,
    /// Length; 3 seconds when omitted.
    #[arg(long, value_parser = parse_time)]
    duration: Option<TimeInput>,
}

/// A shape's fill, stroke and trims, each optional.
#[derive(Debug, Args)]
pub struct Look {
    /// Fill colour as #RRGGBB or #RRGGBBAA.
    #[arg(long)]
    fill: Option<String>,
    /// No fill: the shape shows only its stroke.
    #[arg(long, conflicts_with = "fill")]
    no_fill: bool,
    /// Stroke colour as #RRGGBB or #RRGGBBAA; gives the shape a stroke.
    #[arg(long)]
    stroke: Option<String>,
    /// Stroke width as a fraction of the canvas height (0.01 when omitted).
    #[arg(long)]
    stroke_width: Option<f64>,
    /// Stroke gradient to this colour, across the shape's box.
    #[arg(long)]
    stroke_gradient: Option<String>,
    /// Direction of the stroke gradient in degrees: 0 runs left to right, 90 top to bottom.
    #[arg(long, allow_hyphen_values = true)]
    stroke_angle: Option<f64>,
    /// Ends of open paths and dashes: butt, round or square.
    #[arg(long)]
    cap: Option<String>,
    /// Corners: miter, round or bevel.
    #[arg(long)]
    join: Option<String>,
    /// Dash pattern: dash and gap lengths as fractions of the canvas height, for example
    /// 0.03,0.015.
    #[arg(long, value_delimiter = ',')]
    dash: Option<Vec<f64>>,
    /// Where the dash pattern starts, as a fraction of the canvas height.
    #[arg(long, allow_hyphen_values = true)]
    dash_offset: Option<f64>,
    /// Remove the stroke.
    #[arg(long, conflicts_with_all = ["stroke", "stroke_width"])]
    no_stroke: bool,
    /// The part of the outline that shows, START..END as fractions of its length, for
    /// example 0..0.5; 0..0 hides it before a draw_on animation.
    #[arg(long)]
    trim: Option<String>,
    /// Moves the visible part along the outline by this fraction, wrapping around.
    #[arg(long, allow_hyphen_values = true)]
    trim_offset: Option<f64>,
}

impl Look {
    /// The stroke fields given, as a JSON object; `None` when none is given.
    fn stroke(&self) -> Result<Option<Value>, ApiError> {
        let mut stroke = object(vec![
            ("color", some(&self.stroke)),
            ("width", some(&self.stroke_width)),
            ("cap", some(&self.cap)),
            ("join", some(&self.join)),
            ("dashes", some(&self.dash)),
            ("dash_offset", some(&self.dash_offset)),
        ]);
        if let Some(to) = &self.stroke_gradient {
            stroke["gradient"] = json!({ "to": to, "angle": self.stroke_angle.unwrap_or(0.0) });
        } else if self.stroke_angle.is_some() {
            return Err(invalid("--stroke-angle needs --stroke-gradient"));
        }
        Ok((stroke.as_object().is_some_and(|m| !m.is_empty())).then_some(stroke))
    }

    /// `trim_start` and `trim_end` from `START..END`.
    fn trims(&self) -> Result<Option<(f64, f64)>, ApiError> {
        let Some(text) = &self.trim else {
            return Ok(None);
        };
        text.split_once("..")
            .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)))
            .map(Some)
            .ok_or_else(|| {
                invalid(format!(
                    "--trim {text}: write START..END, for example 0.3..0.7"
                ))
            })
    }

    /// Adds the fill, stroke and trims given to `fields`; a stroke merges into the one the
    /// shape has.
    fn add_to(&self, fields: &mut Map<String, Value>) -> Result<(), ApiError> {
        if self.no_fill {
            fields.insert("fill".into(), Value::Null);
        } else if let Some(color) = &self.fill {
            fields.insert("fill".into(), json!({ "color": color }));
        }
        if self.no_stroke {
            fields.insert("stroke".into(), Value::Null);
        } else if let Some(stroke) = self.stroke()? {
            fields.insert("stroke".into(), stroke);
        }
        if let Some((start, end)) = self.trims()? {
            fields.insert("trim_start".into(), json!(start));
            fields.insert("trim_end".into(), json!(end));
        }
        if let Some(offset) = self.trim_offset {
            fields.insert("trim_offset".into(), json!(offset));
        }
        Ok(())
    }
}

/// Runs `fosscut shape ...`.
pub fn shape(cli: &Cli, command: &ShapeCommand) -> Result<Outcome, ApiError> {
    match command {
        ShapeCommand::Add(args) => {
            let mut fields = object(vec![
                ("item", some(&args.item)),
                ("shape", some(&args.shape)),
                ("d", some(&args.path_data)),
                ("size", some(&args.size)),
                ("roundness", some(&args.roundness)),
                ("track", some(&args.track)),
                ("timeline", some(&args.timeline)),
                ("start", some(&args.start)),
                ("duration", some(&args.duration)),
            ]);
            let Some(map) = fields.as_object_mut() else {
                return Err(invalid("shape add: no fields"));
            };
            if args.open {
                map.insert("closed".into(), json!(false));
            }
            args.look.add_to(map)?;
            if let Some(position) = &args.position {
                if position.len() != 2 {
                    return Err(invalid("--position takes X,Y"));
                }
                map.insert("transform".into(), json!({ "position": position }));
            }
            write(cli, "shape.add", fields)
        }
        ShapeCommand::Set(args) => {
            let ShapeSetArgs { clip, look, patch } = args.as_ref();
            let mut changes = match patch {
                Some(text) => serde_json::from_str(text)
                    .map_err(|e| invalid(format!("--patch is not a JSON object: {e}")))?,
                None => json!({}),
            };
            let Some(map) = changes.as_object_mut() else {
                return Err(invalid("--patch must be a JSON object"));
            };
            look.add_to(map)?;
            if map.is_empty() {
                return Err(invalid(
                    "give a change: --fill, --no-fill, --stroke, --trim or --patch",
                ));
            }
            write(cli, "shape.set", json!({ "clip": clip, "set": changes }))
        }
    }
}

/// `fosscut clip follow`: move a clip along the outline of a shape clip.
#[derive(Debug, Args)]
pub struct FollowArgs {
    /// The clip that moves.
    #[arg(long)]
    clip: String,
    /// The shape clip whose outline it follows, on the same timeline.
    #[arg(long)]
    path: Option<String>,
    /// How far along, 0 (the path's start) to 1 (its end).
    #[arg(long, allow_hyphen_values = true)]
    progress: Option<f64>,
    /// Travel along the path from FROM to TO over the clip's whole length, for example
    /// 0..1: keys follow.progress at the clip's start and end.
    #[arg(long, conflicts_with = "progress")]
    travel: Option<String>,
    /// true turns the clip with the path's direction.
    #[arg(long)]
    orient: Option<bool>,
    /// Offset from the point on the path, as fractions of the canvas, for example 0,-0.03.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    offset: Option<Vec<f64>>,
    /// Stop following.
    #[arg(long, conflicts_with_all = ["path", "progress", "travel", "orient", "offset"])]
    clear: bool,
}

/// Runs `fosscut clip follow`.
pub fn follow(cli: &Cli, args: &FollowArgs) -> Result<Outcome, ApiError> {
    if args.offset.as_ref().is_some_and(|o| o.len() != 2) {
        return Err(invalid("--offset takes X,Y"));
    }
    let mut fields = object(vec![
        ("clip", Some(json!(args.clip))),
        ("path", some(&args.path)),
        ("progress", some(&args.progress)),
        ("orient", some(&args.orient)),
        ("offset", some(&args.offset)),
    ]);
    if args.clear {
        fields["clear"] = json!(true);
    }
    let Some(travel) = &args.travel else {
        return write(cli, "clip.follow", fields);
    };
    let (from, to) = travel
        .split_once("..")
        .and_then(|(a, b)| Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?)))
        .ok_or_else(|| {
            invalid(format!(
                "--travel {travel}: write FROM..TO, for example 0..1"
            ))
        })?;
    // Keys at the clip's start and end, in one step with the follow.
    let path = bundle::bundle_path(cli.project.as_deref())?;
    let project = bundle::snapshot(&path)?;
    let id = Id::parse(&args.clip).map_err(|e| invalid(format!("--clip: {e}")))?;
    let clip = project
        .timelines
        .values()
        .find_map(|t| t.find_clip(&id).map(|(_, c)| c.clone()))
        .ok_or_else(|| ApiError::new(codes::CLIP_NOT_FOUND, format!("clip {id} does not exist")))?;
    if matches!(clip.content, ClipContent::Audio { .. }) {
        return Err(invalid(format!(
            "{id} is an audio clip, with no picture to move"
        )));
    }
    let keys = json!({
        "clip": args.clip,
        "property": "follow.progress",
        "keys": [
            { "offset": { "flicks": 0 }, "value": from },
            { "offset": { "flicks": clip.duration.get() }, "value": to },
        ],
    });
    let mut request = Request {
        commands: vec![
            CommandCall {
                command: "clip.follow".into(),
                args: fields,
            },
            CommandCall {
                command: "keyframe.set_all".into(),
                args: keys,
            },
        ],
        ..Request::single("clip.follow", Value::Null)
    };
    apply_options(cli, &mut request, true);
    execute(cli, request)
}

/// `fosscut vector ...`
#[derive(Debug, Subcommand)]
pub enum VectorCommand {
    /// Split an imported SVG drawing into editable shape clips, one per path with its fill,
    /// stroke and placement, in a compound clip; open its timeline to animate the parts.
    Explode {
        /// The vector asset (an imported .svg file).
        #[arg(long)]
        asset: String,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
        /// Video track id for the compound clip; a graphics track when omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline position; 0 when omitted.
        #[arg(long, value_parser = parse_time)]
        start: Option<TimeInput>,
        /// Length; 3 seconds when omitted.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        /// Name of the compound timeline; the asset's name when omitted.
        #[arg(long)]
        name: Option<String>,
    },
}

/// Runs `fosscut vector ...`; the paths are read from the file when the request runs.
pub fn vector(cli: &Cli, command: &VectorCommand) -> Result<Outcome, ApiError> {
    match command {
        VectorCommand::Explode {
            asset,
            timeline,
            track,
            start,
            duration,
            name,
        } => write(
            cli,
            "vector.explode",
            object(vec![
                ("asset", Some(json!(asset))),
                ("timeline", some(timeline)),
                ("track", some(track)),
                ("start", some(start)),
                ("duration", some(duration)),
                ("name", some(name)),
            ]),
        ),
    }
}
