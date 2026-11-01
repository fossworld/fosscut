//! Light effects on the GPU ([`crate::light`]): a layer's effects with its picture padded
//! so light spreads past its edges, and the passes of `light.glow` and `light.streaks`.

use crate::FrameProvider;
use crate::RenderError;
use crate::effects::PlannedEffect;
use crate::light::{
    Light, Padding, after_padding, annotate, glow_sigmas, pack_add, pack_bright, pack_pad,
    pack_streaks, padding,
};
use crate::plan::{Layer, SourceRef};
use crate::space::LayerSpace;
use fosscut_model::Transform;
use wgpu::util::DeviceExt;

use super::three_d::projected_quad;
use super::{CANVAS_FORMAT, Quad, Renderer, Slot, flow};

/// Added to a layer's slot number for the textures of its padded picture, so they never
/// share a slot, and with it a size, with the picture before padding.
const PADDED: usize = 1 << 28;
/// Added to a layer's slot number for the second blur of a glow (its core), whose middle
/// texture has another size than the halo's.
const CORE: usize = 1 << 27;

fn usage() -> wgpu::TextureUsages {
    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
}

fn mip_count(size: (u32, u32)) -> u32 {
    (size.0.max(size.1).max(1) as f32).log2().floor() as u32 + 1
}

fn first_level(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: 0,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

impl Renderer {
    /// The vertex buffer of `layer` placed by `transform` (and projected through `space`
    /// when it is placed in 3D), with its picture padded: the quad grows past every edge
    /// that is not cropped, and its texture coordinates move into the padded picture, so
    /// the picture keeps its place and size and its masks stay where they were.
    pub(super) fn padded_buffer(
        &self,
        quad: &Quad,
        canvas: (u32, u32),
        transform: &Transform,
        layer: &Layer,
        space: Option<&LayerSpace>,
        padding: &Padding,
    ) -> wgpu::Buffer {
        let turned = layer.rotation == 90 || layer.rotation == 270;
        let kept = padding.kept(layer.crop, turned);
        let contents: Vec<u8> = match space {
            Some(space) => {
                let mut corners = projected_quad(quad, canvas, transform, kept, &space.camera);
                for corner in &mut corners {
                    corner.uv = padding.uv(corner.uv);
                }
                bytemuck::cast_slice(&corners).to_vec()
            }
            None => {
                let corners = padding.remap(quad.vertices(canvas, transform, kept));
                bytemuck::cast_slice(&corners).to_vec()
            }
        };
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("padded layer quad"),
                contents: &contents,
                usage: wgpu::BufferUsages::VERTEX,
            })
    }

    /// Applies a layer's `effects` to its picture `texture` and returns the result with the
    /// padding its light needed, if any. Light effects measure with `per_canvas_pixel` of
    /// the picture's pixels per canvas pixel of `canvas`; when one spreads past the
    /// picture, the effects from [`Padding::at`] on run on a padded copy.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn layer_effects(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        texture: &wgpu::Texture,
        effects: &[PlannedEffect],
        canvas: (u32, u32),
        per_canvas_pixel: f64,
        frames: &mut dyn FrameProvider,
        source: Option<&SourceRef>,
        temporal: Option<&flow::TemporalInputs>,
    ) -> Result<(wgpu::TextureView, Option<Padding>), RenderError> {
        let size = (texture.width(), texture.height());
        let annotated = annotate(effects, canvas, per_canvas_pixel, size);
        let effects = annotated.as_deref().unwrap_or(effects);
        let limit = self.device.limits().max_texture_dimension_2d;
        let Some(padding) = padding(effects, size, limit) else {
            let view =
                self.apply_effects(encoder, index, texture, effects, frames, source, temporal)?;
            return Ok((view, None));
        };
        let before = if padding.at == 0 {
            texture.create_view(&Default::default())
        } else {
            self.apply_effects(
                encoder,
                index,
                texture,
                &effects[..padding.at],
                frames,
                source,
                temporal,
            )?
        };
        let padded = self.pad_picture(encoder, index + PADDED, &before, &padding);
        let after = after_padding(effects, &padding);
        // Nothing after the padding reads cached analyses or neighbouring frames.
        let view =
            self.apply_effects(encoder, index + PADDED, &padded, &after, frames, None, None)?;
        Ok((view, Some(padding)))
    }

    /// The picture `input` in the middle of a transparent one `padding` larger, mipmapped.
    fn pad_picture(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        layer: usize,
        input: &wgpu::TextureView,
        padding: &Padding,
    ) -> wgpu::Texture {
        let size = padding.padded();
        let mips = mip_count(size);
        let texture = self.texture(
            Slot::Light { layer, kind: 0 },
            size,
            CANVAS_FORMAT,
            mips,
            usage(),
        );
        self.effect_pass(
            encoder,
            "fs_light_pad",
            input,
            None,
            None,
            None,
            &pack_pad(padding),
            &first_level(&texture),
        );
        self.generate_mips(encoder, &texture, mips);
        texture
    }

    /// Runs `light.glow` or `light.streaks` on `current` into `target`: the bright pass,
    /// then the blurs or the streak passes, then the light added to the picture.
    pub(super) fn light_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        layer: usize,
        current: &wgpu::Texture,
        effect: &PlannedEffect,
        kind: Light,
        target: &wgpu::TextureView,
    ) {
        let size = (current.width(), current.height());
        let mips = mip_count(size);
        let input = current.create_view(&Default::default());
        let bright = self.texture(
            Slot::Light { layer, kind: 1 },
            size,
            CANVAS_FORMAT,
            mips,
            usage(),
        );
        self.effect_pass(
            encoder,
            "fs_light_bright",
            &input,
            None,
            None,
            None,
            &pack_bright(effect),
            &first_level(&bright),
        );
        self.generate_mips(encoder, &bright, mips);
        match kind {
            Light::Glow => {
                let (wide, core) = glow_sigmas(effect, size);
                let halo = self.canvas(Slot::Light { layer, kind: 2 }, size);
                self.blur_into(encoder, layer, &bright, wide, &halo);
                let inner = self.canvas(Slot::Light { layer, kind: 3 }, size);
                self.blur_into(encoder, layer + CORE, &bright, core, &inner);
                self.effect_pass(
                    encoder,
                    "fs_light_add",
                    &input,
                    Some(&halo),
                    None,
                    Some(&inner),
                    &pack_add(effect, [0.5, 0.5]),
                    target,
                );
            }
            Light::Streaks => {
                let [fine, coarse] = pack_streaks(effect, size, mips);
                let bright_view = bright.create_view(&Default::default());
                let first = self.canvas(Slot::Light { layer, kind: 4 }, size);
                self.effect_pass(
                    encoder,
                    "fs_light_streak",
                    &bright_view,
                    None,
                    None,
                    None,
                    &fine,
                    &first,
                );
                let streaks = self.canvas(Slot::Light { layer, kind: 5 }, size);
                self.effect_pass(
                    encoder,
                    "fs_light_streak",
                    &first,
                    None,
                    None,
                    None,
                    &coarse,
                    &streaks,
                );
                self.effect_pass(
                    encoder,
                    "fs_light_add",
                    &input,
                    Some(&streaks),
                    None,
                    Some(&streaks),
                    &pack_add(effect, [1.0, 0.0]),
                    target,
                );
            }
        }
    }
}
