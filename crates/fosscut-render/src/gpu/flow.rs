//! Optical flow between two source frames, and the passes that read it (ADR-0035):
//! in-between frames for `Interpolate::Flow`, motion blur and temporal noise reduction. The
//! shaders and the methods they follow are described in `shaders/flow.wgsl`.
//!
//! Flow is estimated at the frames' own size or at the mip level no wider than
//! [`FLOW_MAX_SIDE`], whichever is smaller, so its cost stays bounded for large sources, and
//! it is kept for the next frames: the flow from one frame to the next is also the flow
//! back, read the other way round, so playing forward estimates each pair once. Flows are
//! keyed by the decoded frames they were made from, so they are a cache of their inputs
//! (ADR-0008, item 2).

use fosscut_media::VideoFrame;
use fosscut_model::Id;
use wgpu::util::DeviceExt;

use super::{CANVAS_FORMAT, Renderer, Slot, attachment, entry};
use crate::plan::SourceRef;

/// The longest side of the finest level a flow is estimated at, in pixels.
pub(super) const FLOW_MAX_SIDE: u32 = 960;
/// The shortest side of the coarsest level, in pixels.
const FLOW_MIN_SIDE: u32 = 8;
/// The most levels a flow is refined over: the coarsest one moves things up to about
/// 2^6 pixels of the finest per step.
const FLOW_LEVELS: u32 = 7;
/// Gauss-Newton steps per level.
const ITERATIONS: u32 = 3;
/// The Lucas-Kanade window's radius and sigma in pixels of its level.
const WINDOW: (f32, f32) = (3.0, 2.0);
/// Damping of the Lucas-Kanade solve: textureless areas keep the flow of the level above.
const DAMPING: f32 = 4e-5;
/// Luma error at which a pixel's pull on the flow halves (Cauchy weight).
const ROBUST: f32 = 0.06;
/// Luma and chroma difference at which a window pixel's weight falls to 0.6.
const SUPPORT: f32 = 0.15;
/// The flow's joint bilateral smoothing: radius and sigma in pixels, and colour sigma.
const SMOOTHING: (f32, f32, f32) = (2.0, 1.5, 0.08);
/// Flows kept between frames.
const FLOW_CACHE: usize = 8;
/// Brightness levels kept between frames.
const LEVEL_CACHE: usize = 1024;
/// Flow pixels per motion blur tile: the most a pixel streaks to each side.
const TILE: u32 = 32;

const FLOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const PREP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// What identifies a decoded frame: its asset, start and size, and a fingerprint of its
/// pixels, so a frame decoded from another copy of the media, or by another decoder, is
/// never mistaken for one seen before.
pub(super) type FrameKey = (String, i64, u32, u32, u64);

/// Bytes of each plane a fingerprint reads, spread evenly over it.
const FINGERPRINT_SAMPLES: usize = 4096;

/// The key of `frame` of `asset`.
pub(super) fn frame_key(asset: &Id, frame: &VideoFrame) -> FrameKey {
    // FNV-1a over the plane sizes and evenly spaced bytes of each plane.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    };
    for plane in &frame.planes {
        let data = &plane.data;
        for byte in (data.len() as u64).to_le_bytes() {
            add(byte);
        }
        let step = (data.len() / FINGERPRINT_SAMPLES).max(1);
        for byte in data.iter().step_by(step) {
            add(*byte);
        }
    }
    (
        asset.to_string(),
        frame.pts.get(),
        frame.width,
        frame.height,
        hash,
    )
}

/// A flow between two frames: rg holds the first frame's flow to the second and ba the
/// second's to the first, in texture coordinates, unless `swapped` says they are stored the
/// other way round.
#[derive(Clone)]
pub(super) struct Flow {
    pub texture: wgpu::Texture,
    pub swapped: bool,
}

/// A neighbouring source frame of a layer and the flow from the shown frame to it.
pub(super) struct Side {
    pub flow: Flow,
    pub frame: VideoFrame,
    pub source: SourceRef,
    /// The neighbour converted to linear light, when the flow had to be estimated.
    pub picture: Option<wgpu::Texture>,
    /// The slot index the neighbour converts into.
    pub slot: usize,
}

/// Mean linear luminance of the shown frame and of its neighbours, nearest first.
pub(super) struct Levels {
    pub shown: f32,
    pub before: Vec<f32>,
    pub after: Vec<f32>,
}

/// What a layer's temporal effects read (ADR-0035).
#[derive(Default)]
pub(super) struct TemporalInputs {
    pub before: Option<Side>,
    pub after: Option<Side>,
    pub levels: Option<Levels>,
    /// Source frames per timeline frame.
    pub per_frame: f32,
}

/// Flows and brightness levels kept between frames.
#[derive(Default)]
pub(super) struct FlowCache {
    flows: Vec<(FrameKey, FrameKey, wgpu::Texture, u64)>,
    levels: std::collections::HashMap<FrameKey, f32>,
    tick: u64,
}

/// The flow pipelines, sharing one bind group layout.
pub(super) struct FlowPipelines {
    layout: wgpu::BindGroupLayout,
    prep: wgpu::RenderPipeline,
    upsample: wgpu::RenderPipeline,
    lk: wgpu::RenderPipeline,
    smooth: wgpu::RenderPipeline,
    correspond: wgpu::RenderPipeline,
    interpolate: wgpu::RenderPipeline,
    tiles: wgpu::RenderPipeline,
    motion_blur: wgpu::RenderPipeline,
    denoise: wgpu::RenderPipeline,
    blank: wgpu::Texture,
    blank_flow: wgpu::Texture,
    sampler: wgpu::Sampler,
}

/// The textures one flow pass reads; unset ones are bound to blanks.
#[derive(Default)]
struct Inputs<'a> {
    a: Option<&'a wgpu::TextureView>,
    b: Option<&'a wgpu::TextureView>,
    c: Option<&'a wgpu::TextureView>,
    flow_a: Option<&'a wgpu::TextureView>,
    flow_b: Option<&'a wgpu::TextureView>,
    flow_c: Option<&'a wgpu::TextureView>,
}

type Params = [[f32; 4]; 4];

impl FlowPipelines {
    pub(super) fn new(device: &wgpu::Device, common: &str) -> FlowPipelines {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flow"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{common}\n{}", include_str!("../shaders/flow.wgsl")).into(),
            ),
        });
        let texture = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flow"),
            entries: &[
                texture(0, true),
                texture(1, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(3, false),
                texture(4, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(6, true),
                texture(7, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flow"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry_point: &str, formats: &[wgpu::TextureFormat]| {
            let targets: Vec<Option<wgpu::ColorTargetState>> = formats
                .iter()
                .map(|&format| {
                    Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })
                })
                .collect();
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let blank = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("flow blank"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        FlowPipelines {
            prep: pipeline("fs_prep", &[PREP_FORMAT, PREP_FORMAT]),
            upsample: pipeline("fs_upsample", &[FLOW_FORMAT]),
            lk: pipeline("fs_lk", &[FLOW_FORMAT]),
            smooth: pipeline("fs_smooth", &[FLOW_FORMAT]),
            correspond: pipeline("fs_correspond", &[FLOW_FORMAT, PREP_FORMAT]),
            interpolate: pipeline("fs_interpolate", &[CANVAS_FORMAT]),
            tiles: pipeline("fs_tiles", &[FLOW_FORMAT]),
            motion_blur: pipeline("fs_motion_blur", &[CANVAS_FORMAT]),
            denoise: pipeline("fs_denoise_temporal", &[CANVAS_FORMAT]),
            layout,
            blank: blank(PREP_FORMAT),
            blank_flow: blank(FLOW_FORMAT),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("flow"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
        }
    }
}

/// The size of mip level `level` of a `size` texture.
fn level_size(size: (u32, u32), level: u32) -> (u32, u32) {
    ((size.0 >> level).max(1), (size.1 >> level).max(1))
}

/// The finest and coarsest levels a flow between `size` frames with `mips` levels is
/// estimated over.
pub(super) fn flow_levels(size: (u32, u32), mips: u32) -> (u32, u32) {
    let top = mips.max(1) - 1;
    let finest = (0..=top)
        .find(|&k| {
            let (w, h) = level_size(size, k);
            w.max(h) <= FLOW_MAX_SIDE
        })
        .unwrap_or(top);
    let mut coarsest = finest;
    while coarsest < top && coarsest - finest + 1 < FLOW_LEVELS {
        let (w, h) = level_size(size, coarsest + 1);
        if w.min(h) < FLOW_MIN_SIDE {
            break;
        }
        coarsest += 1;
    }
    (finest, coarsest)
}

impl Renderer {
    /// Runs flow pass `pipeline` over `inputs` with `params` into `targets`.
    fn flow_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        inputs: Inputs<'_>,
        params: &Params,
        targets: &[&wgpu::TextureView],
    ) {
        let pipelines = &self.flow;
        let blank = pipelines.blank.create_view(&Default::default());
        let blank_flow = pipelines.blank_flow.create_view(&Default::default());
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("flow"),
                contents: bytemuck::cast_slice(params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        fn view<'a>(
            v: Option<&'a wgpu::TextureView>,
            fallback: &'a wgpu::TextureView,
        ) -> wgpu::BindingResource<'a> {
            wgpu::BindingResource::TextureView(v.unwrap_or(fallback))
        }
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flow"),
            layout: &pipelines.layout,
            entries: &[
                entry(0, view(inputs.a, &blank)),
                entry(1, view(inputs.b, &blank)),
                entry(2, wgpu::BindingResource::Sampler(&pipelines.sampler)),
                entry(3, view(inputs.flow_a, &blank_flow)),
                entry(4, view(inputs.flow_b, &blank_flow)),
                entry(5, uniform.as_entire_binding()),
                entry(6, view(inputs.c, &blank)),
                entry(7, view(inputs.flow_c, &blank_flow)),
            ],
        });
        let attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = targets
            .iter()
            .map(|target| Some(attachment(target, None)))
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flow"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// A texture written by flow passes and read by later ones.
    fn flow_texture(
        &mut self,
        slot: Slot,
        size: (u32, u32),
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        self.texture(
            slot,
            size,
            format,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        )
    }

    /// The kept flow between the frames `first` and `second`, if any.
    pub(super) fn kept_flow(&mut self, first: &FrameKey, second: &FrameKey) -> Option<Flow> {
        let cache = &mut self.flows;
        cache.tick += 1;
        let tick = cache.tick;
        cache.flows.iter_mut().find_map(|(a, b, texture, used)| {
            let swapped = if a == first && b == second {
                false
            } else if a == second && b == first {
                true
            } else {
                return None;
            };
            *used = tick;
            Some(Flow {
                texture: texture.clone(),
                swapped,
            })
        })
    }

    /// Estimates the flow between `first` and `second`, linear-light textures of the same
    /// size with their mips, for layer slot `layer` and pair `pair`, and keeps it under the
    /// two frames' keys.
    pub(super) fn estimate_flow(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        (layer, pair): (usize, u8),
        (first, first_key): (&wgpu::Texture, FrameKey),
        (second, second_key): (&wgpu::Texture, FrameKey),
    ) -> Flow {
        let size = (first.width(), first.height());
        let (finest, coarsest) = flow_levels(size, first.mip_level_count());
        let first_view = first.create_view(&Default::default());
        let second_view = second.create_view(&Default::default());
        let result = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flow"),
            size: {
                let (w, h) = level_size(size, finest);
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                }
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FLOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let mut previous: Option<wgpu::Texture> = None;
        for level in (finest..=coarsest).rev() {
            let dims = level_size(size, level);
            let slot = |kind: u8| Slot::Flow {
                layer,
                pair,
                level: level as u8,
                kind,
            };
            let prep_a = self.flow_texture(slot(0), dims, PREP_FORMAT);
            let prep_b = self.flow_texture(slot(1), dims, PREP_FORMAT);
            let (prep_a_view, prep_b_view) = (
                prep_a.create_view(&Default::default()),
                prep_b.create_view(&Default::default()),
            );
            let mut params = [[0.0f32; 4]; 4];
            params[0][0] = level as f32;
            let pipelines = &self.flow;
            self.flow_pass(
                encoder,
                &pipelines.prep,
                Inputs {
                    a: Some(&first_view),
                    b: Some(&second_view),
                    ..Default::default()
                },
                &params,
                &[&prep_a_view, &prep_b_view],
            );
            let turns = [
                self.flow_texture(slot(2), dims, FLOW_FORMAT),
                self.flow_texture(slot(3), dims, FLOW_FORMAT),
            ];
            let views = [
                turns[0].create_view(&Default::default()),
                turns[1].create_view(&Default::default()),
            ];
            let above = previous
                .as_ref()
                .map(|t| t.create_view(&Default::default()));
            let mut params = [[0.0f32; 4]; 4];
            params[0][0] = if above.is_some() { 1.0 } else { 0.0 };
            self.flow_pass(
                encoder,
                &self.flow.upsample,
                Inputs {
                    a: Some(&prep_a_view),
                    flow_a: above.as_ref(),
                    ..Default::default()
                },
                &params,
                &[&views[0]],
            );
            let mut params = [[0.0f32; 4]; 4];
            params[0] = [WINDOW.0, WINDOW.1, DAMPING, ROBUST];
            params[1][0] = SUPPORT;
            for step in 0..ITERATIONS as usize {
                self.flow_pass(
                    encoder,
                    &self.flow.lk,
                    Inputs {
                        a: Some(&prep_a_view),
                        b: Some(&prep_b_view),
                        flow_a: Some(&views[step % 2]),
                        ..Default::default()
                    },
                    &params,
                    &[&views[(step + 1) % 2]],
                );
            }
            let refined = ITERATIONS as usize % 2;
            let smoothed = if level == finest {
                result.clone()
            } else {
                turns[1 - refined].clone()
            };
            let smoothed_view = smoothed.create_view(&Default::default());
            let mut params = [[0.0f32; 4]; 4];
            params[0] = [SMOOTHING.0, SMOOTHING.1, SMOOTHING.2, 0.0];
            self.flow_pass(
                encoder,
                &self.flow.smooth,
                Inputs {
                    a: Some(&prep_a_view),
                    b: Some(&prep_b_view),
                    flow_a: Some(&views[refined]),
                    ..Default::default()
                },
                &params,
                &[&smoothed_view],
            );
            previous = Some(smoothed);
        }
        let cache = &mut self.flows;
        cache.tick += 1;
        if cache.flows.len() >= FLOW_CACHE
            && let Some(oldest) = (0..cache.flows.len()).min_by_key(|&i| cache.flows[i].3)
        {
            cache.flows.swap_remove(oldest);
        }
        cache
            .flows
            .push((first_key, second_key, result.clone(), cache.tick));
        Flow {
            texture: result,
            swapped: false,
        }
    }

    /// The flow between `first` and `second` (frames of layer slot `layer`, pair `pair`),
    /// kept or estimated. A pair is always estimated earlier frame first and read the other
    /// way round when asked for backwards, so a frame renders the same whichever frames
    /// were rendered before it.
    pub(super) fn flow_between(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        slots: (usize, u8),
        first: (&wgpu::Texture, FrameKey),
        second: (&wgpu::Texture, FrameKey),
    ) -> Flow {
        if let Some(flow) = self.kept_flow(&first.1, &second.1) {
            return flow;
        }
        if first.1 <= second.1 {
            self.estimate_flow(encoder, slots, first, second)
        } else {
            Flow {
                swapped: true,
                ..self.estimate_flow(encoder, slots, second, first)
            }
        }
    }

    /// The in-between frame `amount` of the way from `first` to `second` along their flow,
    /// mipmapped like a layer texture: smooth slow motion that keeps moving things sharp.
    pub(super) fn interpolate_frames(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        (first, first_key): (&wgpu::Texture, FrameKey),
        (second, second_key): (&wgpu::Texture, FrameKey),
        amount: f32,
    ) -> wgpu::Texture {
        let flow = self.flow_between(
            encoder,
            (index, 0),
            (first, first_key),
            (second, second_key),
        );
        // A flow kept the other way round is used as it is, from the second frame.
        let (from, to, t) = if flow.swapped {
            (second, first, 1.0 - amount)
        } else {
            (first, second, amount)
        };
        let dims = (flow.texture.width(), flow.texture.height());
        let offsets = self.flow_texture(
            Slot::Correspond {
                layer: index,
                kind: 0,
            },
            dims,
            FLOW_FORMAT,
        );
        let weights = self.flow_texture(
            Slot::Correspond {
                layer: index,
                kind: 1,
            },
            dims,
            PREP_FORMAT,
        );
        let (offsets_view, weights_view) = (
            offsets.create_view(&Default::default()),
            weights.create_view(&Default::default()),
        );
        let flow_view = flow.texture.create_view(&Default::default());
        let mut params = [[0.0f32; 4]; 4];
        // Errors of a flow pixel cut a frame's weight to a third; candidates are looked for
        // 3 and 12 flow pixels away.
        params[0] = [t, 1.0, 3.0, 12.0];
        self.flow_pass(
            encoder,
            &self.flow.correspond,
            Inputs {
                flow_a: Some(&flow_view),
                ..Default::default()
            },
            &params,
            &[&offsets_view, &weights_view],
        );
        let size = (first.width(), first.height());
        let mips = first.mip_level_count();
        let mixed = self.texture(
            Slot::Between(index),
            size,
            CANVAS_FORMAT,
            mips,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let target = mixed.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..Default::default()
        });
        let from_view = from.create_view(&Default::default());
        let to_view = to.create_view(&Default::default());
        self.flow_pass(
            encoder,
            &self.flow.interpolate,
            Inputs {
                a: Some(&from_view),
                b: Some(&to_view),
                c: Some(&weights_view),
                flow_a: Some(&offsets_view),
                ..Default::default()
            },
            &params,
            &[&target],
        );
        self.generate_mips(encoder, &mixed, mips);
        mixed
    }

    /// Motion blur of `input` into `target` along the flows to the neighbours in
    /// `temporal`, with the shutter open `shutter` of a frame and `samples` samples per
    /// pixel; a copy when nothing moves.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn motion_blur(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        layer: usize,
        input: &wgpu::Texture,
        temporal: Option<&TemporalInputs>,
        shutter: f32,
        samples: usize,
        target: &wgpu::TextureView,
    ) {
        let input_view = input.create_view(&Default::default());
        let flows = temporal.map(|t| {
            (
                t.after.as_ref().map(|s| &s.flow),
                t.before.as_ref().map(|s| &s.flow),
                t.per_frame,
            )
        });
        let Some((next, previous, per_frame)) = flows else {
            self.copy_into(encoder, &input_view, target);
            return;
        };
        let half = per_frame * shutter / 2.0;
        let Some(any) = next.or(previous).filter(|_| half > 0.0) else {
            self.copy_into(encoder, &input_view, target);
            return;
        };
        let size = (input.width(), input.height());
        let flow_size = (any.texture.width(), any.texture.height());
        let next_view = next.unwrap_or(any).texture.create_view(&Default::default());
        let previous_view = previous
            .unwrap_or(any)
            .texture
            .create_view(&Default::default());
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let mut params = [[0.0f32; 4]; 4];
        params[0] = [half, flag(next.is_some()), flag(previous.is_some()), 0.0];
        params[1] = [
            flag(next.is_some_and(|f| f.swapped)),
            flag(previous.is_some_and(|f| f.swapped)),
            size.0 as f32,
            size.1 as f32,
        ];
        // A tile spans TILE flow pixels: the most a pixel moves, in picture pixels.
        let most = TILE as f32 * size.0 as f32 / flow_size.0.max(1) as f32;
        params[2] = [TILE as f32, samples as f32, most, 0.0];
        let tiles = self.flow_texture(
            Slot::Tiles(layer),
            (flow_size.0.div_ceil(TILE), flow_size.1.div_ceil(TILE)),
            FLOW_FORMAT,
        );
        let tiles_view = tiles.create_view(&Default::default());
        self.flow_pass(
            encoder,
            &self.flow.tiles,
            Inputs {
                flow_a: Some(&next_view),
                flow_b: Some(&previous_view),
                ..Default::default()
            },
            &params,
            &[&tiles_view],
        );
        self.flow_pass(
            encoder,
            &self.flow.motion_blur,
            Inputs {
                a: Some(&input_view),
                flow_a: Some(&next_view),
                flow_b: Some(&previous_view),
                flow_c: Some(&tiles_view),
                ..Default::default()
            },
            &params,
            &[target],
        );
    }

    /// `input` averaged with the neighbouring pictures `before` and `after` warped along
    /// their flows, into `target`, with colour differences above about `sigma`
    /// (sRGB-encoded) left out.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn denoise_temporal(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        before: Option<(&wgpu::TextureView, &Flow)>,
        after: Option<(&wgpu::TextureView, &Flow)>,
        sigma: f32,
        target: &wgpu::TextureView,
    ) {
        let input_view = input.create_view(&Default::default());
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let before_flow = before.map(|(_, f)| f.texture.create_view(&Default::default()));
        let after_flow = after.map(|(_, f)| f.texture.create_view(&Default::default()));
        let mut params = [[0.0f32; 4]; 4];
        params[0] = [sigma, flag(before.is_some()), flag(after.is_some()), 1.0];
        params[1] = [
            flag(before.is_some_and(|(_, f)| f.swapped)),
            flag(after.is_some_and(|(_, f)| f.swapped)),
            0.0,
            0.0,
        ];
        self.flow_pass(
            encoder,
            &self.flow.denoise,
            Inputs {
                a: Some(&input_view),
                b: before.map(|(picture, _)| picture),
                c: after.map(|(picture, _)| picture),
                flow_a: before_flow.as_ref(),
                flow_b: after_flow.as_ref(),
                ..Default::default()
            },
            &params,
            &[target],
        );
    }

    /// The mean linear luminance of `frame` of `asset`, kept for the next frames.
    pub(super) fn level(&mut self, asset: &Id, frame: &VideoFrame) -> f32 {
        let key = frame_key(asset, frame);
        if let Some(level) = self.flows.levels.get(&key) {
            return *level;
        }
        if self.flows.levels.len() >= LEVEL_CACHE {
            self.flows.levels.clear();
        }
        let level = crate::effects::mean_luminance(frame);
        self.flows.levels.insert(key, level);
        level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_levels_stay_small_and_reach_far() {
        // A 320x180 source: flow at full size, refined from 20x11 up.
        assert_eq!(flow_levels((320, 180), 9), (0, 4));
        // 1080p estimates at 960x540 and refines over seven levels.
        assert_eq!(flow_levels((1920, 1080), 11), (1, 7));
        // 4K starts at 960x540 too.
        assert_eq!(flow_levels((3840, 2160), 12), (2, 8));
        // Without mips, one level.
        assert_eq!(flow_levels((1920, 1080), 1), (0, 0));
    }
}
