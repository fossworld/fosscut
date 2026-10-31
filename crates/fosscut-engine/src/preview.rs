//! The desktop preview (ADR-0008, preview graph; ADR-0010): frames of a timeline rendered on
//! demand into a window's texture, by the plan and renderer that exports and frame grabs
//! use, with each clip's media kept open from one frame to the next; and its sound, mixed in
//! spans by the export's mixer ([`Sound`]).
//!
//! [`PreviewOptions`] make it faster without changing what an edit looks like (ADR-0032):
//! it can decode proxies ([`crate::proxy`]) and remuxed copies ([`crate::remux`]) instead
//! of the original media when they exist, decode with or without hardware, and keep the
//! frames it renders on disk ([`crate::frame_cache`]). Sound always reads the originals,
//! as exports, frame grabs and contact sheets do.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use fosscut_media::ffmpeg::FfmpegVideoDecoder;
use fosscut_media::{AudioBuffer, DecoderOptions, HardwareDecoding, VideoDecoder, VideoFrame};
use fosscut_model::{AssetKind, ContentIdentity, Id, Project};
use fosscut_render::lut::Lut3d;
use fosscut_render::plan::{
    Layer, LayerSource, RenderPlan, SourceRef, plan_frame, source_geometry,
};
use fosscut_render::{DecoderFrames, FrameProvider, Image, PreviewGraph, RenderError, Renderer};
use fosscut_time::{Flicks, TimeReport};
use serde::{Deserialize, Serialize};

use crate::EngineError;
use crate::cache::touch;
use crate::export::{DecoderAudio, resolve};
use crate::frame::{Still, load_font, load_lut};
use crate::frame_cache::{FrameKey, RenderCache};
use crate::proxy::{ProxySize, proxy_path};
use crate::remux::remux_path;

/// How many bytes of decoded frames a previewer keeps by default.
pub const FRAME_CACHE_BYTES: usize = 256 << 20;

/// How a previewer reads media and reuses rendered frames (ADR-0032). The default decodes
/// the original media with hardware when it can, and renders every frame, as before these
/// options existed. Front ends fill them from the user's `performance` preferences.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviewOptions {
    /// The cache directory ([`crate::cache_dir`]) to find proxies, remuxed copies and
    /// rendered frames in; without it, none are used.
    pub cache: Option<PathBuf>,
    /// Decode the proxy of this size of a clip's media when one has been made
    /// (`performance.proxies` other than `off`, with `performance.proxy_size`).
    pub proxies: Option<ProxySize>,
    /// Decode the remuxed copy of a clip's media when one has been made
    /// (`performance.auto_remux`). A proxy, when used, comes first.
    pub remuxed: bool,
    /// Hardware decoding for the preview's decoders (`performance.hardware_decoding`).
    pub hardware_decoding: HardwareDecoding,
    /// Keep rendered frames on disk, at most this many bytes of them
    /// (`performance.render_cache_gb`), and show a kept frame instead of rendering it;
    /// `None` renders every frame.
    pub render_cache_bytes: Option<u64>,
}

/// Which file the preview decodes for an asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSource {
    /// The asset's own media.
    Original,
    /// Its proxy of this size.
    Proxy(ProxySize),
    /// Its remuxed copy.
    Remux,
}

/// Renders preview frames. Keep one per window: it holds the decoders of every clip shown
/// so far, which stay valid across edits because imported media never changes, and the
/// frames decoded most recently, so scrubbing back over them needs no seek.
pub struct Previewer {
    options: PreviewOptions,
    media: PreviewMedia,
    cache: FrameCache,
    rendered: Option<RenderCache>,
    blit: Option<Blit>,
}

impl Default for Previewer {
    fn default() -> Previewer {
        Previewer::with_options(PreviewOptions::default())
    }
}

impl Previewer {
    /// A previewer with no media open yet.
    pub fn new() -> Previewer {
        Previewer::default()
    }

    /// A previewer with `options` and no media open yet.
    pub fn with_options(options: PreviewOptions) -> Previewer {
        let rendered = render_cache(&options);
        Previewer {
            options,
            media: PreviewMedia::default(),
            cache: FrameCache::new(FRAME_CACHE_BYTES),
            rendered,
            blit: None,
        }
    }

    /// The options in use.
    pub fn options(&self) -> &PreviewOptions {
        &self.options
    }

    /// Uses `options` from the next frame on. Media is opened again when what it decodes
    /// may change (the cache, proxies, remuxed copies or hardware decoding), and the frame
    /// cache follows its new folder and limit.
    pub fn set_options(&mut self, options: PreviewOptions) {
        let media_changed = (
            &options.cache,
            options.proxies,
            options.remuxed,
            options.hardware_decoding,
        ) != (
            &self.options.cache,
            self.options.proxies,
            self.options.remuxed,
            self.options.hardware_decoding,
        );
        if media_changed {
            self.reopen_media();
        }
        if options.cache != self.options.cache
            || options.render_cache_bytes.is_none()
            || self.rendered.is_none()
        {
            self.rendered = render_cache(&options);
        } else if let (Some(rendered), Some(limit)) =
            (&mut self.rendered, options.render_cache_bytes)
        {
            rendered.set_limit(limit);
        }
        self.options = options;
    }

    /// Closes every decoder and forgets the decoded frames, so the next frame opens media
    /// again: call it when a proxy or remuxed copy was made (or the cache cleared) while
    /// previewing, to start or stop using it.
    pub fn reopen_media(&mut self) {
        self.media = PreviewMedia::default();
        self.cache = FrameCache::new(FRAME_CACHE_BYTES);
    }

    /// Which file the preview decodes for `asset`, once a frame showed it.
    pub fn source(&self, asset: &Id) -> Option<PreviewSource> {
        self.media.opened.get(asset).map(|opened| opened.source)
    }

    /// The frames kept on disk, when [`PreviewOptions::render_cache_bytes`] is set.
    pub fn render_cache(&self) -> Option<&RenderCache> {
        self.rendered.as_ref()
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
    /// [`Previewer::size`] for `max_size` in `format`. Returns the frame shown. With a
    /// frame cache, a kept frame is drawn instead of rendered, and a rendered one is kept.
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
        let (graph, at, plan) = self.prepare(project, bundle, timeline, time, max_size)?;
        if let Some(key) = self.key(project, bundle, &plan, &graph) {
            let image = self.cached_or_rendered(renderer, &plan, &graph, &key)?;
            self.blit
                .get_or_insert_with(|| Blit::new(renderer.device()))
                .draw(renderer, &image, target, format);
            return Ok(at);
        }
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
        let (graph, at, plan) = self.prepare(project, bundle, timeline, time, max_size)?;
        let image = match self.key(project, bundle, &plan, &graph) {
            Some(key) => self.cached_or_rendered(renderer, &plan, &graph, &key)?,
            None => self.render_image(renderer, &plan, &graph)?,
        };
        Ok(Still { at, image })
    }

    /// Renders the frames of `timeline` at `times` that the frame cache does not have yet
    /// and keeps them, as the desktop app does around the playhead while it is idle
    /// (`performance.auto_render`). Returns how many were rendered; without a frame cache
    /// nothing is.
    #[allow(clippy::too_many_arguments)]
    pub fn prerender(
        &mut self,
        renderer: &mut Renderer,
        project: &Project,
        bundle: &Path,
        timeline: &Id,
        times: &[Flicks],
        max_size: (u32, u32),
    ) -> Result<usize, EngineError> {
        let mut rendered = 0;
        for &time in times {
            if self.rendered.is_none() {
                break;
            }
            let (graph, _, plan) = self.prepare(project, bundle, timeline, time, max_size)?;
            let Some(key) = self.key(project, bundle, &plan, &graph) else {
                continue;
            };
            if self.rendered.as_ref().is_some_and(|r| r.contains(&key)) {
                continue;
            }
            let image = self.render_image(renderer, &plan, &graph)?;
            if let Some(cache) = &mut self.rendered {
                cache.put(&key, &image);
            }
            rendered += 1;
        }
        Ok(rendered)
    }

    /// The preview graph, the frame shown at `time` and its plan, with its media open and
    /// placed for the files the preview decodes.
    fn prepare(
        &mut self,
        project: &Project,
        bundle: &Path,
        timeline: &Id,
        time: Flicks,
        max_size: (u32, u32),
    ) -> Result<(PreviewGraph, TimeReport, RenderPlan), EngineError> {
        let graph = graph(project, timeline, max_size)?;
        let at = Previewer::shown_at(project, timeline, time)?;
        let mut plan = plan_frame(project, timeline, at.flicks)?;
        self.media.open_for(project, bundle, &plan, &self.options)?;
        self.media.reframe(project, &mut plan);
        Ok((graph, at, plan))
    }

    /// The frame cache key of `plan`, when there is a frame cache and every asset it reads
    /// is known.
    fn key(
        &mut self,
        project: &Project,
        bundle: &Path,
        plan: &RenderPlan,
        graph: &PreviewGraph,
    ) -> Option<FrameKey> {
        self.rendered.as_ref()?;
        let media = &mut self.media;
        FrameKey::new(project, plan, graph.size, graph.transfer, &mut |asset| {
            media.describe(project, bundle, asset)
        })
    }

    fn cached_or_rendered(
        &mut self,
        renderer: &mut Renderer,
        plan: &RenderPlan,
        graph: &PreviewGraph,
        key: &FrameKey,
    ) -> Result<Image, EngineError> {
        if let Some(image) = self
            .rendered
            .as_mut()
            .and_then(|cache| cache.get(key))
            .filter(|image| (image.width, image.height) == graph.size)
        {
            return Ok(image);
        }
        let image = self.render_image(renderer, plan, graph)?;
        if let Some(cache) = &mut self.rendered {
            cache.put(key, &image);
        }
        Ok(image)
    }

    fn render_image(
        &mut self,
        renderer: &mut Renderer,
        plan: &RenderPlan,
        graph: &PreviewGraph,
    ) -> Result<Image, EngineError> {
        let mut frames = Cached {
            inner: self.media.frames(),
            cache: &mut self.cache,
        };
        Ok(renderer.render_image(plan, &mut frames, graph.size, graph.transfer)?)
    }
}

/// The frame cache `options` ask for.
fn render_cache(options: &PreviewOptions) -> Option<RenderCache> {
    match (&options.cache, options.render_cache_bytes) {
        (Some(cache), Some(limit)) => Some(RenderCache::open(cache, limit)),
        _ => None,
    }
}

/// The file a frame source is decoded from, and the size of its frames.
#[derive(Clone, Copy, Debug)]
struct Opened {
    source: PreviewSource,
    size: (u32, u32),
}

/// Decoders for the media the preview shows, each opened on first use and kept, with the
/// LUTs and fonts the frames use.
#[derive(Default)]
struct PreviewMedia {
    decoders: DecoderFrames,
    opened: HashMap<Id, Opened>,
    /// Content identities by asset, recorded or measured once; `None` when unknown.
    identities: HashMap<Id, Option<ContentIdentity>>,
}

impl PreviewMedia {
    /// Opens the media of every frame layer of `plan` not opened yet, choosing a proxy or
    /// remuxed copy as `options` allow, and loads the LUTs and fonts its layers use.
    fn open_for(
        &mut self,
        project: &Project,
        bundle: &Path,
        plan: &RenderPlan,
        options: &PreviewOptions,
    ) -> Result<(), EngineError> {
        // Transitions and compound clips hold layers of their own.
        let layers = plan.layers.iter().flat_map(|layer| layer.flattened());
        for layer in layers {
            for effect in &layer.effects {
                if let Some(asset) = effect.asset("lut")
                    && !self.decoders.has_lut(asset)
                {
                    let lut = load_lut(project, bundle, asset)?;
                    self.decoders.insert_lut(asset.clone(), lut);
                }
            }
            if let LayerSource::Text(text) = &layer.source {
                for family in text.content.fonts() {
                    if !self.decoders.has_font(family)
                        && let Some((families, data)) = load_font(project, bundle, family)?
                    {
                        self.decoders.insert_font(&families, data);
                    }
                }
            }
            if let LayerSource::Frame(source) = &layer.source
                && !self.opened.contains_key(&source.asset)
            {
                self.open(project, bundle, &source.asset, options)?;
            }
        }
        Ok(())
    }

    /// Opens the file the preview decodes for `asset`: its proxy, its remuxed copy or its
    /// own media. A copy that cannot be opened is passed over for the original.
    fn open(
        &mut self,
        project: &Project,
        bundle: &Path,
        asset: &Id,
        options: &PreviewOptions,
    ) -> Result<(), EngineError> {
        let original = resolve(project, bundle, asset)?;
        let decoding = DecoderOptions {
            hardware: options.hardware_decoding,
        };
        let copy = self
            .copies(project, bundle, asset, options)
            .into_iter()
            .find_map(|(path, source)| {
                let decoder = open_copy(&path, &original, decoding)?;
                touch(&path);
                Some((decoder, source))
            });
        let (decoder, source): (Decoder, PreviewSource) = match copy {
            Some(copy) => copy,
            None => (
                Box::new(FfmpegVideoDecoder::open(&original, decoding)?),
                PreviewSource::Original,
            ),
        };
        let size = (decoder.info().width, decoder.info().height);
        self.decoders.insert(asset.clone(), decoder);
        self.opened.insert(asset.clone(), Opened { source, size });
        Ok(())
    }

    /// The copies of `asset` that exist and `options` allow, best first.
    fn copies(
        &mut self,
        project: &Project,
        bundle: &Path,
        asset: &Id,
        options: &PreviewOptions,
    ) -> Vec<(PathBuf, PreviewSource)> {
        let Some(cache) = &options.cache else {
            return Vec::new();
        };
        let is_video = project
            .media
            .get(asset)
            .is_some_and(|a| a.kind == AssetKind::Video && a.info.video.is_some());
        if !is_video || (options.proxies.is_none() && !options.remuxed) {
            return Vec::new();
        }
        let Some(identity) = self.identity(project, bundle, asset) else {
            return Vec::new();
        };
        let mut copies = Vec::new();
        if let Some(size) = options.proxies {
            copies.push((
                proxy_path(cache, &identity, size),
                PreviewSource::Proxy(size),
            ));
        }
        if options.remuxed {
            copies.push((remux_path(cache, &identity), PreviewSource::Remux));
        }
        copies.retain(|(path, _)| path.is_file());
        copies
    }

    /// The content identity of `asset`: recorded at import, or measured from its file.
    fn identity(
        &mut self,
        project: &Project,
        bundle: &Path,
        asset: &Id,
    ) -> Option<ContentIdentity> {
        self.identities
            .entry(asset.clone())
            .or_insert_with(|| {
                crate::cache::source_of(project, bundle, asset)
                    .ok()
                    .map(|(_, identity)| identity)
            })
            .clone()
    }

    /// What the frame cache needs to know about `asset`: what its file contains and which
    /// copy of it is decoded.
    fn describe(&mut self, project: &Project, bundle: &Path, asset: &Id) -> Option<String> {
        let identity = self.identity(project, bundle, asset)?;
        let decoded = match self.opened.get(asset).map(|o| o.source) {
            Some(PreviewSource::Proxy(size)) => format!("proxy-{}", size.name()),
            Some(PreviewSource::Remux) => "remux".to_string(),
            Some(PreviewSource::Original) | None => "original".to_string(),
        };
        Some(format!(
            "{}:{}:{decoded}",
            identity.sample_blake3, identity.size
        ))
    }

    /// Corrects the placement of layers shown from proxies, whose size may differ from the
    /// original's in aspect by up to a pixel: each such layer is stretched so it covers
    /// exactly the area the original would (ADR-0032, item 2).
    fn reframe(&self, project: &Project, plan: &mut RenderPlan) {
        let canvas = plan.canvas;
        for layer in &mut plan.layers {
            self.reframe_layer(project, canvas, layer);
        }
    }

    fn reframe_layer(&self, project: &Project, canvas: (u32, u32), layer: &mut Layer) {
        match &mut layer.source {
            LayerSource::Frame(source) => {
                let asset = source.asset.clone();
                let Some(Opened {
                    source: PreviewSource::Proxy(_),
                    size,
                }) = self.opened.get(&asset).copied()
                else {
                    return;
                };
                let Some((original, _)) = source_geometry(project, &asset) else {
                    return;
                };
                if original.0 == 0 || original.1 == 0 {
                    return;
                }
                let (ow, oh) = fitted(canvas, original, layer.rotation);
                let (pw, ph) = fitted(canvas, size, layer.rotation);
                layer.transform.stretch[0] *= ow / pw;
                layer.transform.stretch[1] *= oh / ph;
            }
            LayerSource::Transition(transition) => {
                self.reframe_layer(project, canvas, &mut transition.outgoing);
                self.reframe_layer(project, canvas, &mut transition.incoming);
            }
            LayerSource::Nested(nested) => self.reframe(project, nested),
            LayerSource::Text(_)
            | LayerSource::Shape(_)
            | LayerSource::Caption(_)
            | LayerSource::Adjustment => {}
        }
    }

    /// The decoders, as the renderer reads frames.
    fn frames(&mut self) -> &mut DecoderFrames {
        &mut self.decoders
    }
}

/// A decoder the renderer reads frames from.
type Decoder = Box<dyn VideoDecoder + Send>;

/// A decoder for the copy at `path` of the media at `original`, showing each frame at the
/// time the original shows it, or `None` when either cannot be decoded. Their first frames'
/// times are compared once: they differ when the original's container starts its stream
/// before its first frame (AVI with reordered frames, see
/// [`fosscut_media::ffmpeg::remux`]; a proxy made from it inherits that).
fn open_copy(path: &Path, original: &Path, decoding: DecoderOptions) -> Option<Decoder> {
    let mut copy = FfmpegVideoDecoder::open(path, decoding).ok()?;
    let first_copied = copy.next_frame().ok()??.pts;
    let first_original = FfmpegVideoDecoder::open(original, decoding)
        .ok()?
        .next_frame()
        .ok()??
        .pts;
    let offset = first_original - first_copied;
    Some(if offset == Flicks::ZERO {
        Box::new(copy)
    } else {
        Box::new(Shifted {
            inner: copy,
            offset,
        })
    })
}

/// A copy's decoder whose frames are shown `offset` later than the copy stores them, so
/// they line up with the original's.
struct Shifted {
    inner: FfmpegVideoDecoder,
    offset: Flicks,
}

impl VideoDecoder for Shifted {
    fn info(&self) -> &fosscut_media::VideoStreamInfo {
        self.inner.info()
    }

    fn acceleration(&self) -> fosscut_media::Acceleration {
        self.inner.acceleration()
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, fosscut_media::MediaError> {
        Ok(self.inner.next_frame()?.map(|mut frame| {
            frame.pts = frame.pts + self.offset;
            frame
        }))
    }

    fn frame_at(&mut self, time: Flicks) -> Result<VideoFrame, fosscut_media::MediaError> {
        let mut frame = self
            .inner
            .frame_at((time - self.offset).max(Flicks::ZERO))?;
        frame.pts = frame.pts + self.offset;
        Ok(frame)
    }
}

/// The displayed size of a `source`-sized frame turned `rotation` degrees and fitted into
/// `canvas`, as [`fosscut_render::geometry::layer_quad`] fits it.
fn fitted(canvas: (u32, u32), source: (u32, u32), rotation: u16) -> (f64, f64) {
    let (canvas_w, canvas_h) = (f64::from(canvas.0), f64::from(canvas.1));
    let (mut w, mut h) = (f64::from(source.0.max(1)), f64::from(source.1.max(1)));
    if rotation == 90 || rotation == 270 {
        std::mem::swap(&mut w, &mut h);
    }
    let fit = (canvas_w / w).min(canvas_h / h);
    (w * fit, h * fit)
}

/// Draws a kept frame into a target view: its pixels as they are, as if the renderer had
/// encoded them into a target of that format.
struct Blit {
    layout: wgpu::BindGroupLayout,
    module: wgpu::ShaderModule,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

const BLIT: &str = r"
@group(0) @binding(0) var image: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4f(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    return textureLoad(image, vec2i(position.xy), 0);
}
";

impl Blit {
    fn new(device: &wgpu::Device) -> Blit {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kept frame"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kept frame"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });
        Blit {
            layout,
            module,
            pipelines: HashMap::new(),
        }
    }

    fn draw(
        &mut self,
        renderer: &Renderer,
        image: &Image,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        use wgpu::util::DeviceExt;
        let device = renderer.device();
        let texture = device.create_texture_with_data(
            renderer.queue(),
            &wgpu::TextureDescriptor {
                label: Some("kept frame"),
                size: wgpu::Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &image.rgba,
        );
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kept frame"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        let (layout, module) = (&self.layout, &self.module);
        let pipeline = self.pipelines.entry(format).or_insert_with(|| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kept frame"),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("kept frame"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("kept frame"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kept frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        renderer.queue().submit([encoder.finish()]);
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

    fn lut(&mut self, asset: &Id) -> Result<std::sync::Arc<Lut3d>, RenderError> {
        self.inner.lut(asset)
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
    fn proxies_are_placed_exactly_where_the_original_would_be() {
        use fosscut_model::{
            Asset, Clip, ClipContent, MediaInfo, PathHints, TimelineFormat, Track, TrackKind,
            VideoInfo,
        };
        use fosscut_render::geometry::layer_quad;
        use fosscut_time::Rate;

        let cases = [
            ((1000, 562), (500, 282), 0),
            ((1366, 768), (342, 192), -90),
            ((1920, 1080), (960, 540), 0),
        ];
        for (original, proxy, rotation) in cases {
            let format = TimelineFormat {
                width: 1920,
                height: 1080,
                rate: Rate::FPS_30,
                sample_rate: 48_000,
            };
            let mut project = Project::new(Id::new(IdKind::Project, 1), "Proxy", format);
            let asset = Id::new(IdKind::Asset, 1);
            project.media.assets.push(Asset {
                folder: None,
                id: asset.clone(),
                kind: AssetKind::Video,
                name: "clip".into(),
                path: PathHints::default(),
                identity: None,
                info: MediaInfo {
                    duration: Some(Rate::FPS_30.frames_to_flicks(30)),
                    video: Some(VideoInfo {
                        width: original.0,
                        height: original.1,
                        rate: Some(Rate::FPS_30),
                        codec: None,
                        rotation,
                    }),
                    ..Default::default()
                },
            });
            let mut clip = Clip::new(
                Id::new(IdKind::Clip, 3),
                Flicks::ZERO,
                Rate::FPS_30.frames_to_flicks(30),
                ClipContent::Video {
                    asset: asset.clone(),
                    source_in: Flicks::ZERO,
                },
            );
            clip.transform.position = [0.2, 0.1];
            clip.transform.scale = 1.7;
            clip.transform.stretch = [1.1, 0.9];
            clip.transform.rotation = 12.0;
            let timeline = project.manifest.primary_timeline.clone();
            project
                .timelines
                .get_mut(&timeline)
                .unwrap()
                .tracks
                .push(Track {
                    id: Id::new(IdKind::Track, 2),
                    kind: TrackKind::Video,
                    name: None,
                    muted: false,
                    hidden: false,
                    locked: false,
                    caption_style: None,
                    clips: vec![clip],
                });
            let mut plan = plan_frame(&project, &timeline, Flicks::ZERO).unwrap();
            let before = plan.layers[0].clone();
            let expected = layer_quad(
                plan.canvas,
                original,
                before.rotation,
                &before.transform,
                before.crop,
            );

            let mut media = PreviewMedia::default();
            media.opened.insert(
                asset.clone(),
                Opened {
                    source: PreviewSource::Original,
                    size: original,
                },
            );
            media.reframe(&project, &mut plan);
            assert_eq!(plan.layers[0], before, "originals are not moved");

            media.opened.insert(
                asset,
                Opened {
                    source: PreviewSource::Proxy(ProxySize::Half),
                    size: proxy,
                },
            );
            media.reframe(&project, &mut plan);
            let after = &plan.layers[0];
            let shown = layer_quad(
                plan.canvas,
                proxy,
                after.rotation,
                &after.transform,
                after.crop,
            );
            for (e, s) in expected.iter().zip(&shown) {
                for axis in 0..2 {
                    assert!(
                        (e.position[axis] - s.position[axis]).abs() < 1e-6,
                        "{original:?} from {proxy:?}: {e:?} against {s:?}"
                    );
                    assert_eq!(e.uv[axis], s.uv[axis]);
                }
            }
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
