//! Render plans: what to draw for one frame (ADR-0008, item 2). Planning is a pure
//! function of the project and a time; the GPU only executes plans.

use std::sync::Arc;

use fosscut_model::{BlendMode, ClipContent, Id, Project, TextContent, TrackKind, Transform};
use fosscut_time::Flicks;

use crate::RenderError;
use crate::caption::CaptionLayer;
use crate::geometry::{UNCROPPED, clockwise_rotation};
use crate::lut::Lut3d;

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
    /// Styled text, rasterized at the output resolution.
    Text(TextContent),
    /// A caption page at one moment (ADR-0021), composed at the output resolution.
    Caption(Box<CaptionLayer>),
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
}

impl Layer {
    /// A full-opacity layer showing a frame of `source` with the default placement.
    pub fn new(source: SourceRef) -> Layer {
        Layer::with_source(LayerSource::Frame(source))
    }

    /// A full-opacity text layer, centred on the canvas.
    pub fn text(content: TextContent) -> Layer {
        Layer::with_source(LayerSource::Text(content))
    }

    /// A full-opacity caption page, placed in the canvas's safe area.
    pub fn caption(caption: CaptionLayer) -> Layer {
        Layer::with_source(LayerSource::Caption(Box::new(caption)))
    }

    fn with_source(source: LayerSource) -> Layer {
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

/// The size and display rotation of an asset's frames, from the project's media table.
pub fn source_geometry(project: &Project, asset: &Id) -> Option<((u32, u32), u16)> {
    let video = project.media.get(asset)?.info.video.as_ref()?;
    Some((
        (video.width, video.height),
        clockwise_rotation(video.rotation),
    ))
}

/// Plans the frame of `timeline` displayed at `time`: every visible, enabled video, image,
/// text and caption clip that covers `time`, bottom track first, with its keyframed
/// properties evaluated at that time.
pub fn plan_frame(
    project: &Project,
    timeline: &Id,
    time: Flicks,
) -> Result<RenderPlan, RenderError> {
    let doc = project
        .timelines
        .get(timeline)
        .ok_or_else(|| RenderError::TimelineNotFound {
            timeline: timeline.to_string(),
        })?;
    let mut plan = RenderPlan::empty((doc.format.width, doc.format.height));
    for track in doc.tracks.iter().filter(|t| !t.hidden) {
        let Some(clip) = track
            .clips
            .iter()
            .find(|c| c.start <= time && time < c.end())
        else {
            continue;
        };
        if !clip.enabled {
            continue;
        }
        let offset = time - clip.start;
        let placed = |source: LayerSource, rotation: u16| Layer {
            clip: Some(clip.id.clone()),
            source,
            rotation,
            transform: clip.transform_at(offset),
            crop: clip.crop_at(offset).kept(),
            opacity: clip.opacity_at(offset) as f32,
            blend: clip.blend,
            mask: None,
            lut: None,
        };
        let source = match (&clip.content, track.kind) {
            (ClipContent::Video { asset, source_in }, TrackKind::Video) => SourceRef {
                asset: asset.clone(),
                time: *source_in + offset,
            },
            (ClipContent::Image { asset }, TrackKind::Video) => SourceRef {
                asset: asset.clone(),
                time: Flicks::ZERO,
            },
            (ClipContent::Text(text), _) => {
                plan.layers.push(placed(LayerSource::Text(text.clone()), 0));
                continue;
            }
            (ClipContent::Caption(caption), _) => {
                let layer = CaptionLayer {
                    style: track.caption_style.clone().unwrap_or_default(),
                    content: caption.clone(),
                    time: offset,
                    duration: clip.duration,
                };
                plan.layers
                    .push(placed(LayerSource::Caption(Box::new(layer)), 0));
                continue;
            }
            _ => continue,
        };
        let rotation = source_geometry(project, &source.asset).map_or(0, |(_, r)| r);
        plan.layers
            .push(placed(LayerSource::Frame(source), rotation));
    }
    Ok(plan)
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
            LayerSource::Text(_) | LayerSource::Caption(_) => None,
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
            ClipContent::Text(TextContent {
                text: "Hi".into(),
                style: TextStyle::default(),
            }),
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
        assert!(matches!(&plan.layers[2].source, LayerSource::Text(t) if t.text == "Hi"));
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
}
