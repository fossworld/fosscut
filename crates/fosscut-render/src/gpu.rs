//! The wgpu compositor (ADR-0008): executes [`RenderPlan`]s on the GPU.
//!
//! Per frame: each layer's planes are uploaded and converted to linear RGB at source size
//! (`convert.wgsl`), mipmapped in linear light (`mip.wgsl`), drawn over a linear
//! `rgba16float` canvas with its transform, LUT, mask and opacity (`composite.wgsl`), and
//! the canvas is encoded with the output transfer function into the target
//! (`encode.wgsl`). Textures are cached between frames by size and layer slot.

use std::collections::HashMap;
use std::sync::Arc;

use fosscut_media::{PixelFormat, VideoFrame};
use wgpu::util::DeviceExt;

use crate::caption::{CaptionLayer, CaptionRaster, PageLook, anchor, evaluate};
use crate::color::{TransferFunction, yuv_to_rgb};
use crate::geometry::{Vertex, layer_quad, placed_quad};
use crate::lut::Lut3d;
use crate::plan::{LayerSource, MaskShape, RenderPlan, SourceRef};
use crate::text::{TextEngine, TextRaster, linear_color};
use crate::{FrameProvider, RenderError};

const COMMON: &str = include_str!("shaders/common.wgsl");
const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Samples per pixel when compositing, so rotated and scaled layer edges are antialiased.
/// WebGPU guarantees 4x multisampling for `rgba16float` render targets.
const SAMPLES: u32 = 4;
const IMAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// An 8-bit RGBA image read back from the GPU, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, RGBA.
    pub rgba: Vec<u8>,
}

impl Image {
    /// The RGBA value of a pixel.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// Peak signal-to-noise ratio over RGB in decibels: infinite for identical images, and
    /// above about 40 dB when differences are invisible. `None` when sizes differ.
    pub fn psnr(&self, other: &Image) -> Option<f64> {
        if (self.width, self.height) != (other.width, other.height) {
            return None;
        }
        let (mut sum, mut count) = (0.0_f64, 0_u64);
        for (a, b) in self
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(other.rgba.as_chunks::<4>().0)
        {
            for c in 0..3 {
                let d = f64::from(a[c]) - f64::from(b[c]);
                sum += d * d;
                count += 1;
            }
        }
        let mse = sum / count.max(1) as f64;
        Some(if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0 * 255.0 / mse).log10()
        })
    }
}

/// Where a texture is used, so each layer keeps its own textures between frames.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum Slot {
    Plane { layer: usize, plane: usize },
    Layer(usize),
    Text(usize),
    Canvas,
    CanvasSamples,
    Readback,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct TextureKey {
    slot: Slot,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    mips: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ConvertUniform {
    rows: [[f32; 4]; 3],
    mode: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LayerUniform {
    params: [f32; 4],
    mask: [f32; 4],
    lut: [f32; 4],
}

/// The GPU compositor. Create it once and render many frames with it.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    convert: wgpu::RenderPipeline,
    mip: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    encode_module: wgpu::ShaderModule,
    encoders: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    linear: wgpu::Sampler,
    no_lut: wgpu::Texture,
    luts: HashMap<usize, (Arc<Lut3d>, wgpu::Texture)>,
    textures: HashMap<TextureKey, wgpu::Texture>,
    text: TextEngine,
    text_cache: HashMap<String, Arc<TextRaster>>,
    caption_cache: HashMap<String, Arc<CaptionRaster>>,
}

/// Rasterized text kept between frames; titles usually stay the same for many frames.
const TEXT_CACHE_LIMIT: usize = 64;

impl Renderer {
    /// A renderer on the default GPU adapter without a window, honouring wgpu's
    /// `WGPU_BACKEND` and `WGPU_ADAPTER_NAME` environment variables. Software adapters
    /// (Mesa lavapipe, WARP) work too.
    pub fn headless() -> Result<Renderer, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .map_err(|e| RenderError::NoAdapter {
            message: e.to_string(),
        })?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("fosscut-render"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|e| RenderError::Device {
            message: e.to_string(),
        })?;
        Ok(Renderer::with_device(adapter.get_info(), device, queue))
    }

    /// A renderer on a device the caller created, for example one shared with a window
    /// surface.
    pub fn with_device(
        info: wgpu::AdapterInfo,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Renderer {
        let module = |label: &str, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(format!("{COMMON}\n{source}").into()),
            })
        };
        let convert_module = module("convert", include_str!("shaders/convert.wgsl"));
        let mip_module = module("mip", include_str!("shaders/mip.wgsl"));
        let composite_module = module("composite", include_str!("shaders/composite.wgsl"));
        let encode_module = module("encode", include_str!("shaders/encode.wgsl"));

        let full_screen = |module: &wgpu::ShaderModule, label: &str, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: None,
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
                        format: CANVAS_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let convert = full_screen(&convert_module, "convert", None);
        let mip = full_screen(&mip_module, "mip", None);

        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2];
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("composite"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &composite_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: CANVAS_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let no_lut = lut_texture(&device, &queue, &Lut3d::identity(2));
        Renderer {
            device,
            queue,
            info,
            convert,
            mip,
            composite,
            encode_module,
            encoders: HashMap::new(),
            linear,
            no_lut,
            luts: HashMap::new(),
            textures: HashMap::new(),
            text: TextEngine::new(),
            text_cache: HashMap::new(),
            caption_cache: HashMap::new(),
        }
    }

    /// The adapter this renderer runs on.
    pub fn adapter(&self) -> &wgpu::AdapterInfo {
        &self.info
    }

    /// The device, for callers that create surfaces or targets.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The queue, for callers that present surfaces.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Renders `plan` into `target`, a `size` view of a `format` texture, encoded with
    /// `transfer`. Used for window surfaces and readback targets alike.
    pub fn render_to(
        &mut self,
        plan: &RenderPlan,
        frames: &mut dyn FrameProvider,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        transfer: TransferFunction,
    ) -> Result<(), RenderError> {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        self.record(plan, frames, &mut encoder, target, format, size, transfer)?;
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Renders `plan` at `size` and reads the 8-bit RGBA result back.
    pub fn render_image(
        &mut self,
        plan: &RenderPlan,
        frames: &mut dyn FrameProvider,
        size: (u32, u32),
        transfer: TransferFunction,
    ) -> Result<Image, RenderError> {
        let target = self.texture(
            Slot::Readback,
            size,
            IMAGE_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let view = target.create_view(&Default::default());
        let padded_row = (size.0 * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded_row * size.1),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame and readback"),
            });
        self.record(
            plan,
            frames,
            &mut encoder,
            &view,
            IMAGE_FORMAT,
            size,
            transfer,
        )?;
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(size.1),
                },
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|e| RenderError::Readback {
                message: e.to_string(),
            })?;
        receiver
            .recv()
            .map_err(|e| RenderError::Readback {
                message: e.to_string(),
            })?
            .map_err(|e| RenderError::Readback {
                message: e.to_string(),
            })?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|e| RenderError::Readback {
                message: e.to_string(),
            })?;
        let row = (size.0 * 4) as usize;
        let mut rgba = Vec::with_capacity(row * size.1 as usize);
        for chunk in mapped.chunks(padded_row as usize).take(size.1 as usize) {
            rgba.extend_from_slice(&chunk[..row]);
        }
        drop(mapped);
        buffer.unmap();
        Ok(Image {
            width: size.0,
            height: size.1,
            rgba,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        plan: &RenderPlan,
        frames: &mut dyn FrameProvider,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        transfer: TransferFunction,
    ) -> Result<(), RenderError> {
        let mut draws = Vec::with_capacity(plan.layers.len());
        for (index, layer) in plan.layers.iter().enumerate() {
            let mut opacity = layer.opacity;
            let (layer_view, vertices) = match &layer.source {
                LayerSource::Frame(source) => {
                    let frame = frames.frame(source)?;
                    let view = self.convert_layer(encoder, index, &frame, source)?;
                    let vertices = layer_quad(
                        plan.canvas,
                        (frame.width, frame.height),
                        layer.rotation,
                        &layer.transform,
                    );
                    (view, vertices)
                }
                LayerSource::Text(content) => {
                    // Rasterize at the output resolution: output pixels per canvas pixel.
                    let scale = size.1 as f32 / plan.canvas.1.max(1) as f32;
                    let raster = self.rasterize(content, plan.canvas, scale);
                    if raster.is_empty() {
                        continue;
                    }
                    let color = linear_color(content.style.color.as_str());
                    let view = self.upload_text(encoder, index, &raster, color);
                    let scale = f64::from(scale);
                    let natural = (
                        f64::from(raster.width) / scale,
                        f64::from(raster.height) / scale,
                    );
                    // Centre the exact layout box, not the pixel-cropped raster.
                    let [x, y, w, h] = raster.layout_box.map(f64::from);
                    let offset = (
                        (x + w / 2.0 - f64::from(raster.width) / 2.0) / scale,
                        (y + h / 2.0 - f64::from(raster.height) / 2.0) / scale,
                    );
                    (
                        view,
                        placed_quad(plan.canvas, natural, offset, &layer.transform),
                    )
                }
                LayerSource::Caption(caption) => {
                    let scale = size.1 as f32 / plan.canvas.1.max(1) as f32;
                    let look = evaluate(caption);
                    if look.alpha <= 0.0 {
                        continue;
                    }
                    let raster = self.caption_raster(caption, &look, plan.canvas, scale);
                    if raster.is_empty() {
                        continue;
                    }
                    opacity *= look.alpha;
                    let view = self.upload_caption(encoder, index, &raster);
                    let scale = f64::from(scale);
                    let natural = (
                        f64::from(raster.width) / scale,
                        f64::from(raster.height) / scale,
                    );
                    // The page sits in the safe area; the clip's transform moves it from
                    // there, and a page pop scales it around its own centre.
                    let (x, y) = anchor(
                        plan.canvas,
                        caption.style.placement,
                        f64::from(raster.lines_height),
                    );
                    let (w, h) = (f64::from(plan.canvas.0), f64::from(plan.canvas.1));
                    let mut placed = layer.transform.clone();
                    placed.position[0] += (x - w / 2.0) / w;
                    placed.position[1] += (y - h / 2.0) / h;
                    placed.scale *= f64::from(look.scale);
                    (view, placed_quad(plan.canvas, natural, (0.0, 0.0), &placed))
                }
            };
            let (mask_kind, mask) = match &layer.mask {
                Some(mask) => (
                    match mask.shape {
                        MaskShape::Rectangle => 1.0,
                        MaskShape::Ellipse => 2.0,
                    },
                    Some(mask),
                ),
                None => (0.0, None),
            };
            let uniform = LayerUniform {
                params: [
                    opacity,
                    mask_kind,
                    mask.map_or(0.0, |m| m.feather.clamp(0.0, 1.0)),
                    mask.map_or(0.0, |m| if m.invert { 1.0 } else { 0.0 }),
                ],
                mask: mask.map_or([0.0; 4], |m| {
                    [m.center[0], m.center[1], m.size[0], m.size[1]]
                }),
                lut: [
                    layer.lut.as_ref().map_or(0.0, |l| l.size as f32),
                    0.0,
                    0.0,
                    0.0,
                ],
            };
            let lut_view = match &layer.lut {
                Some(lut) => self.lut_view(lut),
                None => self.no_lut.create_view(&Default::default()),
            };
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("layer"),
                layout: &self.composite.get_bind_group_layout(0),
                entries: &[
                    entry(0, wgpu::BindingResource::TextureView(&layer_view)),
                    entry(1, wgpu::BindingResource::Sampler(&self.linear)),
                    entry(2, wgpu::BindingResource::TextureView(&lut_view)),
                    entry(3, wgpu::BindingResource::Sampler(&self.linear)),
                    entry(4, self.uniform(&uniform).as_entire_binding()),
                ],
            });
            let vertex_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("layer quad"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            draws.push((bind_group, vertex_buffer));
        }

        let canvas = self.texture(
            Slot::Canvas,
            size,
            CANVAS_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let canvas_view = canvas.create_view(&Default::default());
        let samples = self.multisampled_canvas(size);
        let samples_view = samples.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &samples_view,
                    depth_slice: None,
                    resolve_target: Some(&canvas_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.composite);
            for (bind_group, vertex_buffer) in &draws {
                pass.set_bind_group(0, bind_group, &[]);
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                pass.draw(0..4, 0..1);
            }
        }

        let pipeline = self.encoder_pipeline(format);
        let output = self.uniform(&[transfer as u32, 0, 0, 0]);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("encode"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, wgpu::BindingResource::TextureView(&canvas_view)),
                entry(1, output.as_entire_binding()),
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("encode"),
            color_attachments: &[Some(attachment(target, None))],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }

    /// Uploads a frame and converts it to a mipmapped linear-light texture.
    fn convert_layer(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        frame: &VideoFrame,
        source: &SourceRef,
    ) -> Result<wgpu::TextureView, RenderError> {
        let matrix = yuv_to_rgb(&frame.color)?;
        let transfer = TransferFunction::for_source(frame.color.transfer)?;
        if frame.planes.len() != frame.format.plane_count() || frame.width == 0 || frame.height == 0
        {
            return Err(RenderError::MissingFrame {
                asset: source.asset.to_string(),
                message: "the frame has no image data".to_string(),
            });
        }
        let mut planes = Vec::with_capacity(3);
        for (plane_index, plane) in frame.planes.iter().enumerate() {
            let (row_bytes, rows) = frame
                .format
                .plane_size(plane_index, frame.width, frame.height);
            let (format, width) = match (frame.format, plane_index) {
                (PixelFormat::Nv12, 1) => (wgpu::TextureFormat::Rg8Unorm, row_bytes / 2),
                _ => (wgpu::TextureFormat::R8Unorm, row_bytes),
            };
            let texture = self.texture(
                Slot::Plane {
                    layer: index,
                    plane: plane_index,
                },
                (width as u32, rows as u32),
                format,
                1,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            self.queue.write_texture(
                texture.as_image_copy(),
                &plane.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(plane.stride as u32),
                    rows_per_image: Some(rows as u32),
                },
                texture.size(),
            );
            planes.push(texture.create_view(&Default::default()));
        }
        let chroma_b = planes.get(2).unwrap_or(&planes[1]).clone();

        let mips = (frame.width.max(frame.height) as f32).log2().floor() as u32 + 1;
        let layer = self.texture(
            Slot::Layer(index),
            (frame.width, frame.height),
            CANVAS_FORMAT,
            mips,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let uniform = ConvertUniform {
            rows: matrix,
            mode: [
                u32::from(frame.format == PixelFormat::Yuv420p),
                transfer as u32,
                0,
                0,
            ],
        };
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("convert"),
            layout: &self.convert.get_bind_group_layout(0),
            entries: &[
                entry(0, wgpu::BindingResource::TextureView(&planes[0])),
                entry(1, wgpu::BindingResource::TextureView(&planes[1])),
                entry(2, wgpu::BindingResource::TextureView(&chroma_b)),
                entry(3, wgpu::BindingResource::Sampler(&self.linear)),
                entry(4, self.uniform(&uniform).as_entire_binding()),
            ],
        });
        let level = |mip: u32| {
            layer.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: mip,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        {
            let base = level(0);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("convert"),
                color_attachments: &[Some(attachment(&base, None))],
                ..Default::default()
            });
            pass.set_pipeline(&self.convert);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.generate_mips(encoder, &layer, mips);
        Ok(layer.create_view(&Default::default()))
    }

    /// Fills mip levels 1.. of `texture` from level 0 with the box-filter pass.
    fn generate_mips(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        mips: u32,
    ) {
        let level = |mip: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: mip,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        for mip in 1..mips {
            let source = level(mip - 1);
            let target = level(mip);
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mip"),
                layout: &self.mip.get_bind_group_layout(0),
                entries: &[
                    entry(0, wgpu::BindingResource::TextureView(&source)),
                    entry(1, wgpu::BindingResource::Sampler(&self.linear)),
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mip"),
                color_attachments: &[Some(attachment(&target, None))],
                ..Default::default()
            });
            pass.set_pipeline(&self.mip);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Rasterized text, cached by content, canvas and scale.
    fn rasterize(
        &mut self,
        content: &fosscut_model::TextContent,
        canvas: (u32, u32),
        scale: f32,
    ) -> Arc<TextRaster> {
        let key = format!("{content:?}|{}x{}|{}", canvas.0, canvas.1, scale.to_bits());
        if let Some(raster) = self.text_cache.get(&key) {
            return raster.clone();
        }
        if self.text_cache.len() >= TEXT_CACHE_LIMIT {
            self.text_cache.clear();
        }
        let raster = Arc::new(self.text.rasterize(content, canvas, scale));
        self.text_cache.insert(key, raster.clone());
        raster
    }

    /// A composed caption page, cached by its words, look and output size.
    fn caption_raster(
        &mut self,
        layer: &CaptionLayer,
        look: &PageLook,
        canvas: (u32, u32),
        scale: f32,
    ) -> Arc<CaptionRaster> {
        let key = format!(
            "{:?}|{:?}|{look:?}|{}x{}|{}",
            layer.style,
            layer.content,
            canvas.0,
            canvas.1,
            scale.to_bits()
        );
        if let Some(raster) = self.caption_cache.get(&key) {
            return raster.clone();
        }
        if self.caption_cache.len() >= TEXT_CACHE_LIMIT {
            self.caption_cache.clear();
        }
        let raster = Arc::new(self.text.caption(layer, look, canvas, scale));
        self.caption_cache.insert(key, raster.clone());
        raster
    }

    /// Uploads a composed caption page (premultiplied linear RGBA) and mipmaps it.
    fn upload_caption(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        raster: &CaptionRaster,
    ) -> wgpu::TextureView {
        let texels: Vec<u16> = raster
            .pixels
            .iter()
            .flatten()
            .map(|&v| half::f16::from_f32(v).to_bits())
            .collect();
        self.upload_texels(encoder, index, (raster.width, raster.height), &texels)
    }

    /// Uploads glyph coverage as premultiplied linear color and mipmaps it.
    fn upload_text(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        raster: &TextRaster,
        color: [f32; 4],
    ) -> wgpu::TextureView {
        let texels: Vec<u16> = raster
            .coverage
            .iter()
            .flat_map(|&c| {
                let alpha = f32::from(c) / 255.0 * color[3];
                [color[0] * alpha, color[1] * alpha, color[2] * alpha, alpha]
            })
            .map(|v| half::f16::from_f32(v).to_bits())
            .collect();
        self.upload_texels(encoder, index, (raster.width, raster.height), &texels)
    }

    /// Uploads premultiplied linear RGBA16F texels into the text slot of layer `index` and
    /// mipmaps them.
    fn upload_texels(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        (width, height): (u32, u32),
        texels: &[u16],
    ) -> wgpu::TextureView {
        let mips = (width.max(height) as f32).log2().floor() as u32 + 1;
        let texture = self.texture(
            Slot::Text(index),
            (width, height),
            CANVAS_FORMAT,
            mips,
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
        );
        self.queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 8),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.generate_mips(encoder, &texture, mips);
        texture.create_view(&Default::default())
    }

    fn texture(
        &mut self,
        slot: Slot,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        mips: u32,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        let key = TextureKey {
            slot,
            width: size.0,
            height: size.1,
            format,
            mips,
        };
        // A slot keeps one texture: drop textures of other sizes when it changes size.
        self.textures
            .retain(|existing, _| existing.slot != slot || *existing == key);
        self.textures
            .entry(key)
            .or_insert_with(|| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("fosscut-render"),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: mips,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            })
            .clone()
    }

    /// The multisampled composite target, resolved into the canvas after each frame.
    fn multisampled_canvas(&mut self, size: (u32, u32)) -> wgpu::Texture {
        let key = TextureKey {
            slot: Slot::CanvasSamples,
            width: size.0,
            height: size.1,
            format: CANVAS_FORMAT,
            mips: 1,
        };
        self.textures
            .retain(|existing, _| existing.slot != Slot::CanvasSamples || *existing == key);
        self.textures
            .entry(key)
            .or_insert_with(|| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("canvas samples"),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: SAMPLES,
                    dimension: wgpu::TextureDimension::D2,
                    format: CANVAS_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
            })
            .clone()
    }

    fn lut_view(&mut self, lut: &Arc<Lut3d>) -> wgpu::TextureView {
        let key = Arc::as_ptr(lut) as usize;
        let device = &self.device;
        let queue = &self.queue;
        self.luts
            .entry(key)
            .or_insert_with(|| (lut.clone(), lut_texture(device, queue, lut)))
            .1
            .create_view(&Default::default())
    }

    fn uniform<T: bytemuck::Pod>(&self, value: &T) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("uniform"),
                contents: bytemuck::bytes_of(value),
                usage: wgpu::BufferUsages::UNIFORM,
            })
    }

    fn encoder_pipeline(&mut self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let device = &self.device;
        let module = &self.encode_module;
        self.encoders
            .entry(format)
            .or_insert_with(|| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("encode"),
                    layout: None,
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
            })
            .clone()
    }
}

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

fn attachment(
    view: &wgpu::TextureView,
    clear: Option<wgpu::Color>,
) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: clear.map_or(
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                wgpu::LoadOp::Clear,
            ),
            store: wgpu::StoreOp::Store,
        },
    }
}

/// A filterable `rgba16float` 3D texture holding a LUT.
fn lut_texture(device: &wgpu::Device, queue: &wgpu::Queue, lut: &Lut3d) -> wgpu::Texture {
    let data: Vec<u16> = lut
        .data
        .iter()
        .flat_map(|[r, g, b]| [*r, *g, *b, 1.0])
        .map(|v| half::f16::from_f32(v).to_bits())
        .collect();
    device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("lut"),
            size: wgpu::Extent3d {
                width: lut.size,
                height: lut.size,
                depth_or_array_layers: lut.size,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytemuck::cast_slice(&data),
    )
}
