//! Preview and export graphs over one frame evaluation (ADR-0008, item 3): both plan a
//! frame with [`plan_frame`] and render it with the same [`Renderer`], so they differ only
//! in how they are driven (resolution, which frames, where the pixels go).

use std::collections::HashMap;
use std::sync::Arc;

use fosscut_media::{VideoDecoder, VideoFrame};
use fosscut_model::{Id, Project};
use fosscut_time::Flicks;

use crate::RenderError;
use crate::color::TransferFunction;
use crate::gpu::{Image, Renderer};
use crate::lut::Lut3d;
use crate::plan::{SourceRef, plan_frame};

/// Supplies the decoded frame a layer needs.
pub trait FrameProvider {
    /// The frame of `source.asset` displayed at `source.time`.
    fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError>;

    /// The table of a LUT asset, for `color.lut` effects. Providers without LUTs fail.
    fn lut(&mut self, asset: &Id) -> Result<Arc<Lut3d>, RenderError> {
        Err(RenderError::InvalidLut {
            message: format!("no LUT is available for {asset}"),
        })
    }
}

/// Frames from one [`VideoDecoder`] per asset. Decoders must be `Send` so the provider can
/// live on a render thread.
#[derive(Default)]
pub struct DecoderFrames {
    decoders: HashMap<Id, Box<dyn VideoDecoder + Send>>,
    luts: HashMap<Id, Arc<Lut3d>>,
}

impl DecoderFrames {
    /// No decoders yet.
    pub fn new() -> DecoderFrames {
        DecoderFrames::default()
    }

    /// Uses `decoder` for `asset`'s frames.
    pub fn insert(&mut self, asset: Id, decoder: Box<dyn VideoDecoder + Send>) {
        self.decoders.insert(asset, decoder);
    }

    /// Uses `lut` as the table of LUT asset `asset`.
    pub fn insert_lut(&mut self, asset: Id, lut: Arc<Lut3d>) {
        self.luts.insert(asset, lut);
    }

    /// True when the table of `asset` is loaded.
    pub fn has_lut(&self, asset: &Id) -> bool {
        self.luts.contains_key(asset)
    }
}

impl FrameProvider for DecoderFrames {
    fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError> {
        let missing = |message: String| RenderError::MissingFrame {
            asset: source.asset.to_string(),
            message,
        };
        let decoder = self
            .decoders
            .get_mut(&source.asset)
            .ok_or_else(|| missing("no decoder is open for this asset".to_string()))?;
        decoder
            .frame_at(source.time)
            .map_err(|e| missing(format!("{e} ({})", e.code())))
    }

    fn lut(&mut self, asset: &Id) -> Result<Arc<Lut3d>, RenderError> {
        self.luts
            .get(asset)
            .cloned()
            .ok_or_else(|| RenderError::InvalidLut {
                message: format!("the LUT of {asset} is not loaded"),
            })
    }
}

/// Renders every frame of a timeline at full resolution (the export path).
#[derive(Clone, Debug)]
pub struct ExportGraph {
    /// The timeline to export.
    pub timeline: Id,
    /// Output size in pixels.
    pub size: (u32, u32),
    /// Output transfer function.
    pub transfer: TransferFunction,
}

impl ExportGraph {
    /// Exports `timeline` at its own canvas size with the BT.709 transfer.
    pub fn new(project: &Project, timeline: &Id) -> Result<ExportGraph, RenderError> {
        let doc = project
            .timelines
            .get(timeline)
            .ok_or_else(|| RenderError::TimelineNotFound {
                timeline: timeline.to_string(),
            })?;
        Ok(ExportGraph {
            timeline: timeline.clone(),
            size: (doc.format.width, doc.format.height),
            transfer: TransferFunction::Bt709,
        })
    }

    /// The start time of every frame from zero to the end of the timeline. Export never
    /// skips frames.
    pub fn frame_times(&self, project: &Project) -> Result<Vec<Flicks>, RenderError> {
        let doc =
            project
                .timelines
                .get(&self.timeline)
                .ok_or_else(|| RenderError::TimelineNotFound {
                    timeline: self.timeline.to_string(),
                })?;
        let rate = doc.format.rate;
        let end = rate.frame_at(doc.duration() - Flicks::new(1)) + 1;
        Ok((0..end.max(0)).map(|n| rate.frames_to_flicks(n)).collect())
    }

    /// Renders the frame at `time` and reads it back.
    pub fn render(
        &self,
        renderer: &mut Renderer,
        project: &Project,
        frames: &mut dyn FrameProvider,
        time: Flicks,
    ) -> Result<Image, RenderError> {
        let plan = plan_frame(project, &self.timeline, time)?;
        renderer.render_image(&plan, frames, self.size, self.transfer)
    }
}

/// Renders single frames on demand at a preview resolution (the preview path). The
/// playback clock that decides which frames to show and which to drop belongs to the
/// caller.
#[derive(Clone, Debug)]
pub struct PreviewGraph {
    /// The timeline to preview.
    pub timeline: Id,
    /// Output size in pixels, usually smaller than the canvas.
    pub size: (u32, u32),
    /// Output transfer function.
    pub transfer: TransferFunction,
}

impl PreviewGraph {
    /// Previews `timeline` scaled to fit `max_size`, keeping the canvas aspect.
    pub fn new(
        project: &Project,
        timeline: &Id,
        max_size: (u32, u32),
    ) -> Result<PreviewGraph, RenderError> {
        let export = ExportGraph::new(project, timeline)?;
        let (w, h) = (f64::from(export.size.0), f64::from(export.size.1));
        let scale = (f64::from(max_size.0) / w)
            .min(f64::from(max_size.1) / h)
            .min(1.0);
        Ok(PreviewGraph {
            timeline: timeline.clone(),
            size: (
                ((w * scale).round() as u32).max(1),
                ((h * scale).round() as u32).max(1),
            ),
            transfer: TransferFunction::Bt709,
        })
    }

    /// Renders the frame at `time` into a window surface or other target view.
    pub fn render_to(
        &self,
        renderer: &mut Renderer,
        project: &Project,
        frames: &mut dyn FrameProvider,
        time: Flicks,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) -> Result<(), RenderError> {
        let plan = plan_frame(project, &self.timeline, time)?;
        renderer.render_to(&plan, frames, target, format, self.size, self.transfer)
    }

    /// Renders the frame at `time` and reads it back, for thumbnails and agent frame grabs.
    pub fn grab(
        &self,
        renderer: &mut Renderer,
        project: &Project,
        frames: &mut dyn FrameProvider,
        time: Flicks,
    ) -> Result<Image, RenderError> {
        let plan = plan_frame(project, &self.timeline, time)?;
        renderer.render_image(&plan, frames, self.size, self.transfer)
    }
}
