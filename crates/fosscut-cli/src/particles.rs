//! `fosscut particles ...`: particle emitters (ADR-0038).

use clap::Subcommand;
use fosscut_core::{ApiError, codes};
use fosscut_time::TimeInput;
use serde_json::{Value, json};

use crate::args::Cli;
use crate::{Outcome, json_input, object, some, write};

fn parse_time(text: &str) -> Result<TimeInput, String> {
    text.parse()
        .map_err(|e: fosscut_time::TimeError| e.to_string())
}

/// `COUNT@TIME`, such as `300@1s`: a burst of COUNT particles at TIME from the clip's start.
fn parse_burst(text: &str) -> Result<Value, String> {
    let (count, at) = text
        .split_once('@')
        .ok_or_else(|| format!("{text}: write COUNT@TIME, for example 300@1s"))?;
    let count: u32 = count
        .trim()
        .parse()
        .map_err(|_| format!("{text}: the count must be a whole number"))?;
    let at = parse_time(at.trim())?;
    Ok(json!({ "at": at, "count": count }))
}

/// `fosscut particles ...`
#[derive(Debug, Subcommand)]
pub enum ParticlesCommand {
    /// List the particle presets: sparks, embers, dust, snow, confetti, bokeh, light
    /// streaks, a burst, gathering sparks and sparkles.
    Presets,
    /// Add a particle emitter over the canvas on a track above the footage, from a preset
    /// or from the defaults (a gentle fountain of soft white dots).
    Add {
        /// A preset (see `fosscut particles presets`).
        preset: Option<String>,
        /// Timeline position, for example 2s; 0 when omitted.
        #[arg(long, value_parser = parse_time)]
        at: Option<TimeInput>,
        /// Length; 5 seconds when omitted.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        /// Video track id; a track of particle clips above the footage when omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
        /// Changes to the emitter as a JSON merge patch (inline, @file or -), for example
        /// `'{"rate": 80, "colors": ["#FFD27A", "#FF6A00"]}'`.
        #[arg(long)]
        set: Option<String>,
        /// A burst as COUNT@TIME from the clip's start, such as 300@0.5s (repeat the
        /// option); replaces the preset's bursts.
        #[arg(long = "burst", value_parser = parse_burst)]
        bursts: Vec<Value>,
        /// Emission before the clip starts, so it opens full, for example 3s.
        #[arg(long, value_parser = parse_time)]
        prewarm: Option<TimeInput>,
        /// Another seed gives other particles that look alike.
        #[arg(long)]
        seed: Option<u32>,
    },
    /// Change a particle clip's emitter; animate particles.rate, particles.speed,
    /// particles.emitter.position and the other particles.* settings with `fosscut keyframe
    /// set`.
    Set {
        /// The particle clip.
        #[arg(long)]
        clip: String,
        /// A preset whose emitter replaces the current one first.
        #[arg(long)]
        preset: Option<String>,
        /// Changes as a JSON merge patch (inline, @file or -); null resets a field.
        #[arg(long)]
        set: Option<String>,
        /// A burst as COUNT@TIME from the clip's start (repeat the option); replaces the
        /// clip's bursts.
        #[arg(long = "burst", value_parser = parse_burst)]
        bursts: Vec<Value>,
        /// Remove every burst.
        #[arg(long, conflicts_with = "bursts")]
        no_bursts: bool,
        /// Emission before the clip starts, for example 3s.
        #[arg(long, value_parser = parse_time)]
        prewarm: Option<TimeInput>,
        /// A new seed.
        #[arg(long)]
        seed: Option<u32>,
    },
}

/// Runs `fosscut particles ...`.
pub fn particles(cli: &Cli, command: &ParticlesCommand) -> Result<Outcome, ApiError> {
    match command {
        ParticlesCommand::Presets => Ok(Outcome::Query(
            "library.list".into(),
            json!({ "items": fosscut_engine::library::search(Some("particles"), None) }),
        )),
        ParticlesCommand::Add {
            preset,
            at,
            duration,
            track,
            timeline,
            set,
            bursts,
            prewarm,
            seed,
        } => write(
            cli,
            "particles.add",
            object(vec![
                ("preset", some(preset)),
                ("start", some(at)),
                ("duration", some(duration)),
                ("track", some(track)),
                ("timeline", some(timeline)),
                ("set", json_input(set, "set")?),
                ("bursts", (!bursts.is_empty()).then(|| json!(bursts))),
                ("prewarm", some(prewarm)),
                ("seed", some(seed)),
            ]),
        ),
        ParticlesCommand::Set {
            clip,
            preset,
            set,
            bursts,
            no_bursts,
            prewarm,
            seed,
        } => {
            let bursts = match (bursts.is_empty(), no_bursts) {
                (false, _) => Some(json!(bursts)),
                (true, true) => Some(json!([])),
                (true, false) => None,
            };
            let args = object(vec![
                ("clip", Some(json!(clip))),
                ("preset", some(preset)),
                ("set", json_input(set, "set")?),
                ("bursts", bursts),
                ("prewarm", some(prewarm)),
                ("seed", some(seed)),
            ]);
            if args.as_object().is_some_and(|a| a.len() == 1) {
                return Err(ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    "particles set: give --preset, --set, --burst, --no-bursts, --prewarm or --seed",
                ));
            }
            write(cli, "particles.set", args)
        }
    }
}
