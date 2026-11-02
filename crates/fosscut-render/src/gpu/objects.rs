//! Draws 3D object layers (ADR-0040): the object's mesh through its camera
//! ([`Camera::clip_from_world`]) with a depth buffer of its own and 4x multisampling, lit by
//! a sky light and a key light whose shadow comes from a depth map drawn first, then the
//! floor that catches it, into a transparent canvas-sized texture in premultiplied linear
//! light that the layer composites like any picture (`objects.wgsl`).
//!
//! Meshes, and the images of model files, are kept on the GPU between frames by what they
//! are made from; everything else is made per frame.

use std::collections::HashMap;
use std::sync::Arc;

use fosscut_model::{ModelObject, Transform};
use wgpu::util::DeviceExt;

use super::{CANVAS_FORMAT, Renderer, SAMPLES, Slot, entry};
use crate::objects::{
    MAX_TEXTURES, MeshPart, MeshVertex, ModelData, ModelImage, ObjectLayer, Texture, floor_level,
    light_view, object_model, placed_corners, shadings, solid_mesh, text_mesh,
};
use crate::space::{Camera, LayerSpace, Mat4, Vec3, multiply};
use crate::{FrameProvider, RenderError};

/// Slot numbers of the images objects print start this far up, apart from every layer's
/// own and from the depth of field's.
const TEXTURE_SLOTS: usize = 1 << 28;
/// Meshes kept on the GPU; beyond it the cache starts again.
const MESH_CACHE_LIMIT: usize = 32;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The sharpest and softest shadow maps' sides in texels.
const SHADOW_SIZES: (f64, f64) = (64.0, 2048.0);

/// What `objects.wgsl` reads about the scene of one object.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SceneUniform {
    clip_from_object: [[f32; 4]; 4],
    world_from_object: [[f32; 4]; 4],
    normal_from_object: [[f32; 4]; 4],
    light_from_world: [[f32; 4]; 4],
    clip_from_world: [[f32; 4]; 4],
    eye: [f32; 4],
    light_dir: [f32; 4],
    light_color: [f32; 4],
    sky: [f32; 4],
    ground: [f32; 4],
    shadow: [f32; 4],
    floor: [f32; 4],
}

/// What `objects.wgsl` reads about one material.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfaceUniform {
    color: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
    rect: [f32; 4],
}

/// A mesh on the GPU.
struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    parts: Vec<MeshPart>,
    /// Its bounds in object units.
    bounds: (Vec3, Vec3),
}

/// The pipelines, samplers and caches of 3D objects.
pub(super) struct ObjectPipelines {
    scene_layout: wgpu::BindGroupLayout,
    shadow_scene_layout: wgpu::BindGroupLayout,
    surface_layout: wgpu::BindGroupLayout,
    solid: wgpu::RenderPipeline,
    blended: wgpu::RenderPipeline,
    floor: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    compare: wgpu::Sampler,
    printed: wgpu::Sampler,
    wrapped: wgpu::Sampler,
    white: wgpu::Texture,
    meshes: HashMap<String, Arc<GpuMesh>>,
    images: HashMap<(u64, usize), wgpu::Texture>,
    targets: HashMap<(u32, u32), (wgpu::Texture, wgpu::Texture)>,
    shadow_maps: HashMap<u32, wgpu::Texture>,
}

/// A row-major matrix as WGSL's column-major `mat4x4f`.
fn columns(m: &Mat4) -> [[f32; 4]; 4] {
    std::array::from_fn(|c| std::array::from_fn(|r| m[r][c] as f32))
}

/// The matrix that turns object normals into scene normals: the cofactors of `m`'s upper
/// 3x3 (its inverse transpose up to scale), so stretched objects keep perpendicular normals.
fn normal_matrix(m: &Mat4) -> Mat4 {
    let c =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    [
        [c(1, 2, 1, 2), -c(1, 2, 0, 2), c(1, 2, 0, 1), 0.0],
        [-c(0, 2, 1, 2), c(0, 2, 0, 2), -c(0, 2, 0, 1), 0.0],
        [c(0, 1, 1, 2), -c(0, 1, 0, 2), c(0, 1, 0, 1), 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn linear(hex: &str, scale: f64) -> [f32; 4] {
    let [r, g, b, _] = crate::text::linear_color(hex);
    let s = scale as f32;
    [r * s, g * s, b * s, 1.0]
}

impl ObjectPipelines {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> ObjectPipelines {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("objects"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/objects.wgsl").into()),
        });
        let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object scene"),
            entries: &[
                uniform(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let shadow_scene_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("object shadow scene"),
                entries: &[uniform(0)],
            });
        let surface_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object surface"),
            entries: &[
                uniform(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("objects"),
            bind_group_layouts: &[Some(&scene_layout), Some(&surface_layout)],
            immediate_size: 0,
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("object shadows"),
            bind_group_layouts: &[Some(&shadow_scene_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let pipeline = |label: &str,
                        vertex: &str,
                        fragment: &str,
                        blend: Option<wgpu::BlendState>,
                        write_depth: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&main_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                // Both sides are drawn: the depth test hides the inside of closed meshes,
                // and open ones (planes, files) show their backs.
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(write_depth),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
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
        let over = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let solid = pipeline("object solid", "vs_object", "fs_object", None, true);
        let blended = pipeline("object blended", "vs_object", "fs_object", over, false);
        let floor = pipeline("object floor", "vs_floor", "fs_floor", over, false);
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("object shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let sampler = |label: &str, address: wgpu::AddressMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: address,
                address_mode_v: address,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        let compare = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("object shadow compare"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let white = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("object white"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
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
            &[255, 255, 255, 255],
        );
        ObjectPipelines {
            scene_layout,
            shadow_scene_layout,
            surface_layout,
            solid,
            blended,
            floor,
            shadow,
            compare,
            printed: sampler("object printed", wgpu::AddressMode::ClampToEdge),
            wrapped: sampler("object wrapped", wgpu::AddressMode::Repeat),
            white,
            meshes: HashMap::new(),
            images: HashMap::new(),
            targets: HashMap::new(),
            shadow_maps: HashMap::new(),
        }
    }
}

/// An image of a model file as an sRGB texture with mipmaps made on the CPU (a box filter
/// in linear light).
fn image_texture(device: &wgpu::Device, queue: &wgpu::Queue, image: &ModelImage) -> wgpu::Texture {
    let (mut w, mut h) = (image.width.max(1), image.height.max(1));
    let mips = (w.max(h) as f32).log2().floor() as u32 + 1;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("object image"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let decode = |v: u8| {
        let c = f32::from(v) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let encode = |l: f32| {
        let c = if l <= 0.003_130_8 {
            12.92 * l
        } else {
            1.055 * l.powf(1.0 / 2.4) - 0.055
        };
        (c.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    let mut level: Vec<u8> = image.rgba.clone();
    level.resize((w * h * 4) as usize, 255);
    for mip in 0..mips {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &level,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0.0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((2 * x + dx).min(w - 1), (2 * y + dy).min(h - 1));
                        let v = level[((sy * w + sx) * 4 + c) as usize];
                        sum += if c == 3 {
                            f32::from(v) / 255.0
                        } else {
                            decode(v)
                        };
                    }
                    let mean = sum / 4.0;
                    next[((y * nw + x) * 4 + c) as usize] = if c == 3 {
                        (mean * 255.0).round() as u8
                    } else {
                        encode(mean)
                    };
                }
            }
        }
        level = next;
        (w, h) = (nw, nh);
    }
    texture
}

impl Renderer {
    /// Draws the 3D object `layer` placed by `transform` and seen through `space`'s camera
    /// (the default camera without one) into a mipmapped transparent texture of the output
    /// `size` for the layer in slot `index`; `None` when it has no triangles.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn object_layer(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        size: (u32, u32),
        canvas: (u32, u32),
        layer: &ObjectLayer,
        transform: &Transform,
        space: Option<&LayerSpace>,
        frames: &mut dyn FrameProvider,
    ) -> Result<Option<wgpu::Texture>, RenderError> {
        let content = &layer.content;
        let camera = space.map_or_else(|| Camera::default_for(canvas), |s| s.camera.clone());
        // The model file of an asset object.
        let model = match content.model_asset() {
            Some(asset) => Some(
                frames
                    .model(asset)
                    .ok_or_else(|| RenderError::MissingFrame {
                        asset: asset.to_string(),
                        message: "the 3D model is not loaded".to_string(),
                    })?,
            ),
            None => None,
        };
        let mesh = self.object_mesh(content, model.as_deref(), frames)?;
        if mesh.parts.is_empty() {
            return Ok(None);
        }
        let fit = match (&content.object, &model) {
            (ModelObject::Asset(object), Some(model)) => model.fit(object.size),
            _ => ([0.0; 3], 1.0),
        };
        let bounds = {
            let (low, high) = mesh.bounds;
            let at = |p: Vec3| -> Vec3 { std::array::from_fn(|a| (p[a] + fit.0[a]) * fit.1) };
            (at(low), at(high))
        };
        let extent = [bounds.1[0] - bounds.0[0], bounds.1[1] - bounds.0[1]];
        let model_matrix = object_model(canvas, transform, extent, fit);
        let clip_from_world = camera.clip_from_world();

        // The key light's view over the object's bounding sphere.
        let corners = placed_corners(&model_matrix, mesh.bounds);
        let centre: Vec3 = std::array::from_fn(|a| {
            corners.iter().map(|c| c[a]).sum::<f64>() / corners.len() as f64
        });
        let radius = corners
            .iter()
            .map(|c| {
                ((c[0] - centre[0]).powi(2)
                    + (c[1] - centre[1]).powi(2)
                    + (c[2] - centre[2]).powi(2))
                .sqrt()
            })
            .fold(1e-3, f64::max);
        let light = &content.light;
        let direction = light.direction();
        let light_matrix = light_view(direction, centre, radius);
        // A softer shadow is filtered over a coarser map: 9 by 9 taps of a texel each span
        // the penumbra.
        let penumbra = (0.004 + 0.12 * light.softness.clamp(0.0, 1.0)) * 2.0 * radius;
        let texel_world = penumbra / 9.0;
        let shadow_size = (2.0 * radius / texel_world)
            .clamp(SHADOW_SIZES.0, SHADOW_SIZES.1)
            .round() as u32;
        let texel_world = 2.0 * radius / f64::from(shadow_size);
        let depth_range = 10.0 * radius;

        let shadings = shadings(layer, model.as_deref());
        let floor = content.floor.as_ref();
        let scene = SceneUniform {
            clip_from_object: columns(&multiply(&clip_from_world, &model_matrix)),
            world_from_object: columns(&model_matrix),
            normal_from_object: columns(&normal_matrix(&model_matrix)),
            light_from_world: columns(&light_matrix),
            clip_from_world: columns(&clip_from_world),
            eye: [
                camera.eye[0] as f32,
                camera.eye[1] as f32,
                camera.eye[2] as f32,
                1.0,
            ],
            light_dir: [
                direction[0] as f32,
                direction[1] as f32,
                direction[2] as f32,
                if light.shadows { 1.0 } else { 0.0 },
            ],
            light_color: linear(light.color.as_str(), light.intensity),
            sky: linear(light.sky.as_str(), light.ambient),
            ground: linear(light.ground.as_str(), light.ambient),
            shadow: [
                1.0 / shadow_size as f32,
                (1.5 * texel_world / depth_range) as f32,
                (1.5 * texel_world) as f32,
                floor.map_or(0.0, |f| f.shadow.clamp(0.0, 1.0) as f32),
            ],
            floor: match floor {
                Some(f) => {
                    let [r, g, b, _] = linear(f.color.as_str(), 1.0);
                    [r, g, b, if f.catcher { 1.0 } else { 0.0 }]
                }
                None => [0.0; 4],
            },
        };
        let scene_buffer = self.uniform(&scene);

        // The printed images of built-in materials.
        let mut printed: Vec<(wgpu::Texture, (u32, u32))> = Vec::new();
        for (k, source) in layer.textures.iter().enumerate().take(MAX_TEXTURES) {
            let frame = frames.frame(source)?;
            let texture = self.convert_layer(
                encoder,
                TEXTURE_SLOTS + index * MAX_TEXTURES + k,
                &frame,
                source,
            )?;
            let shown = (texture.width(), texture.height());
            printed.push((texture, shown));
        }
        let key = model.as_deref().map(|m| m.key);
        let mut surfaces = Vec::with_capacity(mesh.parts.len());
        for part in &mesh.parts {
            let Some(shading) = shadings.get(part.slot) else {
                continue;
            };
            let (view, sampler, mode, scale) = match shading.texture {
                Some(Texture::Asset(i)) if i < printed.len() => {
                    let (texture, (tw, th)) = &printed[i];
                    // Fit the picture into its share of a flat face, keeping its shape.
                    let size = f64::from(shading.texture_size);
                    let [fw, fh] = part.face.map(f64::from);
                    let scale = if fw > 0.0 && fh > 0.0 {
                        let (face, picture) = (fw / fh, f64::from(*tw) / f64::from((*th).max(1)));
                        if picture > face {
                            [size, size * face / picture]
                        } else {
                            [size * picture / face, size]
                        }
                    } else {
                        [1.0, 1.0]
                    };
                    (
                        texture.create_view(&Default::default()),
                        &self.objects.printed,
                        1.0,
                        scale,
                    )
                }
                Some(Texture::File(i)) => {
                    match (key, model.as_deref().and_then(|m| m.images.get(i))) {
                        (Some(key), Some(image)) => {
                            let device = &self.device;
                            let queue = &self.queue;
                            let texture = self
                                .objects
                                .images
                                .entry((key, i))
                                .or_insert_with(|| image_texture(device, queue, image))
                                .clone();
                            (
                                texture.create_view(&Default::default()),
                                &self.objects.wrapped,
                                2.0,
                                [1.0, 1.0],
                            )
                        }
                        _ => (
                            self.objects.white.create_view(&Default::default()),
                            &self.objects.wrapped,
                            0.0,
                            [1.0, 1.0],
                        ),
                    }
                }
                _ => (
                    self.objects.white.create_view(&Default::default()),
                    &self.objects.wrapped,
                    0.0,
                    [1.0, 1.0],
                ),
            };
            let surface = SurfaceUniform {
                color: shading.color,
                emissive: [
                    shading.emissive[0],
                    shading.emissive[1],
                    shading.emissive[2],
                    0.0,
                ],
                params: [
                    shading.metallic,
                    shading.roughness,
                    mode,
                    if shading.double_sided { 1.0 } else { 0.0 },
                ],
                rect: [
                    scale[0].max(1e-3) as f32,
                    scale[1].max(1e-3) as f32,
                    shading.cutoff.unwrap_or(-1.0),
                    if shading.blend { 1.0 } else { 0.0 },
                ],
            };
            let buffer = self.uniform(&surface);
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("object surface"),
                layout: &self.objects.surface_layout,
                entries: &[
                    entry(0, buffer.as_entire_binding()),
                    entry(1, wgpu::BindingResource::TextureView(&view)),
                    entry(2, wgpu::BindingResource::Sampler(sampler)),
                ],
            });
            surfaces.push((part.clone(), shading.clone(), bind_group));
        }

        // The key light's depth map of what casts shadows.
        let shadow_map = self.shadow_map(shadow_size);
        let shadow_view = shadow_map.create_view(&Default::default());
        {
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("object shadow scene"),
                layout: &self.objects.shadow_scene_layout,
                entries: &[entry(0, scene_buffer.as_entire_binding())],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("object shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if light.shadows {
                pass.set_pipeline(&self.objects.shadow);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                for (part, shading, _) in &surfaces {
                    if !shading.blend {
                        pass.draw_indexed(part.start..part.start + part.count, 0, 0..1);
                    }
                }
            }
        }

        // The object, its floor and its see-through parts.
        let floor_buffer = floor.map(|f| {
            let level = floor_level(canvas, transform, bounds, f.gap);
            let w = f64::from(canvas.0.max(1));
            let half = f.size * w / 2.0;
            let (x, z) = (model_matrix[0][3], model_matrix[2][3]);
            let corner = |dx: f64, dz: f64, u: f32, v: f32| MeshVertex {
                position: [(x + dx) as f32, level as f32, (z + dz) as f32],
                normal: [0.0, -1.0, 0.0],
                uv: [u, v],
            };
            let quad = [
                corner(-half, -half, 0.0, 0.0),
                corner(half, -half, 1.0, 0.0),
                corner(half, half, 1.0, 1.0),
                corner(-half, -half, 0.0, 0.0),
                corner(half, half, 1.0, 1.0),
                corner(-half, half, 0.0, 1.0),
            ];
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("object floor"),
                    contents: bytemuck::cast_slice(&quad),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });
        let scene_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("object scene"),
            layout: &self.objects.scene_layout,
            entries: &[
                entry(0, scene_buffer.as_entire_binding()),
                entry(1, wgpu::BindingResource::TextureView(&shadow_view)),
                entry(2, wgpu::BindingResource::Sampler(&self.objects.compare)),
            ],
        });
        let floor_surface = (floor_buffer.is_some()).then(|| {
            let buffer = self.uniform(&SurfaceUniform {
                color: [0.0; 4],
                emissive: [0.0; 4],
                params: [0.0; 4],
                rect: [1.0, 1.0, -1.0, 1.0],
            });
            let white = self.objects.white.create_view(&Default::default());
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("object floor surface"),
                layout: &self.objects.surface_layout,
                entries: &[
                    entry(0, buffer.as_entire_binding()),
                    entry(1, wgpu::BindingResource::TextureView(&white)),
                    entry(2, wgpu::BindingResource::Sampler(&self.objects.wrapped)),
                ],
            })
        });
        let (samples, depth) = self.object_targets(size);
        let samples_view = samples.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
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
        let target = texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..Default::default()
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("object"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &samples_view,
                    depth_slice: None,
                    resolve_target: Some(&target),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &scene_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_pipeline(&self.objects.solid);
            for (part, shading, group) in &surfaces {
                if !shading.blend {
                    pass.set_bind_group(1, group, &[]);
                    pass.draw_indexed(part.start..part.start + part.count, 0, 0..1);
                }
            }
            if let (Some(buffer), Some(group)) = (&floor_buffer, &floor_surface) {
                pass.set_pipeline(&self.objects.floor);
                pass.set_bind_group(1, group, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..6, 0..1);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            }
            pass.set_pipeline(&self.objects.blended);
            for (part, shading, group) in &surfaces {
                if shading.blend {
                    pass.set_bind_group(1, group, &[]);
                    pass.draw_indexed(part.start..part.start + part.count, 0, 0..1);
                }
            }
        }
        self.generate_mips(encoder, &texture, mips);
        Ok(Some(texture))
    }

    /// The mesh of `content` on the GPU, made once per shape, text, font set or model.
    fn object_mesh(
        &mut self,
        content: &fosscut_model::ModelContent,
        model: Option<&ModelData>,
        frames: &mut dyn FrameProvider,
    ) -> Result<Arc<GpuMesh>, RenderError> {
        let key = match (&content.object, model) {
            (ModelObject::Asset(_), Some(model)) => format!("model:{:016x}", model.key),
            (ModelObject::Text(text), _) => {
                // Project fonts the text names are loaded first; the key counts them, so
                // text drawn before a font arrived is made again.
                let family = text.font.clone().unwrap_or_default();
                let mut wanted = fosscut_model::TextContent::plain(text.text.clone());
                wanted.style.font = text.font.clone();
                self.load_fonts(&wanted, frames)?;
                format!("text:{:?}:{}:{}", text, family, self.fonts_loaded.len())
            }
            (object, _) => format!("{object:?}"),
        };
        if let Some(mesh) = self.objects.meshes.get(&key) {
            return Ok(mesh.clone());
        }
        let mesh = match (&content.object, model) {
            (ModelObject::Asset(_), Some(model)) => model.mesh.clone(),
            (ModelObject::Text(text), _) => text_mesh(&mut self.text, text),
            (object, _) => solid_mesh(object).unwrap_or_default(),
        };
        let bounds = mesh.bounds().unwrap_or(([0.0; 3], [0.0; 3]));
        // An empty mesh still needs buffers to bind.
        let vertices: &[MeshVertex] = if mesh.vertices.is_empty() {
            &[MeshVertex {
                position: [0.0; 3],
                normal: [0.0, 0.0, -1.0],
                uv: [0.0; 2],
            }]
        } else {
            &mesh.vertices
        };
        let indices: &[u32] = if mesh.indices.is_empty() {
            &[0, 0, 0]
        } else {
            &mesh.indices
        };
        let gpu = Arc::new(GpuMesh {
            vertices: self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("object vertices"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            indices: self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("object indices"),
                    contents: bytemuck::cast_slice(indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
            parts: mesh.parts,
            bounds,
        });
        if self.objects.meshes.len() >= MESH_CACHE_LIMIT {
            self.objects.meshes.clear();
            self.objects.images.clear();
        }
        self.objects.meshes.insert(key, gpu.clone());
        Ok(gpu)
    }

    /// The multisampled colour and depth targets objects draw into, by output size.
    fn object_targets(&mut self, size: (u32, u32)) -> (wgpu::Texture, wgpu::Texture) {
        let device = &self.device;
        self.objects.targets.retain(|key, _| *key == size);
        self.objects
            .targets
            .entry(size)
            .or_insert_with(|| {
                let make = |label: &str, format: wgpu::TextureFormat| {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: size.0,
                            height: size.1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: SAMPLES,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                };
                (
                    make("object samples", CANVAS_FORMAT),
                    make("object depth", DEPTH_FORMAT),
                )
            })
            .clone()
    }

    /// The key light's depth map of `side` texels.
    fn shadow_map(&mut self, side: u32) -> wgpu::Texture {
        let device = &self.device;
        if self.objects.shadow_maps.len() > 4 {
            self.objects.shadow_maps.clear();
        }
        self.objects
            .shadow_maps
            .entry(side)
            .or_insert_with(|| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("object shadow map"),
                    size: wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: DEPTH_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            })
            .clone()
    }
}
