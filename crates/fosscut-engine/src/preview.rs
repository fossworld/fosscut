//! The desktop preview (ADR-0008, preview graph; ADR-0010): frames of a timeline rendered on
//! demand into a window's texture, by the plan and renderer that exports and frame grabs
//! use, with each clip's media kept open from one frame to the next.

use std::path::Path;

use fosscut_model::{Id, Project};
use fosscut_render::plan::plan_frame;
use fosscut_render::{PreviewGraph, Renderer};
use fosscut_time::{Flicks, TimeReport};

use crate::EngineError;
use crate::frame::{Media, Still};

/// Renders preview frames. Keep one per window: it holds the decoders of every clip shown
/// so far, which stay valid across edits because imported media never changes.
#[derive(Default)]
pub struct Previewer {
    media: Media,
}

impl Previewer {
    /// A previewer with no media open yet.
    pub fn new() -> Previewer {
        Previewer::default()
    }

    /// The size frames of `timeline` render at to fit `max_size`, keeping the canvas
    /// aspect and never larger than the canvas.
    pub fn size(
        project: &Project,
        timeline: &Id,
        max_size: (u32, u32),
    ) -> Result<(u32, u32), EngineError> {
        Ok(graph(project, timeline, max_size)?.size)
    }

    /// The frame shown at `time`: the start of the frame containing it, held at the first
    /// or last frame outside the timeline. An empty timeline shows its first frame.
    pub fn shown_at(
        project: &Project,
        timeline: &Id,
        time: Flicks,
    ) -> Result<TimeReport, EngineError> {
        let doc = project
            .timelines
            .get(timeline)
            .ok_or_else(|| EngineError::TimelineNotFound {
                timeline: timeline.to_string(),
            })?;
        let rate = doc.format.rate;
        let last = rate.frame_at(doc.duration() - Flicks::new(1)).max(0);
        let frame = rate.frame_at(time.max(Flicks::ZERO)).min(last);
        Ok(TimeReport::new(rate.frames_to_flicks(frame), rate))
    }

    /// Renders the frame of `timeline` shown at `time` into `target`, a texture view of
    /// [`Previewer::size`] for `max_size` in `format`. Returns the frame shown.
    #[allow(clippy::too_many_arguments)]
    pub fn render_to(
        &mut self,
        renderer: &mut Renderer,
        project: &Project,
        bundle: &Path,
        timeline: &Id,
        time: Flicks,
        max_size: (u32, u32),
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) -> Result<TimeReport, EngineError> {
        let graph = graph(project, timeline, max_size)?;
        let at = Previewer::shown_at(project, timeline, time)?;
        let plan = plan_frame(project, timeline, at.flicks)?;
        self.media.open_for(project, bundle, &plan)?;
        renderer.render_to(
            &plan,
            self.media.frames(),
            target,
            format,
            graph.size,
            graph.transfer,
        )?;
        Ok(at)
    }

    /// Renders the same frame as [`Previewer::render_to`] and reads it back.
    pub fn grab(
        &mut self,
        renderer: &mut Renderer,
        project: &Project,
        bundle: &Path,
        timeline: &Id,
        time: Flicks,
        max_size: (u32, u32),
    ) -> Result<Still, EngineError> {
        let graph = graph(project, timeline, max_size)?;
        let at = Previewer::shown_at(project, timeline, time)?;
        let plan = plan_frame(project, timeline, at.flicks)?;
        self.media.open_for(project, bundle, &plan)?;
        let image =
            renderer.render_image(&plan, self.media.frames(), graph.size, graph.transfer)?;
        Ok(Still { at, image })
    }
}

fn graph(
    project: &Project,
    timeline: &Id,
    max_size: (u32, u32),
) -> Result<PreviewGraph, EngineError> {
    if max_size.0 == 0 || max_size.1 == 0 {
        return Err(EngineError::InvalidArguments {
            message: format!("a preview needs a size, not {}x{}", max_size.0, max_size.1),
        });
    }
    Ok(PreviewGraph::new(project, timeline, max_size)?)
}
