//! Preview and export graphs over one frame evaluation (ADR-0008, item 3): both plan a
//! frame with [`plan_frame`] and render it with the same [`Renderer`], so they differ only
//! in how they are driven (resolution, which frames, where the pixels go).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use fosscut_media::{VideoDecoder, VideoFrame};
use fosscut_model::{Id, Project};
use fosscut_time::Flicks;

use crate::RenderError;
use crate::color::TransferFunction;
use crate::gpu::{Image, Renderer};
use crate::lut::Lut3d;
use crate::plan::{SourceRef, plan_frame};
use crate::subject::{MatteKind, Pose, SubjectData};

/// Supplies the decoded frame a layer needs.
pub trait FrameProvider {
    /// The frame of `source.asset` displayed at `source.time`.
    fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError>;

    /// The data of the font file that has `family`, for text in a project font (ADR-0027);
    /// `None` when no font asset has it.
    fn font(&mut self, family: &str) -> Result<Option<Arc<[u8]>>, RenderError> {
        let _ = family;
        Ok(None)
    }

    /// The table of a LUT asset, for `color.lut` effects. Providers without LUTs fail.
    fn lut(&mut self, asset: &Id) -> Result<Arc<Lut3d>, RenderError> {
        Err(RenderError::InvalidLut {
            message: format!("no LUT is available for {asset}"),
        })
    }

    /// The cached matte of `kind` for the frame of `source.asset` shown at `source.time`,
    /// for `key.subject` (ADR-0033); `None` when it was not analyzed, which leaves the
    /// picture unchanged.
    fn matte(
        &mut self,
        source: &SourceRef,
        kind: MatteKind,
    ) -> Result<Option<Arc<crate::subject::Matte>>, RenderError> {
        let _ = (source, kind);
        Ok(None)
    }

    /// The cached pose and person mask at `source`, for the body effects; `None` when it was
    /// not analyzed.
    fn pose(&mut self, source: &SourceRef) -> Result<Option<Arc<Pose>>, RenderError> {
        let _ = source;
        Ok(None)
    }
}

/// How many bytes of decoded frames a [`DecoderFrames`] keeps by default: enough for the
/// widest window of neighbouring frames temporal effects read (17 frames of 4K) to be
/// decoded once while frames are rendered in order (ADR-0035).
pub const WINDOW_BYTES: usize = 256 << 20;

/// Frames from one [`VideoDecoder`] per asset. Decoders must be `Send` so the provider can
/// live on a render thread.
///
/// The frames decoded most recently are kept, up to [`WINDOW_BYTES`] (the least recently
/// used go first), and answer every time they cover. Temporal effects and smooth slow
/// motion ask for a frame's neighbours as well (ADR-0035), so rendering frames in order asks
/// again for frames decoded a moment before; keeping them means decoders only ever read
/// forward instead of seeking back for each frame.
pub struct DecoderFrames {
    decoders: HashMap<Id, Box<dyn VideoDecoder + Send>>,
    luts: HashMap<Id, Arc<Lut3d>>,
    fonts: HashMap<String, Arc<[u8]>>,
    subjects: HashMap<Id, Arc<dyn SubjectData>>,
    window: FrameWindow,
}

impl Default for DecoderFrames {
    fn default() -> DecoderFrames {
        DecoderFrames {
            decoders: HashMap::new(),
            luts: HashMap::new(),
            fonts: HashMap::new(),
            subjects: HashMap::new(),
            window: FrameWindow::new(WINDOW_BYTES),
        }
    }
}

impl DecoderFrames {
    /// No decoders yet.
    pub fn new() -> DecoderFrames {
        DecoderFrames::default()
    }

    /// Keeps at most `bytes` of decoded frames from now on; 0 keeps none, for callers with
    /// a frame cache of their own.
    pub fn set_window_bytes(&mut self, bytes: usize) {
        self.window.budget = bytes;
        self.window.shrink();
    }

    /// Bytes of decoded frames kept.
    pub fn window_bytes(&self) -> usize {
        self.window.bytes
    }

    /// Uses `decoder` for `asset`'s frames, forgetting frames kept from an earlier one.
    pub fn insert(&mut self, asset: Id, decoder: Box<dyn VideoDecoder + Send>) {
        if let Some(kept) = self.window.frames.remove(&asset) {
            self.window.bytes -= kept.values().map(|(f, _)| frame_bytes(f)).sum::<usize>();
        }
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

    /// Makes a font file's data available under each of its families.
    pub fn insert_font(&mut self, families: &[String], data: Arc<[u8]>) {
        for family in families {
            self.fonts.insert(family.clone(), data.clone());
        }
    }

    /// True when a font file with `family` is available.
    pub fn has_font(&self, family: &str) -> bool {
        self.fonts.contains_key(family)
    }

    /// Uses `data` for the mattes and poses of `asset`'s frames, replacing what it had.
    pub fn insert_subject(&mut self, asset: Id, data: Arc<dyn SubjectData>) {
        self.subjects.insert(asset, data);
    }

    /// The subject data of `asset`, if any.
    pub fn subject(&self, asset: &Id) -> Option<&Arc<dyn SubjectData>> {
        self.subjects.get(asset)
    }
}

/// Decoded frames by asset and start time, the least recently used dropped first beyond a
/// byte budget.
struct FrameWindow {
    budget: usize,
    bytes: usize,
    tick: u64,
    frames: HashMap<Id, BTreeMap<i64, (VideoFrame, u64)>>,
}

impl FrameWindow {
    fn new(budget: usize) -> FrameWindow {
        FrameWindow {
            budget,
            bytes: 0,
            tick: 0,
            frames: HashMap::new(),
        }
    }

    /// The kept frame of `source.asset` showing at `source.time`.
    fn get(&mut self, source: &SourceRef) -> Option<VideoFrame> {
        self.tick += 1;
        let time = source.time.get();
        let (_, (frame, used)) = self
            .frames
            .get_mut(&source.asset)?
            .range_mut(..=time)
            .next_back()?;
        let covers = time == frame.pts.get() || time < (frame.pts + frame.duration).get();
        if !covers {
            return None;
        }
        *used = self.tick;
        Some(frame.clone())
    }

    fn insert(&mut self, asset: &Id, frame: VideoFrame) {
        let size = frame_bytes(&frame);
        if size > self.budget {
            return;
        }
        self.tick += 1;
        let kept = self.frames.entry(asset.clone()).or_default();
        if let Some((old, _)) = kept.insert(frame.pts.get(), (frame, self.tick)) {
            self.bytes -= frame_bytes(&old);
        }
        self.bytes += size;
        self.shrink();
    }

    /// Drops the least recently used frames until the rest fit the budget.
    fn shrink(&mut self) {
        while self.bytes > self.budget {
            let oldest = self
                .frames
                .iter()
                .flat_map(|(asset, kept)| {
                    kept.iter()
                        .map(move |(pts, (_, used))| (*used, asset.clone(), *pts))
                })
                .min();
            let Some((_, asset, pts)) = oldest else {
                break;
            };
            if let Some((frame, _)) = self
                .frames
                .get_mut(&asset)
                .and_then(|kept| kept.remove(&pts))
            {
                self.bytes -= frame_bytes(&frame);
            }
        }
    }
}

fn frame_bytes(frame: &VideoFrame) -> usize {
    frame.planes.iter().map(|plane| plane.data.len()).sum()
}

impl FrameProvider for DecoderFrames {
    fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError> {
        if let Some(frame) = self.window.get(source) {
            return Ok(frame);
        }
        let missing = |message: String| RenderError::MissingFrame {
            asset: source.asset.to_string(),
            message,
        };
        let decoder = self
            .decoders
            .get_mut(&source.asset)
            .ok_or_else(|| missing("no decoder is open for this asset".to_string()))?;
        let frame = decoder
            .frame_at(source.time)
            .map_err(|e| missing(format!("{e} ({})", e.code())))?;
        self.window.insert(&source.asset, frame.clone());
        Ok(frame)
    }

    fn font(&mut self, family: &str) -> Result<Option<Arc<[u8]>>, RenderError> {
        Ok(self.fonts.get(family).cloned())
    }

    fn lut(&mut self, asset: &Id) -> Result<Arc<Lut3d>, RenderError> {
        self.luts
            .get(asset)
            .cloned()
            .ok_or_else(|| RenderError::InvalidLut {
                message: format!("the LUT of {asset} is not loaded"),
            })
    }

    fn matte(
        &mut self,
        source: &SourceRef,
        kind: MatteKind,
    ) -> Result<Option<Arc<crate::subject::Matte>>, RenderError> {
        Ok(self
            .subjects
            .get(&source.asset)
            .and_then(|data| data.matte(kind, source.time)))
    }

    fn pose(&mut self, source: &SourceRef) -> Result<Option<Arc<Pose>>, RenderError> {
        Ok(self
            .subjects
            .get(&source.asset)
            .and_then(|data| data.pose(source.time)))
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
