//! `fosscut model ...`: 3D objects (ADR-0040). Model files (`.glb`, `.gltf`) are imported
//! with `fosscut media import`.

use clap::Subcommand;
use fosscut_core::{ApiError, codes};
use fosscut_time::TimeInput;
use serde_json::json;

use crate::args::Cli;
use crate::{Outcome, json_input, object, some, write};

fn parse_time(text: &str) -> Result<TimeInput, String> {
    text.parse()
        .map_err(|e: fosscut_time::TimeError| e.to_string())
}

/// `fosscut model ...`
#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    /// List the 3D object presets: a shipping box, city blocks, a podium, an extruded
    /// title, a neon ring and a chrome sphere.
    Presets,
    /// Add a 3D object on a track above the footage: a preset, an imported model asset,
    /// extruded text, a shape clip's outline extruded, or a light grey cube.
    Add {
        /// A preset (see `fosscut model presets`).
        preset: Option<String>,
        /// A model asset to show (import .glb or .gltf files with `fosscut media import`).
        #[arg(long)]
        asset: Option<String>,
        /// Text to extrude.
        #[arg(long)]
        text: Option<String>,
        /// A shape clip whose closed outline is extruded.
        #[arg(long)]
        shape_clip: Option<String>,
        /// Changes to the object as a JSON merge patch (inline, @file or -), for example
        /// '{"object": {"kind": "box", "bevel": 0.01}, "material": {"color": "#D9822B"},
        /// "floor": {}}'.
        #[arg(long)]
        set: Option<String>,
        /// Placement in 3D as JSON (inline, @file or -), for example '{"position_z": 0.3,
        /// "rotation_y": -25}'.
        #[arg(long)]
        transform: Option<String>,
        /// Timeline position, for example 2s; 0 when omitted.
        #[arg(long, value_parser = parse_time)]
        at: Option<TimeInput>,
        /// Length; 5 seconds when omitted.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        /// Video track id; a track of 3D objects above the footage when omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
    /// Change a 3D object; animate model.light.azimuth, model.emissive_strength,
    /// model.depth, model.growth and the other model.* settings with `fosscut keyframe set`.
    Set {
        /// The 3D object clip.
        #[arg(long)]
        clip: String,
        /// A preset whose object replaces the current one first.
        #[arg(long)]
        preset: Option<String>,
        /// A model asset to show instead.
        #[arg(long)]
        asset: Option<String>,
        /// New text to extrude.
        #[arg(long)]
        text: Option<String>,
        /// A shape clip whose closed outline is extruded instead.
        #[arg(long)]
        shape_clip: Option<String>,
        /// Changes as a JSON merge patch (inline, @file or -); null resets a field.
        #[arg(long)]
        set: Option<String>,
    },
}

/// Runs `fosscut model ...`.
pub fn model(cli: &Cli, command: &ModelCommand) -> Result<Outcome, ApiError> {
    match command {
        ModelCommand::Presets => Ok(Outcome::Query(
            "library.list".into(),
            json!({ "items": fosscut_engine::library::search(Some("model"), None) }),
        )),
        ModelCommand::Add {
            preset,
            asset,
            text,
            shape_clip,
            set,
            transform,
            at,
            duration,
            track,
            timeline,
        } => write(
            cli,
            "model.add",
            object(vec![
                ("preset", some(preset)),
                ("asset", some(asset)),
                ("text", some(text)),
                ("shape_clip", some(shape_clip)),
                ("set", json_input(set, "set")?),
                ("transform", json_input(transform, "transform")?),
                ("start", some(at)),
                ("duration", some(duration)),
                ("track", some(track)),
                ("timeline", some(timeline)),
            ]),
        ),
        ModelCommand::Set {
            clip,
            preset,
            asset,
            text,
            shape_clip,
            set,
        } => {
            let args = object(vec![
                ("clip", Some(json!(clip))),
                ("preset", some(preset)),
                ("asset", some(asset)),
                ("text", some(text)),
                ("shape_clip", some(shape_clip)),
                ("set", json_input(set, "set")?),
            ]);
            if args.as_object().is_some_and(|a| a.len() == 1) {
                return Err(ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    "model set: give --preset, --asset, --text, --shape-clip or --set",
                ));
            }
            write(cli, "model.set", args)
        }
    }
}
