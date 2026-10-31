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
            // A zoomed preview may reach past the window's edges: the frame is drawn whole
            // and clipped to the window, never squeezed into the part that shows.
            let clip = visible([x, y, w, h], (width, height));
            let limit = device.limits().max_texture_dimension_2d as f32;
            if let Some(frame) = frame
                && let Some([left, top, shown_w, shown_h]) = clip
                && w <= limit
                && h <= limit
                && x >= -limit
                && y >= -limit
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
                pass.set_scissor_rect(left, top, shown_w, shown_h);
                pass.set_pipeline(&self.blit);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        renderer.queue().submit([encoder.finish()]);
        renderer.queue().present(output);
    }
}

/// The part of `rect` (x, y, width, height in physical pixels) inside a window of `size`,
/// in whole pixels, as a scissor rectangle; `None` when less than a pixel of it shows.
fn visible(rect: [f32; 4], size: (f32, f32)) -> Option<[u32; 4]> {
    let [x, y, w, h] = rect;
    if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
        return None;
    }
    let left = x.max(0.0).floor();
    let top = y.max(0.0).floor();
    let right = (x + w).min(size.0).ceil().min(size.0);
    let bottom = (y + h).min(size.1).ceil().min(size.1);
    if right - left < 1.0 || bottom - top < 1.0 {
        return None;
    }
    Some([
        left as u32,
        top as u32,
        (right - left) as u32,
        (bottom - top) as u32,
    ])
}

#[cfg(test)]
mod tests {
    use super::visible;

    #[test]
    fn a_frame_inside_the_window_shows_whole() {
        assert_eq!(
            visible([10.0, 20.0, 300.0, 200.0], (800.0, 600.0)),
            Some([10, 20, 300, 200])
        );
    }

    #[test]
    fn a_zoomed_frame_past_the_edges_is_clipped_to_the_window_not_squeezed() {
        // Twice the window's size, centred: everything shows, the frame keeps its size.
        assert_eq!(
            visible([-400.0, -300.0, 1600.0, 1200.0], (800.0, 600.0)),
            Some([0, 0, 800, 600])
        );
        // Past the right and bottom edges only.
        assert_eq!(
            visible([700.0, 500.0, 300.0, 300.0], (800.0, 600.0)),
            Some([700, 500, 100, 100])
        );
        // Fractions round outwards, within the window.
        assert_eq!(
            visible([0.5, 0.5, 99.2, 99.2], (800.0, 600.0)),
            Some([0, 0, 100, 100])
        );
    }

    #[test]
    fn a_frame_outside_the_window_or_without_a_size_draws_nothing() {
        assert_eq!(visible([900.0, 0.0, 100.0, 100.0], (800.0, 600.0)), None);
        assert_eq!(visible([-200.0, 0.0, 150.0, 100.0], (800.0, 600.0)), None);
        assert_eq!(visible([10.0, 10.0, 0.0, 0.0], (800.0, 600.0)), None);
        assert_eq!(visible([f32::NAN, 0.0, 10.0, 10.0], (800.0, 600.0)), None);
    }
}
