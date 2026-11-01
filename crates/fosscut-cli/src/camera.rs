//! `fosscut camera ...`: cameras for clips placed in 3D (ADR-0037).

use clap::{Args, Subcommand};
use fosscut_core::ApiError;
use fosscut_time::TimeInput;
use serde_json::{Value, json};

use crate::args::Cli;
use crate::{Outcome, object, property_value, some, write};

fn parse_time(text: &str) -> Result<TimeInput, String> {
    text.parse()
        .map_err(|e: fosscut_time::TimeError| e.to_string())
}

/// `fosscut camera ...`
#[derive(Debug, Subcommand)]
pub enum CameraCommand {
    /// Add a camera clip: while it lasts, the timeline's 3D clips (transform.three_d) are
    /// seen through it. Without settings it frames the z = 0 plane exactly like 2D.
    Add {
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
        /// Video track id; a `Camera` track above the video tracks when omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline position, for example 0s; 0 when omitted.
        #[arg(long, value_parser = parse_time)]
        start: Option<TimeInput>,
        /// Length, for example 4s; to the timeline's end when omitted.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        #[command(flatten)]
        fields: CameraFields,
    },
    /// Change a camera clip's settings; animate them with `fosscut keyframe set --property
    /// camera.position_z` and the other camera.* properties.
    Set {
        /// The camera clip.
        #[arg(long)]
        clip: String,
        #[command(flatten)]
        fields: CameraFields,
    },
}

/// A camera's settings, each optional.
#[derive(Debug, Args)]
pub struct CameraFields {
    /// Field of view in degrees across the canvas width, 1 to 170 (40 by default).
    #[arg(long)]
    fov: Option<f64>,
    /// Where the camera is, as fractions of the canvas from its centre, for example 0.2,0.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    position: Option<Vec<f64>>,
    /// How far the camera has moved forward from where the z = 0 plane fills the frame, as
    /// a fraction of the canvas width: positive pushes in.
    #[arg(long, allow_hyphen_values = true)]
    position_z: Option<f64>,
    /// What the camera looks at, as fractions of the canvas from its centre.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    point_of_interest: Option<Vec<f64>>,
    /// Depth of the point of interest as a fraction of the canvas width.
    #[arg(long, allow_hyphen_values = true)]
    point_of_interest_z: Option<f64>,
    /// Degrees around the point of interest as YAW,PITCH: positive yaw moves the camera
    /// right, positive pitch up.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    orbit: Option<Vec<f64>>,
    /// Degrees the view turns around its axis; positive turns the picture clockwise.
    #[arg(long, allow_hyphen_values = true)]
    roll: Option<f64>,
    /// true blurs 3D clips by their distance from the plane in focus.
    #[arg(long)]
    depth_of_field: Option<bool>,
    /// How far the plane in focus lies beyond the point of interest, as a fraction of the
    /// canvas width.
    #[arg(long, allow_hyphen_values = true)]
    focus_offset: Option<f64>,
    /// Lens opening as a fraction of the canvas width, 0 to 1 (0.1 by default).
    #[arg(long)]
    aperture: Option<f64>,
    /// Largest blur radius as a fraction of the canvas width, 0 to 0.2 (0.02 by default).
    #[arg(long)]
    max_blur: Option<f64>,
    /// Any setting as NAME=VALUE, VALUE in JSON, for example `orbit=[30,10]`. Repeatable.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    settings: Vec<String>,
}

impl CameraFields {
    /// The settings given, as a camera object.
    fn camera(&self) -> Result<Value, ApiError> {
        let mut camera = object(vec![
            ("field_of_view", some(&self.fov)),
            ("position", some(&self.position)),
            ("position_z", some(&self.position_z)),
            ("point_of_interest", some(&self.point_of_interest)),
            ("point_of_interest_z", some(&self.point_of_interest_z)),
            ("orbit", some(&self.orbit)),
            ("roll", some(&self.roll)),
            ("depth_of_field", some(&self.depth_of_field)),
            ("focus_offset", some(&self.focus_offset)),
            ("aperture", some(&self.aperture)),
            ("max_blur", some(&self.max_blur)),
        ]);
        for setting in &self.settings {
            let (name, value) = property_value(setting)?;
            camera[name] = value;
        }
        Ok(camera)
    }
}

pub(crate) fn camera(cli: &Cli, command: &CameraCommand) -> Result<Outcome, ApiError> {
    match command {
        CameraCommand::Add {
            timeline,
            track,
            start,
            duration,
            fields,
        } => write(
            cli,
            "camera.add",
            object(vec![
                ("camera", Some(fields.camera()?)),
                ("timeline", some(timeline)),
                ("track", some(track)),
                ("start", some(start)),
                ("duration", some(duration)),
            ]),
        ),
        CameraCommand::Set { clip, fields } => write(
            cli,
            "camera.set",
            json!({ "clip": clip, "set": fields.camera()? }),
        ),
    }
}
