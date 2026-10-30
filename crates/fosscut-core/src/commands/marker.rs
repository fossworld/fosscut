//! Timeline markers: points to come back to.

use fosscut_model::{Color, Id, IdKind, Marker};
use fosscut_time::{TimeInput, TimeReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::util::{resolve, timeline_id, timeline_mut};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// A marker after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MarkerReport {
    /// Marker id.
    pub marker: Id,
    /// Its timeline.
    pub timeline: Id,
    /// Where it is.
    pub at: TimeReport,
    /// Its label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its color.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl MarkerReport {
    fn of(marker: &Marker, timeline: &Id, rate: fosscut_time::Rate) -> MarkerReport {
        MarkerReport {
            marker: marker.id.clone(),
            timeline: timeline.clone(),
            at: TimeReport::new(marker.at, rate),
            name: marker.name.clone(),
            color: marker.color.as_ref().map(|c| c.as_str().to_string()),
        }
    }
}

fn color(input: &Option<String>) -> Result<Option<Color>, ApiError> {
    input
        .as_deref()
        .map(|c| Color::parse(c).map_err(invalid))
        .transpose()
}

/// Where a marker lives: its timeline and index.
fn find(ctx: &Context<'_>, marker: &Id) -> Result<(Id, usize), ApiError> {
    ctx.project
        .timelines
        .iter()
        .find_map(|(id, t)| {
            t.markers
                .iter()
                .position(|m| &m.id == marker)
                .map(|i| (id.clone(), i))
        })
        .ok_or_else(|| {
            ApiError::new(
                codes::MARKER_NOT_FOUND,
                format!("marker {marker} does not exist"),
            )
            .with_hint("timeline.read lists each timeline's markers.")
        })
}

/// Adds a marker to a timeline.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkerAdd {
    /// Where; snapped to a frame.
    pub at: TimeInput,
    /// A label, such as `drop` or `cut here`.
    #[serde(default)]
    pub name: Option<String>,
    /// A color as `#RRGGBB`.
    #[serde(default)]
    pub color: Option<String>,
    /// The timeline; the primary one when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
}

impl Command for MarkerAdd {
    const NAME: &'static str = "marker.add";
    const SUMMARY: &'static str = "Add a marker to a timeline at a time, with an optional name and color: a beat, a cue, a note to come back to.";
    type Output = MarkerReport;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MarkerReport, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let id = ctx.project.allocate_id(IdKind::Marker);
        let color = color(&self.color)?;
        let doc = timeline_mut(ctx.project, &timeline)?;
        let rate = doc.format.rate;
        let at = resolve(&self.at, rate)?;
        if at.is_negative() {
            return Err(invalid("marker.add: a marker cannot be before zero"));
        }
        let marker = Marker {
            id,
            at,
            name: self.name.filter(|n| !n.trim().is_empty()),
            color,
        };
        let report = MarkerReport::of(&marker, &timeline, rate);
        let index = doc.markers.partition_point(|m| m.at <= at);
        doc.markers.insert(index, marker);
        Ok(report)
    }
}

/// Moves, renames or recolors a marker.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkerSet {
    /// The marker.
    pub marker: Id,
    /// A new time.
    #[serde(default)]
    pub at: Option<TimeInput>,
    /// A new label; an empty one removes it.
    #[serde(default)]
    pub name: Option<String>,
    /// A new color as `#RRGGBB`; an empty one returns to the default.
    #[serde(default)]
    pub color: Option<String>,
}

impl Command for MarkerSet {
    const NAME: &'static str = "marker.set";
    const SUMMARY: &'static str = "Move a marker, or change its name or color.";
    type Output = MarkerReport;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MarkerReport, ApiError> {
        if self.at.is_none() && self.name.is_none() && self.color.is_none() {
            return Err(invalid("marker.set: give at, name or color"));
        }
        let (timeline, index) = find(ctx, &self.marker)?;
        let doc = timeline_mut(ctx.project, &timeline)?;
        let rate = doc.format.rate;
        let mut marker = doc.markers.remove(index);
        if let Some(at) = &self.at {
            marker.at = resolve(at, rate)?;
            if marker.at.is_negative() {
                return Err(invalid("marker.set: a marker cannot be before zero"));
            }
        }
        if let Some(name) = self.name {
            marker.name = Some(name).filter(|n| !n.trim().is_empty());
        }
        if let Some(text) = &self.color {
            marker.color = if text.is_empty() {
                None
            } else {
                color(&self.color)?
            };
        }
        let report = MarkerReport::of(&marker, &timeline, rate);
        let index = doc.markers.partition_point(|m| m.at <= marker.at);
        doc.markers.insert(index, marker);
        Ok(report)
    }
}

/// Removes a marker.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkerRemove {
    /// The marker.
    pub marker: Id,
}

impl Command for MarkerRemove {
    const NAME: &'static str = "marker.remove";
    const SUMMARY: &'static str = "Remove a marker.";
    type Output = MarkerReport;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MarkerReport, ApiError> {
        let (timeline, index) = find(ctx, &self.marker)?;
        let doc = timeline_mut(ctx.project, &timeline)?;
        let rate = doc.format.rate;
        let marker = doc.markers.remove(index);
        Ok(MarkerReport::of(&marker, &timeline, rate))
    }
}
