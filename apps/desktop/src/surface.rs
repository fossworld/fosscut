//! The window's GPU surface (ADR-0010, path A, from the milestone 2 spike): the preview
//! frame is rendered into an offscreen texture by the engine, then drawn into the hole the
//! webview leaves, and the rest of the surface is cleared dark.

use fosscut_render::Renderer;

use crate::preview::Screen;

/// The format preview frames render in before they are drawn to the window.
pub const FRAME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Behind the hole, around the frame.
const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.035,
    g: 0.036,
    b: 0.042,
    a: 1.0,
};

const BLIT: &str = r"
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
struct Out { @builtin(position) position: vec4f, @location(0) uv: vec2f }
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.position = vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}
@fragment fn fs_main(in: Out) -> @location(0) vec4f {
    return textureSampleLevel(image, image_sampler, in.uv, 0.0);
}
";

/// The window surface, the pipeline that draws a frame into it, and the frame texture.
pub struct Surface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    blit: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    window: tauri::WebviewWindow,
    frame: Option<(wgpu::Texture, (u32, u32))>,
}

impl Surface {
    /// A GPU device on an adapter that can present to `window`, its surface, and a renderer
    /// on that device.
    pub fn create(window: tauri::WebviewWindow) -> Result<(Surface, Renderer), String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("fosscut-desktop"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or("the window surface supports no format")?;
        let size = window
            .inner_size()
            .unwrap_or(tauri::PhysicalSize::new(1280, 800));
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&device, &config);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });
        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let renderer = Renderer::with_device(adapter.get_info(), device, queue);
        Ok((
            Surface {
                surface,
                config,
                blit,
                sampler,
                window,
                frame: None,
            },
            renderer,
        ))
    }
}

impl Screen for Surface {
    /// Follows the window's size.
    fn resize(&mut self, renderer: &Renderer) {
        if let Ok(size) = self.window.inner_size() {
            self.config.width = size.width.max(1);
            self.config.height = size.height.max(1);
            self.surface.configure(renderer.device(), &self.config);
        }
    }

    /// The texture a frame of `size` renders into, kept while the size stays the same.
    fn frame_view(&mut self, renderer: &Renderer, size: (u32, u32)) -> wgpu::TextureView {
        let texture = match self.frame.take() {
            Some((texture, kept)) if kept == size => texture,
            _ => renderer.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("preview frame"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FRAME_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }),
        };
        let view = texture.create_view(&Default::default());
        self.frame = Some((texture, size));
        view
    }

    /// Clears the window and draws `frame` into `rect` (physical pixels). Nothing is
    /// presented while the window is minimized.
    fn present(&mut self, renderer: &Renderer, frame: Option<&wgpu::TextureView>, rect: [f32; 4]) {
        if self.window.is_minimized().unwrap_or(false) {
            return;
        }
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output)
            | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
            _ => {
                self.resize(renderer);
                return;
            }
        };
        let target = output.texture.create_view(&Default::default());
        let device = renderer.device();
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BACKGROUND),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let (width, height) = (self.config.width as f32, self.config.height as f32);
            let [x, y, w, h] = rect;
            let (x, y) = (x.clamp(0.0, width - 1.0), y.clamp(0.0, height - 1.0));
            let (w, h) = (w.min(width - x), h.min(height - y));
            if let Some(frame) = frame
                && w >= 1.0
                && h >= 1.0
            {
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("blit"),
                    layout: &self.blit.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(frame),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                pass.set_viewport(x, y, w, h, 0.0, 1.0);
                pass.set_pipeline(&self.blit);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        renderer.queue().submit([encoder.finish()]);
        renderer.queue().present(output);
    }
}
