//! Render plans: what to draw for one frame (ADR-0008, item 2). Planning is a pure
//! function of the project and a time; the GPU only executes plans.

use std::sync::Arc;

use fosscut_model::{
    BlendMode, Clip, ClipContent, Id, Interpolate, Project, TextContent, Track, TrackKind,
    Transform, animate,
};
use fosscut_time::Flicks;

use crate::RenderError;
use crate::caption::CaptionLayer;
use crate::effects::PlannedEffect;
use crate::geometry::{UNCROPPED, clockwise_rotation};
use crate::lut::Lut3d;
use crate::text::TextLayer;

/// Which frame of which asset a layer shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRef {
    /// The asset (`asset_…`).
    pub asset: Id,
    /// Time within the asset; zero for still images.
    pub time: Flicks,
}

/// What a layer shows.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerSource {
    /// A frame of a video or image asset.
    Frame(SourceRef),
    /// Styled text at one moment, composed at the output resolution.
    Text(Box<TextLayer>),
    /// A drawn shape, background or sticker (ADR-0028), composed at the output resolution.
    Shape(Box<fosscut_model::ShapeContent>),
    /// A caption page at one moment (ADR-0021), composed at the output resolution.
    Caption(Box<CaptionLayer>),
    /// An adjustment layer: its effects apply to everything below it, mixed by its
    /// opacity.
    Adjustment,
    /// Two clips over the cut between them, mixed by a transition.
    Transition(Box<TransitionLayer>),
    /// A compound clip (ADR-0025): another timeline's frame, planned at its own canvas.
    Nested(Box<RenderPlan>),
}

impl Layer {
    /// This layer and every layer inside its transitions and compound clips, at any depth:
    /// what a frame source must be able to show.
    pub fn flattened(&self) -> Vec<&Layer> {
        match &self.source {
            LayerSource::Transition(t) => {
                let mut all = t.outgoing.flattened();
                all.extend(t.incoming.flattened());
                all
            }
            LayerSource::Nested(plan) => plan.layers.iter().flat_map(Layer::flattened).collect(),
            _ => vec![self],
        }
    }
}

/// A transition in progress: the clip it leaves and the one it enters, each as its own
/// layer, and how far it has gone.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionLayer {
    /// The clip before the cut.
    pub outgoing: Layer,
    /// The clip after the cut.
    pub incoming: Layer,
    /// The transition type, such as `wipe`.
    pub kind: String,
    /// Its parameters that differ from the defaults.
    pub params: std::collections::BTreeMap<String, fosscut_model::ParamValue>,
    /// 0 at the start, 1 at the end.
    pub progress: f64,
}

/// Mask shape, in the layer's displayed coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskShape {
    /// An axis-aligned rectangle.
    Rectangle,
    /// An ellipse inscribed in the rectangle.
    Ellipse,
}

/// A layer mask (ADR-0011, item 4: masks with feather and invert).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mask {
    /// Shape.
    pub shape: MaskShape,
    /// Centre as fractions of the layer's width and height (0.5, 0.5 is the middle).
    pub center: [f32; 2],
    /// Size as fractions of the layer's width and height.
    pub size: [f32; 2],
    /// Softness of the edge as a fraction of the mask's radius, 0 (hard) to 1.
    pub feather: f32,
    /// Keep the outside instead of the inside.
    pub invert: bool,
}

/// One layer of a frame, drawn over the layers before it.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// The clip this layer comes from, when it comes from a timeline.
    pub clip: Option<Id>,
    /// What the layer shows.
    pub source: LayerSource,
    /// Degrees clockwise the source frame is rotated for display (0, 90, 180 or 270).
    pub rotation: u16,
    /// Placement relative to the fitted size.
    pub transform: Transform,
    /// The part of the picture kept, `[left, top, right, bottom]` as fractions from its
    /// top-left corner before flips ([`UNCROPPED`] keeps everything).
    pub crop: [f64; 4],
    /// Opacity, 0 to 1.
    pub opacity: f32,
    /// How the layer combines with the layers below it.
    pub blend: BlendMode,
    /// Optional mask.
    pub mask: Option<Mask>,
    /// Optional 3D LUT, applied to the display-referred signal.
    pub lut: Option<Arc<Lut3d>>,
    /// Effects applied to the layer's picture in order before placement (ADR-0011); on an
    /// adjustment layer, to the canvas below it.
    pub effects: Vec<PlannedEffect>,
    /// The clip's enabled masks at the frame's time, combined in order; on an adjustment
    /// layer they limit where its effects apply.
    pub masks: Vec<fosscut_model::Mask>,
    /// For smooth slow motion (ADR-0023 item 8): the next source frame, and how much of it
    /// mixes into the frame the source shows, 0 to 1.
    pub next_frame: Option<(SourceRef, f32)>,
    /// True when `next_frame` mixes in along the optical flow between the two frames
    /// (`Interpolate::Flow`, ADR-0035), so moving things stay sharp; false blends the two
    /// frames as they are.
    pub flow: bool,
    /// The source frames around the one shown, for effects that read them: motion blur,
    /// temporal noise reduction and flicker removal (ADR-0035).
    pub neighbours: Neighbours,
    /// Where the layer is placed at evenly spaced moments while a motion blur's shutter is
    /// open, earliest first, when it moves during the frame; empty when it holds still or
    /// has no motion blur. The layer is drawn at each placement and the draws averaged, so
    /// it streaks along its path.
    pub shutter: Vec<Transform>,
}

/// The source frames around the one a layer shows, nearest first on each side, within the
/// media (ADR-0035). Effects that compare a frame with its neighbours read them: they are
/// empty when no effect of the layer needs them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Neighbours {
    /// Earlier source frames, nearest first.
    pub before: Vec<SourceRef>,
    /// Later source frames, nearest first.
    pub after: Vec<SourceRef>,
    /// How many source frames pass during one frame of the timeline at the layer's speed:
    /// the motion between neighbouring source frames times this is the motion during one
    /// frame. 0 for a freeze frame.
    pub per_frame: f64,
}

impl Neighbours {
    /// True when there are no neighbouring frames.
    pub fn is_empty(&self) -> bool {
        self.before.is_empty() && self.after.is_empty()
    }
}

impl Layer {
    /// A full-opacity layer showing a frame of `source` with the default placement.
    pub fn new(source: SourceRef) -> Layer {
        Layer::with_source(LayerSource::Frame(source))
    }

    /// A full-opacity text layer, centred on the canvas.
    /// A layer drawing `shape`.
    pub fn shape(shape: fosscut_model::ShapeContent) -> Layer {
        Layer::with_source(LayerSource::Shape(Box::new(shape)))
    }

    /// A layer showing `content` as text that does not change over time.
    pub fn text(content: TextContent) -> Layer {
        Layer::with_source(LayerSource::Text(Box::new(TextLayer::still(content))))
    }

    /// A full-opacity caption page, placed in the canvas's safe area.
    pub fn caption(caption: CaptionLayer) -> Layer {
        Layer::with_source(LayerSource::Caption(Box::new(caption)))
    }

    pub(crate) fn with_source(source: LayerSource) -> Layer {
        Layer {
            clip: None,
            source,
            rotation: 0,
            transform: Transform::default(),
            crop: UNCROPPED,
            opacity: 1.0,
            blend: BlendMode::Normal,
            mask: None,
            lut: None,
            effects: Vec::new(),
            masks: Vec::new(),
            next_frame: None,
            flow: false,
            neighbours: Neighbours::default(),
            shutter: Vec::new(),
        }
    }
}

/// Something present at the planned time that this renderer cannot draw yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The clip.
    pub clip: Id,
    /// Why it is not drawn.
    pub reason: String,
}

/// Everything needed to render one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderPlan {
    /// Canvas size in pixels: the timeline format. Output may be rendered at another size
    /// with the same aspect.
    pub canvas: (u32, u32),
    /// Layers from bottom to top.
    pub layers: Vec<Layer>,
    /// Clips present at this time that are not drawn.
    pub skipped: Vec<Skipped>,
}

impl RenderPlan {
    /// An empty (black) frame.
    pub fn empty(canvas: (u32, u32)) -> RenderPlan {
        RenderPlan {
            canvas,
            layers: Vec::new(),
            skipped: Vec::new(),
        }
    }
}

/// The version of [`RenderPlan::cache_key_bytes`]: bump it whenever planning, the
/// compositor, an effect or a shader changes the pixels a plan renders, or the encoding
/// changes, so frames cached by an older renderer are never shown again (ADR-0032, item 4).
pub const CACHE_KEY_VERSION: u32 = 2;

impl RenderPlan {
    /// A stable encoding of everything in this plan that affects its pixels when rendered at
    /// `size`, for keying a cache of rendered frames (ADR-0032, item 4): the canvas, every
    /// layer's source with its asset and time, placement, crop, opacity, blend mode, masks,
    /// LUT, effects with their parameters, transitions, compound clips and next-frame mixes,
    /// the skipped clips, the output size and [`CACHE_KEY_VERSION`]. Equal plans encode
    /// equally on every run; any difference in those values encodes differently. Hash the
    /// bytes for a fixed-size key.
    ///
    /// Asset ids belong to one project, so callers add what each asset's file contains, and
    /// font data for project fonts, before hashing. The id of the clip a layer comes from is
    /// left out: it does not change pixels, so the same picture from another clip reuses
    /// the frame.
    ///
    /// Every struct is taken apart field by field without `..`, so a field added to a layer
    /// or a plan fails to compile here until it is encoded.
    pub fn cache_key_bytes(&self, size: (u32, u32)) -> Vec<u8> {
        let mut key = KeyBytes(Vec::with_capacity(256));
        key.text("fosscut.render.plan");
        key.u32(CACHE_KEY_VERSION);
        key.u32(size.0);
        key.u32(size.1);
        key.plan(self);
        key.0
    }
}

/// The encoding behind [`RenderPlan::cache_key_bytes`]: tags before variants, fixed-width
/// little-endian numbers, floats by their bits, and texts with their lengths, so no two
/// different sequences of values encode alike. Model types without a layout of their own
/// here (text, shapes, captions, effect parameters, masks) are encoded by their derived
/// `Debug` text, which shows every field and, through ordered maps, is the same on every run.
struct KeyBytes(Vec<u8>);

impl KeyBytes {
    fn tag(&mut self, tag: u8) {
        self.0.push(tag);
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn len(&mut self, len: usize) {
        self.u64(len as u64);
    }

    fn i64(&mut self, value: i64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }

    fn f64(&mut self, value: f64) {
        self.u64(value.to_bits());
    }

    fn bool(&mut self, value: bool) {
        self.tag(u8::from(value));
    }

    fn text(&mut self, text: &str) {
        self.len(text.len());
        self.0.extend_from_slice(text.as_bytes());
    }

    fn debug(&mut self, value: &impl std::fmt::Debug) {
        self.text(&format!("{value:?}"));
    }

    fn time(&mut self, time: Flicks) {
        self.i64(time.get());
    }

    fn plan(&mut self, plan: &RenderPlan) {
        let RenderPlan {
            canvas,
            layers,
            skipped,
        } = plan;
        self.u32(canvas.0);
        self.u32(canvas.1);
        self.len(layers.len());
        for layer in layers {
            self.layer(layer);
        }
        self.len(skipped.len());
        for Skipped { clip, reason } in skipped {
            self.text(&clip.to_string());
            self.text(reason);
        }
    }

    fn source_ref(&mut self, source: &SourceRef) {
        let SourceRef { asset, time } = source;
        self.text(&asset.to_string());
        self.time(*time);
    }

    fn layer(&mut self, layer: &Layer) {
        let Layer {
            clip: _,
            source,
            rotation,
            transform,
            crop,
            opacity,
            blend,
            mask,
            lut,
            effects,
            masks,
            next_frame,
            flow,
            neighbours,
            shutter,
        } = layer;
        self.layer_source(source);
        self.u32(u32::from(*rotation));
        self.transform(transform);
        for edge in crop {
            self.f64(*edge);
        }
        self.f32(*opacity);
        self.debug(blend);
        match mask {
            None => self.tag(0),
            Some(Mask {
                shape,
                center,
                size,
                feather,
                invert,
            }) => {
                self.tag(1);
                self.tag(match shape {
                    MaskShape::Rectangle => 0,
                    MaskShape::Ellipse => 1,
                });
                for value in center.iter().chain(size) {
                    self.f32(*value);
                }
                self.f32(*feather);
                self.bool(*invert);
            }
        }
        match lut {
            None => self.tag(0),
            Some(lut) => {
                let Lut3d { size, data } = lut.as_ref();
                self.tag(1);
                self.u32(*size);
                self.len(data.len());
                for value in data.iter().flatten() {
                    self.f32(*value);
                }
            }
        }
        self.len(effects.len());
        for effect in effects {
            let PlannedEffect {
                effect_type,
                values,
                seed,
                time,
            } = effect;
            self.text(effect_type);
            self.debug(values);
            self.u32(*seed);
            self.f64(*time);
        }
        self.debug(masks);
        match next_frame {
            None => self.tag(0),
            Some((source, amount)) => {
                self.tag(1);
                self.source_ref(source);
                self.f32(*amount);
            }
        }
        self.bool(*flow);
        let Neighbours {
            before,
            after,
            per_frame,
        } = neighbours;
        for side in [before, after] {
            self.len(side.len());
            for source in side {
                self.source_ref(source);
            }
        }
        self.f64(*per_frame);
        self.len(shutter.len());
        for transform in shutter {
            self.transform(transform);
        }
    }

    fn layer_source(&mut self, source: &LayerSource) {
        match source {
            LayerSource::Frame(source) => {
                self.tag(1);
                self.source_ref(source);
            }
            LayerSource::Text(text) => {
                let TextLayer {
                    content,
                    time,
                    duration,
                } = text.as_ref();
                self.tag(2);
                self.debug(content);
                self.time(*time);
                self.time(*duration);
            }
            LayerSource::Shape(shape) => {
                self.tag(3);
                self.debug(shape);
            }
            LayerSource::Caption(caption) => {
                let CaptionLayer {
                    style,
                    content,
                    time,
                    duration,
                } = caption.as_ref();
                self.tag(4);
                self.debug(style);
                self.debug(content);
                self.time(*time);
                self.time(*duration);
            }
            LayerSource::Adjustment => self.tag(5),
            LayerSource::Transition(transition) => {
                let TransitionLayer {
                    outgoing,
                    incoming,
                    kind,
                    params,
                    progress,
                } = transition.as_ref();
                self.tag(6);
                self.layer(outgoing);
                self.layer(incoming);
                self.text(kind);
                self.debug(params);
                self.f64(*progress);
            }
            LayerSource::Nested(plan) => {
                self.tag(7);
                self.plan(plan);
            }
        }
    }

    fn transform(&mut self, transform: &Transform) {
        let Transform {
            position,
            scale,
            stretch,
            rotation,
            flip_horizontal,
            flip_vertical,
        } = transform;
        for value in position
            .iter()
            .chain([scale])
            .chain(stretch)
            .chain([rotation])
        {
            self.f64(*value);
        }
        self.bool(*flip_horizontal);
        self.bool(*flip_vertical);
    }
}

/// The size and display rotation of an asset's frames, from the project's media table.
pub fn source_geometry(project: &Project, asset: &Id) -> Option<((u32, u32), u16)> {
    let video = project.media.get(asset)?.info.video.as_ref()?;
    Some((
        (video.width, video.height),
        clockwise_rotation(video.rotation),
    ))
}

/// The layer of `clip` on `track` at timeline time `time`, or `None` when it draws
/// nothing. `time` may lie outside the clip during a transition: video then shows the
/// media beyond the clip's edges, held at the media's first and last frames.
fn layer_for(
    project: &Project,
    format: &fosscut_model::TimelineFormat,
    track: &Track,
    clip: &Clip,
    time: Flicks,
) -> Option<Layer> {
    if !clip.enabled {
        return None;
    }
    let offset = time - clip.start;
    let frame = u32::try_from(format.rate.frame_at(time).max(0)).unwrap_or(u32::MAX);
    let effects: Vec<PlannedEffect> = clip
        .effects
        .iter()
        // Sound effects belong to the mixer (ADR-0026).
        .filter(|e| {
            e.enabled && !fosscut_model::effect_type(&e.effect_type).is_some_and(|t| t.is_audio())
        })
        .filter_map(|e| {
            Some(PlannedEffect {
                effect_type: e.effect_type.clone(),
                values: e.values_at(offset)?,
                seed: frame,
                time: offset.as_seconds_f64(),
            })
        })
        .collect();
    let masks: Vec<fosscut_model::Mask> = clip
        .masks
        .iter()
        .filter(|m| m.enabled)
        .map(|m| m.at(offset))
        .collect();
    // The clip's keyframed placement with its animations, at any offset.
    let placement = |at: Flicks| {
        let motion = animate(&clip.animations, at, clip.duration);
        let mut transform = clip.transform_at(at);
        transform.position[0] += motion.offset[0];
        transform.position[1] += motion.offset[1];
        transform.scale *= motion.scale;
        transform.rotation += motion.rotation;
        (transform, motion.opacity)
    };
    let (transform, motion_opacity) = placement(offset);
    let shutter = crate::effects::motion_blur(&effects)
        .map(|(amount, samples)| {
            let open = format.rate.frame_duration().get() as f64 * amount;
            shutter_path(&|at| placement(at).0, offset, open, samples, &transform)
        })
        .unwrap_or_default();
    let placed = |source: LayerSource, rotation: u16| Layer {
        clip: Some(clip.id.clone()),
        source,
        rotation,
        transform: transform.clone(),
        crop: clip.crop_at(offset).kept(),
        opacity: (clip.opacity_at(offset) * motion_opacity).clamp(0.0, 1.0) as f32,
        blend: clip.blend,
        mask: None,
        lut: None,
        effects: effects.clone(),
        masks: masks.clone(),
        next_frame: None,
        flow: false,
        neighbours: Neighbours::default(),
        shutter: shutter.clone(),
    };
    let mut next_frame = None;
    let mut flow = false;
    let mut neighbours = Neighbours::default();
    let source = match (&clip.content, track.kind) {
        (ClipContent::Video { asset, .. }, TrackKind::Video) => {
            let info = project.media.get(asset).map(|a| &a.info);
            let last = info
                .and_then(|i| i.duration)
                .map(|d| d - format.rate.frame_duration())
                .unwrap_or(Flicks::new(i64::MAX));
            let held = |t: Flicks| t.max(Flicks::ZERO).min(last.max(Flicks::ZERO));
            // Through the clip's speed map (ADR-0023).
            let position = clip.source_position(offset).unwrap_or(0.0);
            let source_rate = info.and_then(|i| i.video.as_ref()).and_then(|v| v.rate);
            if let (Some(speed), Some(source_rate)) = (&clip.speed, source_rate)
                && matches!(speed.interpolate, Interpolate::Blend | Interpolate::Flow)
                && !speed.is_frozen()
            {
                next_frame = between_frames(asset, position, source_rate, held);
                flow = speed.interpolate == Interpolate::Flow;
            }
            let shown = SourceRef {
                asset: asset.clone(),
                time: held(Flicks::new(position.round() as i64)),
            };
            let reach = crate::effects::neighbours_needed(&effects);
            if reach > 0
                && let Some(source_rate) = source_rate
            {
                // Source frames per timeline frame: the clip's rate here times the ratio
                // of the frame rates (ADR-0023: the map's slope is the rate).
                let rate = match &clip.speed {
                    Some(speed) if speed.is_frozen() => 0.0,
                    Some(speed) => {
                        let at = offset.get() as f64 / clip.duration.get().max(1) as f64;
                        speed.rate_at(at).abs()
                    }
                    None => 1.0,
                };
                let fps = |r: fosscut_time::Rate| f64::from(r.num()) / f64::from(r.den().max(1));
                let per_frame = rate * fps(source_rate) / fps(format.rate);
                let end = info.and_then(|i| i.duration);
                neighbours = around(asset, shown.time, source_rate, reach, end, per_frame);
            }
            shown
        }
        (ClipContent::Image { asset }, TrackKind::Video) => SourceRef {
            asset: asset.clone(),
            time: Flicks::ZERO,
        },
        (ClipContent::Text(text), _) => {
            let layer = TextLayer {
                content: (**text).clone(),
                time: offset,
                duration: clip.duration,
            };
            return Some(placed(LayerSource::Text(Box::new(layer)), 0));
        }
        (ClipContent::Shape(shape), TrackKind::Video) => {
            return Some(placed(LayerSource::Shape(shape.clone()), 0));
        }
        (ClipContent::Adjustment, TrackKind::Video) => {
            return (!effects.is_empty()).then(|| placed(LayerSource::Adjustment, 0));
        }
        (ClipContent::Timeline { timeline, .. }, TrackKind::Video) => {
            // The nested timeline's frame at the source time, held within it.
            let inner = project.timelines.get(timeline)?;
            let last = (inner.duration() - format.rate.frame_duration()).max(Flicks::ZERO);
            let at = clip
                .source_at(offset)
                .unwrap_or(Flicks::ZERO)
                .max(Flicks::ZERO)
                .min(last);
            let nested = plan_tracks(project, timeline, at, clip.angle.as_ref()).ok()?;
            return Some(placed(LayerSource::Nested(Box::new(nested)), 0));
        }
        (ClipContent::Caption(caption), _) => {
            let layer = CaptionLayer {
                style: track.caption_style.clone().unwrap_or_default(),
                content: caption.clone(),
                time: offset,
                duration: clip.duration,
            };
            return Some(placed(LayerSource::Caption(Box::new(layer)), 0));
        }
        _ => return None,
    };
    let rotation = source_geometry(project, &source.asset).map_or(0, |(_, r)| r);
    Some(Layer {
        next_frame,
        flow,
        neighbours,
        ..placed(LayerSource::Frame(source), rotation)
    })
}

/// The placements `placement` gives at `samples` moments spread evenly over a shutter open
/// for `open` flicks around `offset`, each in the middle of its share of the opening, or
/// nothing when they all equal `main`: the layer does not move during the frame.
fn shutter_path(
    placement: &dyn Fn(Flicks) -> Transform,
    offset: Flicks,
    open: f64,
    samples: usize,
    main: &Transform,
) -> Vec<Transform> {
    let samples = samples.max(1);
    let path: Vec<Transform> = (0..samples)
        .map(|i| {
            let share = (i as f64 + 0.5) / samples as f64 - 0.5;
            placement(offset + Flicks::new((share * open).round() as i64))
        })
        .collect();
    if path.iter().all(|t| t == main) {
        Vec::new()
    } else {
        path
    }
}

/// The source frames up to `reach` away from the one shown at `time` in a `rate` source of
/// `asset` that lasts `end` (unbounded when unknown), each at its middle, nearest first on
/// each side, never before the first frame or after the last.
fn around(
    asset: &Id,
    time: Flicks,
    rate: fosscut_time::Rate,
    reach: usize,
    end: Option<Flicks>,
    per_frame: f64,
) -> Neighbours {
    let index = rate.frame_at(time);
    let last = end.map(|d| rate.frame_at(d - Flicks::new(1)));
    let at = |i: i64| {
        let start = rate.frames_to_flicks(i);
        let width = rate.frames_to_flicks(i + 1) - start;
        SourceRef {
            asset: asset.clone(),
            time: start + Flicks::new(width.get() / 2),
        }
    };
    let reach = reach as i64;
    Neighbours {
        before: (1..=reach)
            .map(|k| index - k)
            .take_while(|i| *i >= 0)
            .map(at)
            .collect(),
        after: (1..=reach)
            .map(|k| index + k)
            .take_while(|i| last.is_none_or(|l| *i <= l))
            .map(at)
            .collect(),
        per_frame,
    }
}

/// The source frame after the one at `position` and how far `position` lies towards it,
/// when it falls between two frames of a `rate` source; times are frame middles, held
/// within the media by `held`.
fn between_frames(
    asset: &Id,
    position: f64,
    rate: fosscut_time::Rate,
    held: impl Fn(Flicks) -> Flicks,
) -> Option<(SourceRef, f32)> {
    let index = rate.frame_at(Flicks::new(position.floor() as i64));
    let (this, next) = (
        rate.frames_to_flicks(index),
        rate.frames_to_flicks(index + 1),
    );
    let width = (next - this).get() as f64;
    let amount = (position - this.get() as f64) / width;
    let middle = |start: Flicks| start + Flicks::new((width / 2.0) as i64);
    let time = held(middle(next));
    (amount > 1e-3 && amount < 1.0 - 1e-3 && time > held(middle(this))).then(|| {
        (
            SourceRef {
                asset: asset.clone(),
                time,
            },
            amount as f32,
        )
    })
}

/// The transition playing on `track` at `time`: the clip it leaves, the clip it enters,
/// and how far it has gone, 0 to 1. A transition is centred on the cut between a clip
/// and the next one, when the next starts where the clip ends.
fn transition_at(track: &Track, time: Flicks) -> Option<(&Clip, &Clip, f64)> {
    track.clips.windows(2).find_map(|pair| {
        let (out, into) = (&pair[0], &pair[1]);
        let t = out.transition.as_ref()?;
        if into.start != out.end() || !t.duration.is_positive() {
            return None;
        }
        let half = Flicks::new(t.duration.get() / 2);
        let begin = out.end() - half;
        let end = begin + t.duration;
        (begin <= time && time < end).then(|| {
            let progress = (time - begin).get() as f64 / t.duration.get() as f64;
            (out, into, progress)
        })
    })
}

/// Plans the frame of `timeline` displayed at `time`: every visible, enabled video, image,
/// text and caption clip that covers `time`, bottom track first, with its keyframed
/// properties and animations evaluated at that time. Over a cut with a transition, the
/// two clips form one transition layer.
pub fn plan_frame(
    project: &Project,
    timeline: &Id,
    time: Flicks,
) -> Result<RenderPlan, RenderError> {
    plan_tracks(project, timeline, time, None)
}

/// [`plan_frame`] showing only the video track `only`, and the tracks of other kinds, when
/// it is given: a multicam clip's angle (ADR-0029).
fn plan_tracks(
    project: &Project,
    timeline: &Id,
    time: Flicks,
    only: Option<&Id>,
) -> Result<RenderPlan, RenderError> {
    let doc = project
        .timelines
        .get(timeline)
        .ok_or_else(|| RenderError::TimelineNotFound {
            timeline: timeline.to_string(),
        })?;
    let mut plan = RenderPlan::empty((doc.format.width, doc.format.height));
    let shown =
        |t: &&Track| !t.hidden && only.is_none_or(|id| t.kind != TrackKind::Video || &t.id == id);
    for track in doc.tracks.iter().filter(shown) {
        if let Some((out, into, progress)) = transition_at(track, time) {
            let (Some(outgoing), Some(incoming)) = (
                layer_for(project, &doc.format, track, out, time),
                layer_for(project, &doc.format, track, into, time),
            ) else {
                // One side is off or empty: show the other one alone.
                let clip = if time < out.end() { out } else { into };
                if let Some(layer) = layer_for(project, &doc.format, track, clip, time) {
                    plan.layers
                        .extend(canvas_layer(project, &doc.format, clip, &layer));
                    plan.layers.push(layer);
                }
                continue;
            };
            // The canvas fill of the side that shows more stays under the transition.
            let (main, main_layer) = if progress < 0.5 {
                (out, &outgoing)
            } else {
                (into, &incoming)
            };
            plan.layers
                .extend(canvas_layer(project, &doc.format, main, main_layer));
            if let Some(t) = &out.transition {
                plan.layers.push(Layer {
                    clip: into.id.clone().into(),
                    ..Layer::with_source(LayerSource::Transition(Box::new(TransitionLayer {
                        outgoing,
                        incoming,
                        kind: t.transition_type.clone(),
                        params: t.params.clone(),
                        progress,
                    })))
                });
            }
            continue;
        }
        let Some(clip) = track
            .clips
            .iter()
            .find(|c| c.start <= time && time < c.end())
        else {
            continue;
        };
        if let Some(layer) = layer_for(project, &doc.format, track, clip, time) {
            plan.layers
                .extend(canvas_layer(project, &doc.format, clip, &layer));
            plan.layers.push(layer);
        }
    }
    Ok(plan)
}

/// What fills the canvas around `layer`, the picture of `clip`, when the clip has a canvas
/// fill: a solid colour, or the same picture scaled to cover the canvas and blurred.
fn canvas_layer(
    project: &Project,
    format: &fosscut_model::TimelineFormat,
    clip: &Clip,
    layer: &Layer,
) -> Option<Layer> {
    let fill = clip.canvas.as_ref()?;
    let LayerSource::Frame(source) = &layer.source else {
        return None;
    };
    match fill {
        fosscut_model::CanvasFill::Color { color } => Some(Layer {
            clip: Some(clip.id.clone()),
            ..Layer::shape(fosscut_model::ShapeContent {
                shape: fosscut_model::MaskShape::Rectangle,
                size: [1.0, 1.0],
                roundness: 0.0,
                points: Vec::new(),
                fill: fosscut_model::Paint {
                    color: color.clone(),
                    gradient: None,
                },
                outline: Default::default(),
                label: None,
                label_at: [0.5, 0.5],
            })
        }),
        fosscut_model::CanvasFill::Blur { amount } => {
            let ((w, h), _) = source_geometry(project, &source.asset)?;
            let (mut w, mut h) = (f64::from(w.max(1)), f64::from(h.max(1)));
            if layer.rotation % 180 == 90 {
                std::mem::swap(&mut w, &mut h);
            }
            let (canvas_w, canvas_h) = (f64::from(format.width), f64::from(format.height));
            // The fitted picture shows whole inside the canvas; scale it up to cover.
            let fit = (canvas_w / w).min(canvas_h / h);
            let cover = (canvas_w / (w * fit)).max(canvas_h / (h * fit));
            Some(Layer {
                clip: Some(clip.id.clone()),
                rotation: layer.rotation,
                transform: Transform {
                    scale: cover,
                    ..Transform::default()
                },
                effects: vec![PlannedEffect {
                    effect_type: "blur.gaussian".into(),
                    values: [(
                        "radius".to_string(),
                        fosscut_model::ParamValue::Number(*amount),
                    )]
                    .into(),
                    seed: 0,
                    time: 0.0,
                }],
                ..Layer::new(source.clone())
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_model::{
        Asset, AssetKind, Clip, MediaInfo, PathHints, TextContent, TextStyle, TimelineFormat,
        Track, VideoInfo,
    };
    use fosscut_time::Rate;

    fn frames(n: i64) -> Flicks {
        Rate::FPS_30.frames_to_flicks(n)
    }

    fn time(layer: &Layer) -> Option<Flicks> {
        match &layer.source {
            LayerSource::Frame(source) => Some(source.time),
            _ => None,
        }
    }

    fn clip(project: &mut Project, start: i64, length: i64, content: ClipContent) -> Clip {
        Clip::new(
            project.allocate_id(fosscut_model::IdKind::Clip),
            frames(start),
            frames(length),
            content,
        )
    }

    fn project() -> (Project, Id) {
        let format = TimelineFormat {
            width: 1920,
            height: 1080,
            rate: Rate::FPS_30,
            sample_rate: 48_000,
        };
        let mut project = Project::new(Id::new(fosscut_model::IdKind::Project, 1), "Plan", format);
        let asset = project.allocate_id(fosscut_model::IdKind::Asset);
        project.media.assets.push(Asset {
            folder: None,
            id: asset.clone(),
            kind: AssetKind::Video,
            name: "clip.mp4".into(),
            path: PathHints::default(),
            identity: None,
            info: MediaInfo {
                duration: Some(frames(300)),
                container: None,
                video: Some(VideoInfo {
                    width: 1080,
                    height: 1920,
                    rate: Some(Rate::FPS_30),
                    codec: None,
                    rotation: -90,
                }),
                audio: None,
                lut: None,
                ..Default::default()
            },
        });
        let base = clip(
            &mut project,
            0,
            100,
            ClipContent::Video {
                asset: asset.clone(),
                source_in: frames(30),
            },
        );
        let mut overlay = clip(
            &mut project,
            50,
            100,
            ClipContent::Video {
                asset: asset.clone(),
                source_in: Flicks::ZERO,
            },
        );
        overlay.opacity = 1.5;
        let title = clip(
            &mut project,
            0,
            90,
            ClipContent::Text(Box::new(TextContent {
                text: "Hi".into(),
                style: TextStyle::default(),
                ..Default::default()
            })),
        );
        let timeline_id = project.manifest.primary_timeline.clone();
        let tracks = vec![
            (TrackKind::Video, false, vec![base]),
            (TrackKind::Video, false, vec![overlay]),
            (TrackKind::Text, false, vec![title]),
        ];
        for (kind, hidden, clips) in tracks {
            let id = project.allocate_id(fosscut_model::IdKind::Track);
            project
                .timelines
                .get_mut(&timeline_id)
                .unwrap()
                .tracks
                .push(Track {
                    id,
                    kind,
                    name: None,
                    muted: false,
                    hidden,
                    locked: false,
                    caption_style: None,
                    clips,
                });
        }
        (project, timeline_id)
    }

    #[test]
    fn layers_follow_track_order_and_source_times() {
        let (project, timeline) = project();
        let plan = plan_frame(&project, &timeline, frames(60)).unwrap();
        assert_eq!(plan.canvas, (1920, 1080));
        assert_eq!(plan.layers.len(), 3, "two video layers and the title");
        assert_eq!(
            time(&plan.layers[0]),
            Some(frames(90)),
            "source_in + offset"
        );
        assert_eq!(time(&plan.layers[1]), Some(frames(10)));
        assert!(matches!(&plan.layers[2].source, LayerSource::Text(t) if t.content.text == "Hi"));
        assert_eq!(plan.layers[2].rotation, 0);
        assert_eq!(plan.layers[1].opacity, 1.0, "opacity is clamped");
        assert_eq!(
            plan.layers[0].rotation, 90,
            "from the media table, clockwise"
        );
        assert!(plan.skipped.is_empty());
        let plan = plan_frame(&project, &timeline, frames(120)).unwrap();
        assert_eq!(
            plan.layers.len(),
            1,
            "the base clip and the title have ended"
        );
        assert!(
            plan_frame(&project, &timeline, frames(400))
                .unwrap()
                .layers
                .is_empty()
        );
    }

    #[test]
    fn hidden_tracks_are_not_drawn_and_unknown_timelines_fail() {
        let (mut project, timeline) = project();
        project.timelines.get_mut(&timeline).unwrap().tracks[1].hidden = true;
        let plan = plan_frame(&project, &timeline, frames(60)).unwrap();
        assert_eq!(plan.layers.len(), 2, "base video and title");
        let missing = plan_frame(
            &project,
            &Id::new(fosscut_model::IdKind::Timeline, 9),
            Flicks::ZERO,
        );
        assert_eq!(missing.unwrap_err().code(), "fosscut.timeline.not_found");
    }

    #[test]
    fn keyframes_are_evaluated_and_disabled_clips_skipped() {
        let (mut project, timeline) = project();
        let doc = project.timelines.get_mut(&timeline).unwrap();
        let base = &mut doc.tracks[0].clips[0];
        base.keyframes.insert(
            "opacity".into(),
            vec![
                fosscut_model::Keyframe {
                    time: frames(0),
                    value: fosscut_model::KeyValue::Number(0.0),
                    interpolation: fosscut_model::Interpolation::Linear,
                },
                fosscut_model::Keyframe {
                    time: frames(100),
                    value: fosscut_model::KeyValue::Number(1.0),
                    interpolation: fosscut_model::Interpolation::Linear,
                },
            ],
        );
        base.crop.left = 0.25;
        base.blend = BlendMode::Screen;
        base.transform.flip_vertical = true;
        doc.tracks[1].clips[0].enabled = false;
        let plan = plan_frame(&project, &timeline, frames(60)).unwrap();
        assert_eq!(plan.layers.len(), 2, "the disabled overlay is not drawn");
        let layer = &plan.layers[0];
        assert!((layer.opacity - 0.6).abs() < 1e-6, "{}", layer.opacity);
        assert_eq!(layer.crop, [0.25, 0.0, 1.0, 1.0]);
        assert_eq!(layer.blend, BlendMode::Screen);
        assert!(layer.transform.flip_vertical);
        assert_eq!(plan.layers[1].crop, UNCROPPED, "the title is not cropped");
    }

    #[test]
    fn effects_are_planned_at_the_frame_and_adjustments_become_layers() {
        let (mut project, timeline) = project();
        let effect = |id: u64, kind: &str, enabled: bool| fosscut_model::Effect {
            id: Id::new(fosscut_model::IdKind::Effect, id),
            effect_type: kind.into(),
            version: 1,
            enabled,
            params: [(
                "exposure".to_string(),
                fosscut_model::ParamValue::Number(0.5),
            )]
            .into_iter()
            .filter(|_| kind == "color.adjust")
            .collect(),
            keyframes: Default::default(),
        };
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks[0].clips[0].effects = vec![
            effect(1, "color.adjust", true),
            effect(2, "color.vignette", false),
            effect(3, "color.future", true),
        ];
        let adjustment = |id: u64, effects| fosscut_model::Clip {
            effects,
            ..Clip::new(
                Id::new(fosscut_model::IdKind::Clip, id),
                frames(0),
                frames(100),
                ClipContent::Adjustment,
            )
        };
        doc.tracks[1].clips = vec![adjustment(50, vec![effect(4, "stylize.grain", true)])];
        let plan = plan_frame(&project, &timeline, frames(60)).unwrap();
        let base = &plan.layers[0];
        assert_eq!(
            base.effects.len(),
            1,
            "disabled and unknown effects are left out"
        );
        assert_eq!(base.effects[0].effect_type, "color.adjust");
        assert_eq!(
            base.effects[0].values["exposure"],
            fosscut_model::ParamValue::Number(0.5)
        );
        assert_eq!(base.effects[0].seed, 60, "the timeline frame");
        assert!(matches!(plan.layers[1].source, LayerSource::Adjustment));
        assert_eq!(plan.layers[1].effects[0].effect_type, "stylize.grain");
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks[1].clips = vec![adjustment(51, Vec::new())];
        let plan = plan_frame(&project, &timeline, frames(60)).unwrap();
        assert!(
            plan.layers
                .iter()
                .all(|l| !matches!(l.source, LayerSource::Adjustment)),
            "an adjustment without effects draws nothing"
        );
    }

    #[test]
    fn source_geometry_uses_the_media_table() {
        let (project, _) = project();
        let asset = project.media.assets[0].id.clone();
        assert_eq!(source_geometry(&project, &asset), Some(((1080, 1920), 90)));
        assert_eq!(
            source_geometry(&project, &Id::new(fosscut_model::IdKind::Asset, 99)),
            None
        );
        let layer = Layer::new(SourceRef {
            asset,
            time: Flicks::ZERO,
        });
        assert_eq!((layer.opacity, layer.mask, layer.clip), (1.0, None, None));
    }

    #[test]
    fn caption_clips_carry_their_track_style_and_local_time() {
        let (mut project, timeline) = project();
        let page = clip(
            &mut project,
            30,
            45,
            ClipContent::Caption(fosscut_model::CaptionContent {
                lines: vec![fosscut_model::CaptionLine {
                    words: vec![fosscut_model::CaptionWord {
                        text: "Hello".into(),
                        start: Flicks::ZERO,
                        end: frames(20),
                        emphasis: false,
                    }],
                }],
                ..Default::default()
            }),
        );
        let style = fosscut_model::CaptionStyle {
            uppercase: true,
            ..fosscut_model::CaptionStyle::default()
        };
        let id = project.allocate_id(fosscut_model::IdKind::Track);
        project
            .timelines
            .get_mut(&timeline)
            .unwrap()
            .tracks
            .push(Track {
                id,
                kind: TrackKind::Caption,
                name: None,
                muted: false,
                hidden: false,
                locked: false,
                caption_style: Some(style.clone()),
                clips: vec![page],
            });
        let plan = plan_frame(&project, &timeline, frames(40)).unwrap();
        let caption = plan.layers.last().unwrap();
        let LayerSource::Caption(layer) = &caption.source else {
            panic!("expected a caption layer, got {:?}", caption.source);
        };
        assert_eq!(layer.style, style);
        assert_eq!(layer.time, frames(10), "time from the clip's start");
        assert_eq!(layer.duration, frames(45));
        assert_eq!(layer.content.text(), "Hello");
        assert!(time(caption).is_none());
        // Outside the page there is no caption layer.
        let later = plan_frame(&project, &timeline, frames(80)).unwrap();
        assert!(
            later
                .layers
                .iter()
                .all(|l| !matches!(l.source, LayerSource::Caption(_)))
        );
    }

    #[test]
    fn video_follows_the_speed_map_and_blends_between_frames() {
        let (mut project, timeline) = project();
        fn base<'a>(project: &'a mut Project, timeline: &Id) -> &'a mut Clip {
            &mut project.timelines.get_mut(timeline).unwrap().tracks[0].clips[0]
        }
        // Only the bottom clip: it starts at frame 30 of its media.
        project
            .timelines
            .get_mut(&timeline)
            .unwrap()
            .tracks
            .truncate(1);
        base(&mut project, &timeline).speed = Some(fosscut_model::Speed::constant(2.0));
        let at = |project: &Project, n: i64| plan_frame(project, &timeline, frames(n)).unwrap();
        assert_eq!(
            time(&at(&project, 10).layers[0]),
            Some(frames(50)),
            "30 + 2 x 10"
        );
        assert!(
            at(&project, 10).layers[0].next_frame.is_none(),
            "nearest by default"
        );
        base(&mut project, &timeline)
            .speed
            .as_mut()
            .unwrap()
            .reverse = true;
        assert_eq!(
            time(&at(&project, 0).layers[0]),
            Some(frames(230)),
            "reversed: the end of the 200 frames it uses"
        );
        base(&mut project, &timeline).speed = Some(fosscut_model::Speed {
            interpolate: Interpolate::Blend,
            ..fosscut_model::Speed::constant(0.5)
        });
        let half = at(&project, 1);
        let shown = time(&half.layers[0]).unwrap();
        assert_eq!(
            shown,
            frames(30) + Flicks::new(frames(1).get() / 2),
            "half a frame in"
        );
        assert_eq!(Rate::FPS_30.frame_at(shown), 30, "which shows frame 30");
        let (next, amount) = half.layers[0].next_frame.clone().unwrap();
        assert!((amount - 0.5).abs() < 1e-6, "{amount}");
        assert_eq!(
            Rate::FPS_30.frame_at(next.time),
            31,
            "the next source frame"
        );
        assert!(
            at(&project, 2).layers[0].next_frame.is_none(),
            "on a source frame, nothing to blend"
        );
        base(&mut project, &timeline).speed = Some(fosscut_model::Speed::constant(0.0));
        assert_eq!(
            time(&at(&project, 50).layers[0]),
            Some(frames(30)),
            "frozen"
        );
    }

    #[test]
    fn temporal_effects_plan_neighbouring_frames() {
        let (mut project, timeline) = project();
        project
            .timelines
            .get_mut(&timeline)
            .unwrap()
            .tracks
            .truncate(1);
        let effect =
            |kind: &str, params: &[(&str, fosscut_model::ParamValue)]| fosscut_model::Effect {
                id: Id::new(fosscut_model::IdKind::Effect, 9),
                effect_type: kind.into(),
                version: 1,
                enabled: true,
                params: params
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
                keyframes: Default::default(),
            };
        let number = fosscut_model::ParamValue::Number;
        let edit = |project: &mut Project, change: &dyn Fn(&mut Clip)| {
            change(&mut project.timelines.get_mut(&timeline).unwrap().tracks[0].clips[0]);
        };
        let at = |project: &Project, n: i64| {
            plan_frame(project, &timeline, frames(n)).unwrap().layers[0].clone()
        };
        // Frame middles of the 30 fps source.
        let middle = |i: i64| frames(i) + Flicks::new(frames(1).get() / 2);
        let times = |side: &[SourceRef]| side.iter().map(|s| s.time).collect::<Vec<_>>();

        assert!(
            at(&project, 10).neighbours.is_empty(),
            "nothing asks for them"
        );
        edit(&mut project, &|clip| {
            clip.effects = vec![effect("detail.denoise", &[])]
        });
        let layer = at(&project, 10);
        assert_eq!(time(&layer), Some(frames(40)), "30 frames in, 10 more");
        assert_eq!(times(&layer.neighbours.before), [middle(39)]);
        assert_eq!(times(&layer.neighbours.after), [middle(41)]);
        assert_eq!(layer.neighbours.per_frame, 1.0);
        assert!(layer.shutter.is_empty() && !layer.flow);

        edit(&mut project, &|clip| {
            clip.effects = vec![effect(
                "detail.denoise",
                &[("temporal", fosscut_model::ParamValue::Flag(false))],
            )]
        });
        assert!(at(&project, 10).neighbours.is_empty(), "spatial only");

        // A window of 6 frames reaches 3 to each side, within the media.
        edit(&mut project, &|clip| {
            clip.effects = vec![effect("color.deflicker", &[("window", number(6.0))])]
        });
        let layer = at(&project, 10);
        assert_eq!(
            times(&layer.neighbours.before),
            [middle(39), middle(38), middle(37)]
        );
        assert_eq!(
            times(&layer.neighbours.after),
            [middle(41), middle(42), middle(43)]
        );
        edit(&mut project, &|clip| {
            clip.content = ClipContent::Video {
                asset: Id::new(fosscut_model::IdKind::Asset, 1),
                source_in: frames(1),
            }
        });
        let layer = at(&project, 0);
        assert_eq!(
            times(&layer.neighbours.before),
            [middle(0)],
            "the media's first frame"
        );
        edit(&mut project, &|clip| {
            clip.content = ClipContent::Video {
                asset: Id::new(fosscut_model::IdKind::Asset, 1),
                source_in: frames(298),
            };
            clip.duration = frames(2);
        });
        let layer = at(&project, 1);
        assert!(
            layer.neighbours.after.is_empty(),
            "past the media's last frame"
        );
        assert_eq!(layer.neighbours.before.len(), 3);

        // Speed scales the motion per frame; a freeze has none.
        edit(&mut project, &|clip| {
            clip.content = ClipContent::Video {
                asset: Id::new(fosscut_model::IdKind::Asset, 1),
                source_in: frames(30),
            };
            clip.duration = frames(100);
            clip.effects = vec![effect("motion.blur", &[])];
            clip.speed = Some(fosscut_model::Speed::constant(2.0));
        });
        assert_eq!(at(&project, 10).neighbours.per_frame, 2.0);
        edit(&mut project, &|clip| {
            clip.speed = Some(fosscut_model::Speed::constant(0.0))
        });
        assert_eq!(at(&project, 10).neighbours.per_frame, 0.0);
        // Optical-flow slow motion carries both frames, like blending, and says so.
        edit(&mut project, &|clip| {
            clip.effects.clear();
            clip.speed = Some(fosscut_model::Speed {
                interpolate: Interpolate::Flow,
                ..fosscut_model::Speed::constant(0.5)
            })
        });
        let layer = at(&project, 1);
        assert!(layer.flow);
        let (next, amount) = layer.next_frame.clone().unwrap();
        assert_eq!(Rate::FPS_30.frame_at(next.time), 31);
        assert!((amount - 0.5).abs() < 1e-6);
        assert!(layer.neighbours.is_empty());
        let blend_key = {
            let mut blended = project.clone();
            edit(&mut blended, &|clip| {
                clip.speed.as_mut().unwrap().interpolate = Interpolate::Blend
            });
            plan_frame(&blended, &timeline, frames(1))
                .unwrap()
                .cache_key_bytes((640, 360))
        };
        assert_ne!(
            plan_frame(&project, &timeline, frames(1))
                .unwrap()
                .cache_key_bytes((640, 360)),
            blend_key,
            "flow and blending render differently"
        );
    }

    #[test]
    fn motion_blur_spreads_placements_over_the_shutter() {
        let (mut project, timeline) = project();
        let title = &mut project.timelines.get_mut(&timeline).unwrap().tracks[2].clips[0];
        let key = |frame: i64, x: f64| fosscut_model::Keyframe {
            time: frames(frame),
            value: fosscut_model::KeyValue::Pair([x, 0.0]),
            interpolation: fosscut_model::Interpolation::Linear,
        };
        // Moves 0.3 of the canvas a frame.
        title
            .keyframes
            .insert("transform.position".into(), vec![key(0, 0.0), key(2, 0.6)]);
        title.effects = vec![fosscut_model::Effect {
            id: Id::new(fosscut_model::IdKind::Effect, 9),
            effect_type: "motion.blur".into(),
            version: 1,
            enabled: true,
            params: [
                ("amount".to_string(), fosscut_model::ParamValue::Number(0.5)),
                (
                    "samples".to_string(),
                    fosscut_model::ParamValue::Number(4.4),
                ),
            ]
            .into(),
            keyframes: Default::default(),
        }];
        let layer = |project: &Project, n: i64| {
            plan_frame(project, &timeline, frames(n))
                .unwrap()
                .layers
                .into_iter()
                .find(|l| matches!(l.source, LayerSource::Text(_)))
                .unwrap()
        };
        let moving = layer(&project, 1);
        assert_eq!(moving.transform.position[0], 0.3);
        let xs: Vec<f64> = moving.shutter.iter().map(|t| t.position[0]).collect();
        assert_eq!(xs.len(), 4, "samples round to a whole number");
        // A half-frame shutter around the frame: 0.15 of the canvas, sampled at the middle
        // of each quarter.
        for (x, expected) in xs.iter().zip([0.24375, 0.28125, 0.31875, 0.35625]) {
            assert!((x - expected).abs() < 1e-6, "{xs:?}");
        }
        // Holding still after the last keyframe: no streak.
        assert!(layer(&project, 50).shutter.is_empty());
        let mut closed = project.clone();
        closed.timelines.get_mut(&timeline).unwrap().tracks[2].clips[0].effects[0]
            .params
            .insert("amount".into(), fosscut_model::ParamValue::Number(0.0));
        assert!(layer(&closed, 1).shutter.is_empty(), "a closed shutter");
        assert_ne!(
            plan_frame(&project, &timeline, frames(1))
                .unwrap()
                .cache_key_bytes((640, 360)),
            plan_frame(&closed, &timeline, frames(1))
                .unwrap()
                .cache_key_bytes((640, 360)),
        );
    }

    #[test]
    fn compound_clips_plan_their_timeline_at_the_source_time() {
        let (mut project, timeline) = project();
        // The video track's clips move into a compound timeline shown from frame 10.
        let inner_id = project.allocate_id(fosscut_model::IdKind::Timeline);
        let mut inner = project.timelines[&timeline].clone();
        inner.id = inner_id.clone();
        inner.tracks.truncate(1);
        project
            .manifest
            .timelines
            .push(fosscut_model::TimelineEntry {
                id: inner_id.clone(),
                name: "Inside".into(),
                role: fosscut_model::TimelineRole::Compound,
            });
        project.timelines.insert(inner_id.clone(), inner);
        let compound = clip(
            &mut project,
            0,
            50,
            ClipContent::Timeline {
                timeline: inner_id,
                source_in: frames(10),
            },
        );
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks.truncate(1);
        doc.tracks[0].clips = vec![compound];
        let plan = plan_frame(&project, &timeline, frames(5)).unwrap();
        let LayerSource::Nested(nested) = &plan.layers[0].source else {
            panic!("a nested plan");
        };
        assert_eq!(nested.canvas, (1920, 1080));
        assert_eq!(
            time(&nested.layers[0]),
            Some(frames(45)),
            "frame 15 inside, 30 frames into its media"
        );
        assert_eq!(plan.layers[0].flattened().len(), 1, "the inner layer");
    }

    #[test]
    fn cache_keys_change_with_what_a_frame_shows_and_only_then() {
        let (project, timeline) = project();
        let key = |project: &Project, n: i64| {
            plan_frame(project, &timeline, frames(n))
                .unwrap()
                .cache_key_bytes((640, 360))
        };
        let base = key(&project, 60);
        assert_eq!(base, key(&project, 60), "the same frame again");
        assert_eq!(base, key(&project.clone(), 60), "a copy of the project");
        assert_ne!(base, key(&project, 61), "another time");
        let plan = plan_frame(&project, &timeline, frames(60)).unwrap();
        assert_ne!(
            plan.cache_key_bytes((640, 360)),
            plan.cache_key_bytes((320, 180)),
            "another output size"
        );

        // Each edit of the bottom clip changes the frame's key.
        let edited = |edit: &dyn Fn(&mut Clip)| {
            let mut copy = project.clone();
            edit(&mut copy.timelines.get_mut(&timeline).unwrap().tracks[0].clips[0]);
            copy
        };
        let effect = |exposure: f64| fosscut_model::Effect {
            id: Id::new(fosscut_model::IdKind::Effect, 7),
            effect_type: "color.adjust".into(),
            version: 1,
            enabled: true,
            params: [(
                "exposure".to_string(),
                fosscut_model::ParamValue::Number(exposure),
            )]
            .into_iter()
            .collect(),
            keyframes: Default::default(),
        };
        let mask = |feather: f64| fosscut_model::Mask {
            feather,
            ..fosscut_model::Mask::new(
                Id::new(fosscut_model::IdKind::Mask, 8),
                fosscut_model::MaskShape::Ellipse,
            )
        };
        let with_effect = edited(&|clip| clip.effects = vec![effect(0.5)]);
        let with_mask = edited(&|clip| clip.masks = vec![mask(0.2)]);
        let edits: Vec<(&str, Project)> = vec![
            ("position", edited(&|clip| clip.transform.position[0] = 0.1)),
            ("scale", edited(&|clip| clip.transform.scale = 0.5)),
            ("stretch", edited(&|clip| clip.transform.stretch[1] = 1.1)),
            ("rotation", edited(&|clip| clip.transform.rotation = 3.0)),
            (
                "flip",
                edited(&|clip| clip.transform.flip_horizontal = true),
            ),
            ("opacity", edited(&|clip| clip.opacity = 0.5)),
            ("crop", edited(&|clip| clip.crop.left = 0.1)),
            ("blend", edited(&|clip| clip.blend = BlendMode::Screen)),
            (
                "source time",
                edited(&|clip| {
                    clip.content = ClipContent::Video {
                        asset: Id::new(fosscut_model::IdKind::Asset, 1),
                        source_in: frames(31),
                    }
                }),
            ),
            ("an effect", with_effect.clone()),
            ("a mask", with_mask.clone()),
        ];
        let mut keys = vec![base.clone()];
        for (what, edited) in &edits {
            let changed = key(edited, 60);
            assert!(!keys.contains(&changed), "{what} changes the key");
            keys.push(changed);
        }
        let tweaked = |project: &Project, edit: &dyn Fn(&mut Clip)| {
            let mut copy = project.clone();
            edit(&mut copy.timelines.get_mut(&timeline).unwrap().tracks[0].clips[0]);
            key(&copy, 60)
        };
        assert_ne!(
            key(&with_effect, 60),
            tweaked(&with_effect, &|clip| clip.effects = vec![effect(0.6)]),
            "an effect parameter"
        );
        assert_ne!(
            key(&with_effect, 60),
            tweaked(&with_effect, &|clip| clip.effects[0].enabled = false),
            "turning an effect off"
        );
        assert_ne!(
            key(&with_mask, 60),
            tweaked(&with_mask, &|clip| clip.masks = vec![mask(0.3)]),
            "a mask's feather"
        );
        let mut retitled = project.clone();
        if let ClipContent::Text(text) =
            &mut retitled.timelines.get_mut(&timeline).unwrap().tracks[2].clips[0].content
        {
            text.text = "Hello".into();
        }
        assert_ne!(key(&project, 10), key(&retitled, 10), "the title's text");

        // Frames the edited clip is not in keep their keys: only the overlay shows at 120.
        for (what, edited) in &edits {
            assert_eq!(key(edited, 120), key(&project, 120), "{what} elsewhere");
        }
        // The clip a layer comes from is not part of it: the same picture from another
        // clip has the same key.
        let renamed = edited(&|clip| clip.id = Id::new(fosscut_model::IdKind::Clip, 99));
        assert_eq!(key(&renamed, 60), base);
    }

    #[test]
    fn cache_keys_cover_transitions_compound_clips_and_luts() {
        let (mut project, timeline) = project();
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks.truncate(1);
        let first = doc.tracks[0].clips[0].clone();
        let mut second = first.clone();
        second.id = Id::new(fosscut_model::IdKind::Clip, 60);
        second.start = first.end();
        let mut first = first;
        first.transition = Some(fosscut_model::Transition {
            transition_type: "dissolve".into(),
            duration: frames(10),
            params: Default::default(),
        });
        doc.tracks[0].clips = vec![first, second];
        let at = |project: &Project| plan_frame(project, &timeline, frames(98)).unwrap();
        let plan = at(&project);
        assert!(matches!(plan.layers[0].source, LayerSource::Transition(_)));
        let key = plan.cache_key_bytes((640, 360));
        let mut slower = project.clone();
        slower.timelines.get_mut(&timeline).unwrap().tracks[0].clips[0]
            .transition
            .as_mut()
            .unwrap()
            .duration = frames(20);
        assert_ne!(at(&slower).cache_key_bytes((640, 360)), key, "progress");
        let mut wiped = project.clone();
        wiped.timelines.get_mut(&timeline).unwrap().tracks[0].clips[0]
            .transition
            .as_mut()
            .unwrap()
            .transition_type = "wipe".into();
        assert_ne!(at(&wiped).cache_key_bytes((640, 360)), key, "the kind");
        let mut incoming = project.clone();
        incoming.timelines.get_mut(&timeline).unwrap().tracks[0].clips[1].opacity = 0.5;
        assert_ne!(
            at(&incoming).cache_key_bytes((640, 360)),
            key,
            "the clip it enters"
        );

        let mut nested = RenderPlan::empty((1920, 1080));
        nested.layers.push(plan.layers[0].clone());
        let outer = |inner: RenderPlan| {
            let mut outer = RenderPlan::empty((1920, 1080));
            outer
                .layers
                .push(Layer::with_source(LayerSource::Nested(Box::new(inner))));
            outer.cache_key_bytes((640, 360))
        };
        let mut moved = nested.clone();
        moved.layers[0].transform.position[1] = 0.2;
        assert_ne!(
            outer(nested.clone()),
            outer(moved),
            "inside a compound clip"
        );

        let lut = |value: f32| {
            let mut layer = plan.layers[0].clone();
            layer.lut = Some(Arc::new(Lut3d {
                size: 2,
                data: vec![[value, 0.0, 0.0]; 8],
            }));
            let mut with = RenderPlan::empty((1920, 1080));
            with.layers.push(layer);
            with.cache_key_bytes((640, 360))
        };
        assert_ne!(lut(0.5), lut(0.25), "the LUT's table");
        let mut skipping = plan.clone();
        skipping.skipped.push(Skipped {
            clip: Id::new(fosscut_model::IdKind::Clip, 70),
            reason: "unsupported".into(),
        });
        assert_ne!(skipping.cache_key_bytes((640, 360)), key, "skipped clips");
    }

    #[test]
    fn a_multicam_angle_shows_only_its_track() {
        let (mut project, timeline) = project();
        let inner_id = project.allocate_id(fosscut_model::IdKind::Timeline);
        let mut inner = project.timelines[&timeline].clone();
        inner.id = inner_id.clone();
        inner.tracks.truncate(1);
        // A second angle: the same picture on a track of its own.
        let mut second = inner.tracks[0].clone();
        second.id = project.allocate_id(fosscut_model::IdKind::Track);
        for clip in &mut second.clips {
            clip.id = project.allocate_id(fosscut_model::IdKind::Clip);
        }
        let angle = second.id.clone();
        inner.tracks.push(second);
        project
            .manifest
            .timelines
            .push(fosscut_model::TimelineEntry {
                id: inner_id.clone(),
                name: "Angles".into(),
                role: fosscut_model::TimelineRole::Compound,
            });
        project.timelines.insert(inner_id.clone(), inner);
        let mut multicam = clip(
            &mut project,
            0,
            50,
            ClipContent::Timeline {
                timeline: inner_id,
                source_in: Flicks::ZERO,
            },
        );
        let doc = project.timelines.get_mut(&timeline).unwrap();
        doc.tracks.truncate(1);
        doc.tracks[0].clips = vec![multicam.clone()];
        let nested_layers = |project: &Project| {
            let plan = plan_frame(project, &timeline, frames(5)).unwrap();
            let LayerSource::Nested(nested) = &plan.layers[0].source else {
                panic!("a nested plan");
            };
            nested.layers.len()
        };
        assert_eq!(nested_layers(&project), 2, "both angles without a choice");
        multicam.angle = Some(angle);
        project.timelines.get_mut(&timeline).unwrap().tracks[0].clips = vec![multicam];
        let angle_issues: Vec<_> = fosscut_model::validate(&project)
            .into_iter()
            .filter(|i| i.code == "fosscut.model.angle")
            .collect();
        assert!(angle_issues.is_empty(), "{angle_issues:?}");
        assert_eq!(nested_layers(&project), 1, "only the chosen angle");
    }
}
