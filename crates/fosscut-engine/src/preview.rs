//! The desktop preview (ADR-0008, preview graph; ADR-0010): frames of a timeline rendered on
//! demand into a window's texture, by the plan and renderer that exports and frame grabs
//! use, with each clip's media kept open from one frame to the next; and its sound, mixed in
//! spans by the export's mixer ([`Sound`]).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use fosscut_media::{AudioBuffer, VideoFrame};
use fosscut_model::{Id, Project};
use fosscut_render::plan::{SourceRef, plan_frame};
use fosscut_render::{FrameProvider, PreviewGraph, RenderError, Renderer};
use fosscut_time::{Flicks, TimeReport};

use crate::EngineError;
use crate::export::{DecoderAudio, resolve};
use crate::frame::{Media, Still};

/// How many bytes of decoded frames a previewer keeps by default.
pub const FRAME_CACHE_BYTES: usize = 256 << 20;

/// Renders preview frames. Keep one per window: it holds the decoders of every clip shown
/// so far, which stay valid across edits because imported media never changes, and the
/// frames decoded most recently, so scrubbing back over them needs no seek.
pub struct Previewer {
    media: Media,
    cache: FrameCache,
}

impl Default for Previewer {
    fn default() -> Previewer {
        Previewer {
            media: Media::default(),
            cache: FrameCache::new(FRAME_CACHE_BYTES),
        }
    }
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
        let mut frames = Cached {
            inner: self.media.frames(),
            cache: &mut self.cache,
        };
        renderer.render_to(
            &plan,
            &mut frames,
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
        let mut frames = Cached {
            inner: self.media.frames(),
            cache: &mut self.cache,
        };
        let image = renderer.render_image(&plan, &mut frames, graph.size, graph.transfer)?;
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

/// Timeline sound for playback: spans mixed by the export's mixer, with each asset's
/// decoder kept open from one span to the next.
#[derive(Default)]
pub struct Sound {
    audio: DecoderAudio,
}

impl Sound {
    /// No media open yet.
    pub fn new() -> Sound {
        Sound::default()
    }

    /// `duration` of `timeline` from `start`, as interleaved samples at `sample_rate` with
    /// `channels` channels, mixed exactly as an export mixes it. Media that cannot be found
    /// fails only when a span needs it.
    #[allow(clippy::too_many_arguments)]
    pub fn mix(
        &mut self,
        project: &Project,
        bundle: &Path,
        timeline: &Id,
        start: Flicks,
        duration: Flicks,
        sample_rate: u32,
        channels: u16,
    ) -> Result<AudioBuffer, EngineError> {
        for asset in &project.media.assets {
            if asset.info.audio.is_some()
                && !self.audio.paths.contains_key(&asset.id)
                && let Ok(path) = resolve(project, bundle, &asset.id)
            {
                self.audio.paths.insert(asset.id.clone(), path);
            }
        }
        Ok(fosscut_audio::mix(
            project,
            timeline,
            start,
            duration,
            sample_rate,
            channels,
            &mut self.audio,
        )?)
    }
}

/// Decoded frames by asset and time, the least recently used dropped first beyond a byte
/// budget (ADR-0008, item 6). A frame answers every time it covers, from its start up to
/// its duration, so repeated and backward requests skip the decoder's seek.
pub struct FrameCache {
    budget: usize,
    bytes: usize,
    tick: u64,
    frames: HashMap<Id, BTreeMap<i64, (VideoFrame, u64)>>,
}

impl FrameCache {
    /// An empty cache that keeps at most `budget` bytes of frames.
    pub fn new(budget: usize) -> FrameCache {
        FrameCache {
            budget,
            bytes: 0,
            tick: 0,
            frames: HashMap::new(),
        }
    }

    /// Bytes of frames kept.
    pub fn bytes(&self) -> usize {
        self.bytes
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
        self.tick += 1;
        let size = frame_bytes(&frame);
        if size > self.budget {
            return;
        }
        let kept = self.frames.entry(asset.clone()).or_default();
        if let Some((old, _)) = kept.insert(frame.pts.get(), (frame, self.tick)) {
            self.bytes -= frame_bytes(&old);
        }
        self.bytes += size;
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

/// A frame provider that answers from the cache, and asks `inner` otherwise.
struct Cached<'a, P: FrameProvider> {
    inner: &'a mut P,
    cache: &'a mut FrameCache,
}

impl<P: FrameProvider> FrameProvider for Cached<'_, P> {
    fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError> {
        if let Some(frame) = self.cache.get(source) {
            return Ok(frame);
        }
        let frame = self.inner.frame(source)?;
        self.cache.insert(&source.asset, frame.clone());
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fosscut_media::{ColorInfo, PixelFormat, Plane};
    use fosscut_model::IdKind;

    use super::*;

    /// One 100-byte frame every 10 flicks (frames of no duration for asset 9, like a still),
    /// counting how often it decodes.
    #[derive(Default)]
    struct Counting {
        decoded: usize,
    }

    impl FrameProvider for Counting {
        fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError> {
            self.decoded += 1;
            let start = source.time.get() / 10 * 10;
            Ok(VideoFrame {
                pts: Flicks::new(start),
                duration: Flicks::new(if source.asset.to_string().ends_with('9') {
                    0
                } else {
                    10
                }),
                width: 10,
                height: 10,
                format: PixelFormat::Nv12,
                planes: vec![Plane {
                    stride: 10,
                    rows: 10,
                    data: Arc::from(vec![0u8; 100]),
                }],
                color: ColorInfo::default(),
            })
        }
    }

    fn at(asset: u64, time: i64) -> SourceRef {
        SourceRef {
            asset: Id::new(IdKind::Asset, asset),
            time: Flicks::new(time),
        }
    }

    #[test]
    fn frames_answer_every_time_they_cover() {
        let (mut inner, mut cache) = (Counting::default(), FrameCache::new(1_000));
        let mut frames = Cached {
            inner: &mut inner,
            cache: &mut cache,
        };
        let first = frames.frame(&at(1, 23)).unwrap();
        assert_eq!(first.pts, Flicks::new(20));
        assert_eq!(
            frames.frame(&at(1, 29)).unwrap(),
            first,
            "same frame, from the cache"
        );
        frames.frame(&at(1, 30)).unwrap();
        frames.frame(&at(2, 25)).unwrap();
        frames.frame(&at(1, 21)).unwrap();
        assert_eq!(
            inner.decoded, 3,
            "backward and repeated requests skip the decoder"
        );
        assert_eq!(cache.bytes(), 300);
    }

    #[test]
    fn stills_match_their_own_time_and_old_frames_go_first() {
        let (mut inner, mut cache) = (Counting::default(), FrameCache::new(250));
        let mut frames = Cached {
            inner: &mut inner,
            cache: &mut cache,
        };
        // Asset 9 has frames of no duration, like a still image.
        frames.frame(&at(9, 0)).unwrap();
        frames.frame(&at(9, 0)).unwrap();
        frames.frame(&at(1, 0)).unwrap();
        frames.frame(&at(9, 0)).unwrap();
        frames.frame(&at(1, 10)).unwrap();
        // Over the budget: the least recently used frame, asset 1 at 0, went.
        frames.frame(&at(9, 0)).unwrap();
        frames.frame(&at(1, 0)).unwrap();
        assert_eq!(inner.decoded, 4);
        assert!(cache.bytes() <= 250);
        let mut huge = FrameCache::new(50);
        huge.insert(
            &Id::new(IdKind::Asset, 1),
            Counting::default().frame(&at(1, 0)).unwrap(),
        );
        assert_eq!(
            huge.bytes(),
            0,
            "a frame larger than the budget is not kept"
        );
    }
}
