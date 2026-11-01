//! Layers placed in 3D on the GPU (ADR-0037).
//!
//! A layer with a [`LayerSpace`] is drawn as a textured quad whose corners are projected
//! through its camera into homogeneous clip coordinates (`projected.wgsl`), so the picture
//! is mapped with perspective and sampled with anisotropic filtering where it recedes.
//! Its effects and masks have already run in its own space, before projection. The planner
//! has ordered runs of 3D layers by depth, so compositing draws them in plan order like
//! any layer. A layer blurred by the depth of field is drawn alone on a canvas-sized
//! texture, blurred there so its edges soften too, and then composited.

use fosscut_model::Transform;
use wgpu::util::DeviceExt;

use super::{CANVAS_FORMAT, Draw, Quad, Renderer, SAMPLES, Slot};
use crate::geometry::{layer_quad, placed_quad};
use crate::plan::Layer;
use crate::space::{Camera, LayerSpace, apply, layer_model, multiply};

/// Slot numbers of the blur between the two halves of a depth-of-field blur, per nesting
/// depth, start this far up, apart from every layer's own.
const DEFOCUS_BLUR_SLOTS: usize = 1 << 30;

/// One corner of a projected layer quad, as `projected.wgsl` receives it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct ProjectedVertex {
    /// Homogeneous clip coordinates.
    pub(super) position: [f32; 4],
    /// Position within the layer as displayed, 0 to 1 (x right, y down); masks use it.
    pub(super) local: [f32; 2],
    /// Texture coordinate in the stored frame, 0 to 1.
    pub(super) uv: [f32; 2],
}

/// The pipelines and sampler of projected layers.
pub(super) struct Projected {
    composite: wgpu::RenderPipeline,
    accumulate: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl Projected {
    /// Builds them on `layout`, the compositing pipelines' layout, so a layer's bind group
    /// draws with either kind.
    pub(super) fn new(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        common: &str,
    ) -> Projected {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("projected"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{common}\n{}\n{}",
                    include_str!("../shaders/composite.wgsl"),
                    include_str!("../shaders/projected.wgsl")
                )
                .into(),
            ),
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2, 2 => Float32x2];
        let pipeline = |label: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_projected"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<ProjectedVertex>() as u64,
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
        let sum = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Constant,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        Projected {
            composite: pipeline(
                "projected composite",
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            ),
            accumulate: pipeline(
                "projected accumulate",
                wgpu::BlendState {
                    color: sum,
                    alpha: sum,
                },
            ),
            // Pictures that recede are seen at a slant: anisotropic filtering keeps them
            // sharp along the slant instead of blurring them as a whole.
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("projected"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                anisotropy_clamp: 16,
                ..Default::default()
            }),
        }
    }
}

/// The displayed size in canvas pixels of a `source` frame turned `rotation` degrees and
/// fitted inside the `canvas`, as [`layer_quad`] places it.
fn fitted_size(canvas: (u32, u32), source: (u32, u32), rotation: u16) -> (f64, f64) {
    let (mut w, mut h) = (f64::from(source.0.max(1)), f64::from(source.1.max(1)));
    if rotation == 90 || rotation == 270 {
        std::mem::swap(&mut w, &mut h);
    }
    let fit = (f64::from(canvas.0) / w).min(f64::from(canvas.1) / h);
    (w * fit, h * fit)
}

/// The corners of a layer placed by `quad` and `transform` on a `canvas`, showing the
/// `kept` part of its picture, projected through `camera`. The corners, texture and mask
/// coordinates come from the flat placement with only the flips, so cropping, flipping and
/// source rotation mean exactly what they do in 2D.
pub(super) fn projected_quad(
    quad: &Quad,
    canvas: (u32, u32),
    transform: &Transform,
    kept: [f64; 4],
    camera: &Camera,
) -> [ProjectedVertex; 4] {
    let flat = Transform {
        flip_horizontal: transform.flip_horizontal,
        flip_vertical: transform.flip_vertical,
        ..Transform::default()
    };
    let (corners, size, placed) = match quad {
        Quad::Fitted { source, rotation } => (
            layer_quad(canvas, *source, *rotation, &flat, kept),
            fitted_size(canvas, *source, *rotation),
            transform.clone(),
        ),
        Quad::Natural {
            size,
            offset,
            shift,
            scale,
        } => {
            let mut placed = transform.clone();
            placed.position[0] += shift[0];
            placed.position[1] += shift[1];
            placed.scale *= scale;
            (
                placed_quad(canvas, *size, *offset, &flat, kept),
                *size,
                placed,
            )
        }
    };
    let (w, h) = (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1)));
    let to_clip = multiply(
        &camera.clip_from_world(),
        &layer_model(canvas, &placed, size),
    );
    corners.map(|corner| {
        // Back from the flat quad's device coordinates to canvas pixels from the placed
        // point, which the flat placement puts at the canvas centre.
        let point = [
            f64::from(corner.position[0]) * w / 2.0,
            -f64::from(corner.position[1]) * h / 2.0,
            0.0,
        ];
        ProjectedVertex {
            position: apply(&to_clip, point).map(|v| v as f32),
            local: corner.local,
            uv: corner.uv,
        }
    })
}

/// True when `quad` placed by `transform` and seen through `camera` is foreshortened: its
/// corners lie at different depths, so the picture maps with perspective rather than as an
/// affine placement.
pub(super) fn foreshortened(
    quad: &Quad,
    canvas: (u32, u32),
    transform: &Transform,
    kept: [f64; 4],
    camera: &Camera,
) -> bool {
    let corners = projected_quad(quad, canvas, transform, kept, camera);
    let w = corners[0].position[3];
    corners
        .iter()
        .any(|c| (c.position[3] - w).abs() > 1e-6 * w.abs().max(1.0))
}

impl Renderer {
    /// The pipeline that draws `draw`, averaging over a shutter when `accumulate` is set.
    pub(super) fn layer_pipeline(&self, draw: &Draw, accumulate: bool) -> &wgpu::RenderPipeline {
        match (draw.projected, accumulate) {
            (false, false) => &self.composite,
            (false, true) => &self.accumulate,
            (true, false) => &self.projected.composite,
            (true, true) => &self.projected.accumulate,
        }
    }

    /// The sampler `layer`'s picture, placed by `quad` on a `canvas`, is read with:
    /// anisotropic where it is foreshortened, so a layer at rest samples exactly as the
    /// 2D path does.
    pub(super) fn layer_sampler(
        &self,
        layer: &Layer,
        quad: &Quad,
        canvas: (u32, u32),
    ) -> &wgpu::Sampler {
        let slanted = layer.space.as_ref().is_some_and(|space| {
            std::iter::once(&layer.transform)
                .chain(&layer.shutter)
                .any(|t| foreshortened(quad, canvas, t, layer.crop, &space.camera))
        });
        if slanted {
            &self.projected.sampler
        } else {
            &self.linear
        }
    }

    /// A vertex buffer of `quad` placed by `transform` and projected through `space`.
    pub(super) fn projected_buffer(
        &self,
        quad: &Quad,
        canvas: (u32, u32),
        transform: &Transform,
        kept: [f64; 4],
        space: &LayerSpace,
    ) -> wgpu::Buffer {
        let vertices = projected_quad(quad, canvas, transform, kept, &space.camera);
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("projected layer quad"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
    }

    /// A mipmapped canvas-sized texture a defocused layer is drawn on alone, and the view
    /// of its first level to draw into.
    pub(super) fn defocus_target(
        &mut self,
        size: (u32, u32),
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let mips = (size.0.max(size.1) as f32).log2().floor() as u32 + 1;
        let texture = self.texture(
            Slot::Defocus(self.nesting),
            size,
            CANVAS_FORMAT,
            mips,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..Default::default()
        });
        (texture, view)
    }

    /// Blurs the layer drawn on `drawn` by the depth of field's `radius` in output pixels
    /// into `target`. A disc of confusion blurs about as much as a Gaussian with half its
    /// radius as standard deviation.
    pub(super) fn defocus_into(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        drawn: &wgpu::Texture,
        radius: f32,
        target: &wgpu::TextureView,
    ) {
        self.generate_mips(encoder, drawn, drawn.mip_level_count());
        self.blur_into(
            encoder,
            DEFOCUS_BLUR_SLOTS + self.nesting,
            drawn,
            radius / 2.0,
            target,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::UNCROPPED;

    #[test]
    fn at_rest_on_the_canvas_plane_projection_matches_the_flat_quad() {
        let canvas = (1920, 1080);
        let camera = Camera::default_for(canvas);
        let quads = [
            Quad::Fitted {
                source: (1080, 1920),
                rotation: 90,
            },
            Quad::Natural {
                size: (480.0, 108.0),
                offset: (96.0, -12.0),
                shift: [0.0, 0.3],
                scale: 1.2,
            },
        ];
        let transform = Transform {
            position: [0.2, -0.1],
            scale: 0.7,
            stretch: [1.2, 0.9],
            rotation: 17.0,
            flip_horizontal: true,
            ..Transform::default()
        };
        for quad in &quads {
            let flat = quad.vertices(canvas, &transform, [0.1, 0.0, 0.9, 0.8]);
            let projected = projected_quad(quad, canvas, &transform, [0.1, 0.0, 0.9, 0.8], &camera);
            for (a, b) in flat.iter().zip(&projected) {
                assert!((b.position[3] - 1.0).abs() < 1e-6, "w is 1 at z = 0");
                for axis in 0..2 {
                    assert!(
                        (a.position[axis] - b.position[axis]).abs() < 1e-5,
                        "{a:?} {b:?}"
                    );
                }
                assert_eq!((a.local, a.uv), (b.local, b.uv));
            }
        }
        // Turned around its vertical axis, the right edge recedes: smaller w on the left.
        let turned = Transform {
            three_d: true,
            rotation_y: 40.0,
            ..Transform::default()
        };
        let quad = Quad::Fitted {
            source: (1920, 1080),
            rotation: 0,
        };
        let corners = projected_quad(&quad, canvas, &turned, UNCROPPED, &camera);
        assert!(corners[1].position[3] > 1.0 && corners[0].position[3] < 1.0);
        assert!(foreshortened(&quad, canvas, &turned, UNCROPPED, &camera));
        assert!(!foreshortened(
            &quads[1], canvas, &transform, UNCROPPED, &camera
        ));
        let deep = Transform {
            three_d: true,
            position_z: 0.7,
            rotation: 30.0,
            ..Transform::default()
        };
        assert!(
            !foreshortened(&quad, canvas, &deep, UNCROPPED, &camera),
            "parallel to the view, only smaller"
        );
        let height = |c: &ProjectedVertex, d: &ProjectedVertex| {
            (c.position[1] / c.position[3] - d.position[1] / d.position[3]).abs()
        };
        assert!(
            height(&corners[1], &corners[3]) < height(&corners[0], &corners[2]),
            "the far edge looks shorter"
        );
    }
}
