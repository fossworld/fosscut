//! Draws particle layers (ADR-0038): the particles of a moment, computed on the CPU by
//! [`crate::particles`], become one instanced quad each in `particles.wgsl`, drawn into a
//! transparent texture of the output size that the layer then composites like any picture.

use fosscut_model::{ParticleBlend, SpriteKind};
use wgpu::util::DeviceExt;

use super::{CANVAS_FORMAT, Renderer, Slot, attachment, entry};
use crate::effects::PlannedEffect;
use crate::particles::{ParticleLayer, particles_during};
use crate::{FrameProvider, RenderError};

/// The smallest radius a particle is drawn with, in output pixels; smaller particles keep
/// their light by losing opacity instead, so they never flicker.
const MIN_RADIUS: f32 = 0.75;

/// One particle as the shader reads it.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    center: [f32; 2],
    half_size: [f32; 2],
    axis: [f32; 2],
    extent: [f32; 2],
    color: [f32; 4],
    shape: [f32; 4],
}

/// The particle pipelines: light adding up, and covering.
pub(super) struct ParticlePipelines {
    add: wgpu::RenderPipeline,
    normal: wgpu::RenderPipeline,
}

impl ParticlePipelines {
    pub(super) fn new(device: &wgpu::Device, common: &str) -> ParticlePipelines {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{common}\n{}", include_str!("../shaders/particles.wgsl")).into(),
            ),
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x2,
            1 => Float32x2,
            2 => Float32x2,
            3 => Float32x2,
            4 => Float32x4,
            5 => Float32x4,
        ];
        let pipeline = |label: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Instance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: CANVAS_FORMAT,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // Light adds up; coverage combines as with normal layers, so it never passes 1.
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        ParticlePipelines {
            add: pipeline("particles add", add),
            normal: pipeline(
                "particles normal",
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            ),
        }
    }
}

/// The shader's instances for `layer` on a `canvas` drawn at `scale` output pixels per
/// canvas pixel.
fn instances(
    layer: &ParticleLayer,
    canvas: (u32, u32),
    scale: f32,
    shutter: Option<(f64, usize)>,
) -> Vec<Instance> {
    let sprite = &layer.content.sprite;
    let kind = match sprite.kind {
        SpriteKind::Dot => 0.0,
        SpriteKind::Spark => 1.0,
        SpriteKind::Star => 2.0,
        SpriteKind::Ring => 3.0,
        SpriteKind::Square => 4.0,
        SpriteKind::Image => 5.0,
    };
    // An image sprite without its image draws as a dot.
    let kind = if kind == 5.0 && layer.sprite.is_none() {
        0.0
    } else {
        kind
    };
    particles_during(layer, canvas, shutter)
        .into_iter()
        .filter_map(|p| {
            let mut radius = p.size * scale / 2.0;
            let mut alpha = p.color[3];
            if radius < MIN_RADIUS {
                alpha *= (radius / MIN_RADIUS).powi(2);
                radius = MIN_RADIUS;
            }
            if alpha <= 0.0 {
                return None;
            }
            let half_length = p.length * scale / 2.0;
            let flip = p.flip;
            let half_size = match sprite.kind {
                SpriteKind::Spark => [half_length + radius, radius],
                SpriteKind::Square => [
                    (radius * sprite.aspect as f32 * flip.abs()).max(MIN_RADIUS / 2.0),
                    radius,
                ],
                _ if flip != 1.0 => [(radius * flip.abs()).max(MIN_RADIUS / 2.0), radius],
                _ => [radius, radius],
            };
            let (sin, cos) = p.angle.sin_cos();
            Some(Instance {
                center: [p.center[0] * scale, p.center[1] * scale],
                half_size,
                axis: [cos, sin],
                extent: [radius, half_length],
                color: [p.color[0], p.color[1], p.color[2], alpha],
                shape: [kind, sprite.softness as f32, sprite.points as f32, flip],
            })
        })
        .collect()
}

impl Renderer {
    /// Draws the particles of `layer` into a mipmapped texture of the output `size` for the
    /// layer in slot `index`; `None` when no particle shows. A motion blur among `effects`
    /// opens a shutter over the particles' own motion too.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn particle_layer(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        size: (u32, u32),
        canvas: (u32, u32),
        layer: &ParticleLayer,
        effects: &[PlannedEffect],
        frames: &mut dyn FrameProvider,
    ) -> Result<Option<wgpu::Texture>, RenderError> {
        let scale = size.1 as f32 / canvas.1.max(1) as f32;
        let shutter = crate::effects::motion_blur(effects);
        let instances = instances(layer, canvas, scale, shutter);
        if instances.is_empty() {
            return Ok(None);
        }
        let sprite = match &layer.sprite {
            Some(source) => {
                let frame = frames.frame(source)?;
                Some(self.convert_layer(encoder, index, &frame, source)?)
            }
            None => None,
        };
        let sprite_view = match &sprite {
            Some(texture) => texture.create_view(&Default::default()),
            None => self.blank.create_view(&Default::default()),
        };
        let mips = (size.0.max(size.1) as f32).log2().floor() as u32 + 1;
        let texture = self.texture(
            Slot::Text(index),
            size,
            CANVAS_FORMAT,
            mips,
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
        );
        let pipeline = match layer.content.blend {
            ParticleBlend::Add => &self.particles.add,
            ParticleBlend::Normal => &self.particles.normal,
        };
        let viewport = self.uniform(&[size.0 as f32, size.1 as f32, 0.0, 0.0]);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particles"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, viewport.as_entire_binding()),
                entry(1, wgpu::BindingResource::TextureView(&sprite_view)),
                entry(2, wgpu::BindingResource::Sampler(&self.linear)),
            ],
        });
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("particle instances"),
                contents: bytemuck::cast_slice(&instances),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let target = texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..Default::default()
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("particles"),
                color_attachments: &[Some(attachment(&target, None))],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..4, 0..instances.len() as u32);
        }
        self.generate_mips(encoder, &texture, mips);
        Ok(Some(texture))
    }
}
