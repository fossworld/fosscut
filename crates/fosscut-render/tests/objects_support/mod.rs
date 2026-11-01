//! Scenes with 3D objects (ADR-0040) shared by the object tests and their golden frames: a
//! project on a 1920x1080 canvas, pictures for floors, cards and logos, a frame provider
//! that also holds loaded glTF models, and the scenes themselves.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;

use fosscut_media::{PixelFormat, Plane, VideoFrame};
use fosscut_model::{
    Asset, AssetKind, CameraContent, Clip, ClipContent, Id, IdKind, MediaInfo, ModelContent,
    ModelInfo, PathHints, Project, TimelineFormat, Track, TrackKind, Transform, VideoInfo,
};
use fosscut_render::objects::ModelData;
use fosscut_render::plan::SourceRef;
use fosscut_render::{FrameProvider, RenderError};
use fosscut_time::{Flicks, Rate};

use super::support::{Frames, asset, bt709_limited, nv12, solid};

pub const CANVAS: (u32, u32) = (1920, 1080);
/// The wooden floor.
pub const FLOOR: u64 = 1;
/// A logo with a transparent background.
pub const LOGO: u64 = 2;
/// Cards: coral and teal.
pub const CORAL: u64 = 3;
pub const TEAL: u64 = 4;
/// A dark backdrop.
pub const BACKDROP: u64 = 5;
/// The glTF house.
pub const HOUSE: u64 = 6;

/// Warm planks: stripes of slightly different browns with darker seams, lighter towards
/// the top as if lit from a window.
pub fn planks(width: u32, height: u32) -> VideoFrame {
    nv12(width, height, bt709_limited(), move |x, y| {
        let plank = x * 9 / width;
        let seam = (x * 9 % width) < width / 90;
        let grain = ((x * 7 + y * 3 + plank * 37) % 23) as u8;
        let shade = 120 - (y * 30 / height) as u8 + (plank % 3) as u8 * 6 + grain / 4;
        let luma = if seam { shade - 40 } else { shade };
        [luma, 104, 150]
    })
}

/// A white ring with a dot, on transparency, as straight-alpha R'G'B'A.
pub fn logo(side: u32) -> VideoFrame {
    let mut data = Vec::with_capacity((side * side * 4) as usize);
    let half = side as f32 / 2.0;
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (x as f32 + 0.5 - half, y as f32 + 0.5 - half);
            let r = (dx * dx + dy * dy).sqrt() / half;
            let ring = (1.0 - ((r - 0.78).abs() - 0.1) * side as f32 / 4.0).clamp(0.0, 1.0);
            let dot = (1.0 - (r - 0.3) * side as f32 / 4.0).clamp(0.0, 1.0);
            let alpha = ring.max(dot);
            data.extend_from_slice(&[40, 30, 25, (alpha * 255.0).round() as u8]);
        }
    }
    VideoFrame {
        pts: Flicks::ZERO,
        duration: Rate::FPS_30.frame_duration(),
        width: side,
        height: side,
        format: PixelFormat::Rgba,
        planes: vec![Plane {
            stride: (side * 4) as usize,
            rows: side as usize,
            data: Arc::from(data),
        }],
        color: bt709_limited(),
    }
}

pub fn frames() -> Frames {
    Frames::default()
        .with(FLOOR, planks(640, 360))
        .with(LOGO, logo(128))
        .with(CORAL, solid(64, 36, [120, 110, 190]))
        .with(TEAL, solid(64, 36, [130, 150, 80]))
        .with(
            BACKDROP,
            nv12(320, 180, bt709_limited(), |_, y| {
                [(26 + y / 8) as u8, 136, 122]
            }),
        )
}

/// Frames and loaded models.
pub struct Provider {
    pub frames: Frames,
    pub models: HashMap<Id, Arc<ModelData>>,
}

impl Provider {
    pub fn new() -> Provider {
        Provider {
            frames: frames(),
            models: HashMap::new(),
        }
    }

    pub fn with_house(mut self, model: ModelData) -> Provider {
        self.models.insert(asset(HOUSE), Arc::new(model));
        self
    }
}

impl FrameProvider for Provider {
    fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError> {
        self.frames.frame(source)
    }

    fn model(&self, asset: &Id) -> Option<Arc<ModelData>> {
        self.models.get(asset).cloned()
    }
}

/// An empty project on the canvas with the pictures as image assets and the house as a
/// model asset.
pub fn project() -> (Project, Id) {
    let format = TimelineFormat {
        width: CANVAS.0,
        height: CANVAS.1,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 4), "Objects", format);
    for n in [FLOOR, LOGO, CORAL, TEAL, BACKDROP] {
        project.media.assets.push(Asset {
            folder: None,
            id: asset(n),
            kind: AssetKind::Image,
            name: format!("picture-{n}.png"),
            path: PathHints::default(),
            identity: None,
            info: MediaInfo {
                video: Some(VideoInfo {
                    width: 640,
                    height: 360,
                    rate: None,
                    codec: None,
                    rotation: 0,
                }),
                ..Default::default()
            },
        });
    }
    project.media.assets.push(Asset {
        folder: None,
        id: asset(HOUSE),
        kind: AssetKind::Model,
        name: "house.glb".into(),
        path: PathHints::default(),
        identity: None,
        info: MediaInfo {
            model: Some(ModelInfo {
                meshes: 2,
                triangles: 18,
                materials: 2,
                textures: 0,
                size: [1.1, 1.6, 1.1],
                generator: None,
                copyright: None,
            }),
            ..Default::default()
        },
    });
    project.manifest.next_id = 100;
    let timeline = project.manifest.primary_timeline.clone();
    (project, timeline)
}

pub const LENGTH: i64 = 60;

/// A clip of `content` lasting two seconds from 0.
pub fn clip(id: u64, content: ClipContent, transform: Transform) -> Clip {
    Clip {
        transform,
        ..Clip::new(
            Id::new(IdKind::Clip, id),
            Flicks::ZERO,
            Rate::FPS_30.frames_to_flicks(LENGTH),
            content,
        )
    }
}

pub fn picture(id: u64, n: u64, transform: Transform) -> Clip {
    clip(id, ClipContent::Image { asset: asset(n) }, transform)
}

pub fn object(id: u64, content: ModelContent, transform: Transform) -> Clip {
    clip(
        id,
        ClipContent::Model {
            model: Box::new(content),
        },
        transform,
    )
}

pub fn camera(id: u64, content: CameraContent) -> Clip {
    clip(
        id,
        ClipContent::Camera(Box::new(content)),
        Transform::default(),
    )
}

pub fn content(json: serde_json::Value) -> ModelContent {
    serde_json::from_value(json).unwrap()
}

/// Puts each clip on a video track of its own, bottom first.
pub fn with_tracks(project: &mut Project, timeline: &Id, clips: Vec<Clip>) {
    let doc = project.timelines.get_mut(timeline).unwrap();
    for (i, clip) in clips.into_iter().enumerate() {
        doc.tracks.push(Track {
            id: Id::new(IdKind::Track, 20 + i as u64),
            kind: TrackKind::Video,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![clip],
        });
    }
}

/// An orange cardboard box with a logo on its front, standing on a floor that only shows
/// its soft shadow over a photo of planks, seen from a little above.
pub fn shipping_box() -> (Project, Id) {
    let (mut project, timeline) = project();
    let model = content(serde_json::json!({
        "object": {"kind": "box", "size": [0.3, 0.21, 0.24], "bevel": 0.006},
        "material": {"color": "#C9792F", "roughness": 0.9},
        "faces": {"front": {"color": "#C9792F", "roughness": 0.9, "texture": "asset_0002", "texture_size": 0.55}},
        "light": {"ambient": 0.45, "ground": "#7A6A58", "azimuth": -40, "elevation": 55, "intensity": 1.1, "softness": 0.55},
        "floor": {"shadow": 0.55}
    }));
    let clips = vec![
        picture(30, FLOOR, Transform::default()),
        object(
            31,
            model,
            Transform {
                position: [0.0, 0.1],
                rotation_y: -28.0,
                ..Transform::default()
            },
        ),
        camera(
            32,
            CameraContent {
                orbit: [0.0, 22.0],
                point_of_interest: [0.0, 0.1],
                ..CameraContent::default()
            },
        ),
    ];
    with_tracks(&mut project, &timeline, clips);
    (project, timeline)
}

/// A dark city of blocks on a dark ground, seen from above by a camera tilted down and
/// turned.
pub fn city() -> (Project, Id) {
    let (mut project, timeline) = project();
    let model = content(serde_json::json!({
        "object": {"kind": "blocks", "columns": 16, "rows": 16, "block": 0.045, "gap": 0.018, "heights": [0.01, 0.18], "fill": 0.85, "seed": 3},
        "material": {"color": "#3A435E", "roughness": 0.7},
        "faces": {"top": {"color": "#5C688C", "roughness": 0.55}},
        "light": {"ambient": 0.55, "sky": "#8EA2D6", "ground": "#20242F", "azimuth": 35, "elevation": 40, "color": "#C9D6FF", "intensity": 1.0, "softness": 0.3},
        "floor": {"catcher": false, "color": "#1E2230", "size": 3}
    }));
    let clips = vec![
        picture(30, BACKDROP, Transform::default()),
        object(
            31,
            model,
            Transform {
                position_z: 0.3,
                ..Transform::default()
            },
        ),
        camera(
            32,
            CameraContent {
                orbit: [25.0, 42.0],
                position_z: -0.5,
                point_of_interest_z: 0.3,
                ..CameraContent::default()
            },
        ),
    ];
    with_tracks(&mut project, &timeline, clips);
    (project, timeline)
}

/// Bold letters with depth and bevelled edges, a darker colour on their sides, turned
/// towards the light.
pub fn extruded_title() -> (Project, Id) {
    let (mut project, timeline) = project();
    let model = content(serde_json::json!({
        "object": {"kind": "text", "text": "FOSS", "bold": true, "size": 0.16, "depth": 0.05, "bevel": 0.006},
        "material": {"color": "#E2A33B", "roughness": 0.35, "metallic": 0.6},
        "faces": {"front": {"color": "#F4F1EA", "roughness": 0.3}},
        "light": {"ambient": 0.4, "azimuth": -30, "elevation": 40, "intensity": 1.1, "softness": 0.3}
    }));
    let clips = vec![
        picture(30, BACKDROP, Transform::default()),
        object(
            31,
            model,
            Transform {
                rotation_y: -24.0,
                rotation_x: 12.0,
                ..Transform::default()
            },
        ),
    ];
    with_tracks(&mut project, &timeline, clips);
    (project, timeline)
}

/// A coral card in front, a teal card behind, and a ring between them on the top track:
/// the ring sorts between the cards by depth.
pub fn between_cards() -> (Project, Id) {
    let (mut project, timeline) = project();
    let card = |x: f64, z: f64| Transform {
        three_d: true,
        scale: 0.26,
        position: [x, 0.0],
        position_z: z,
        rotation_y: -15.0,
        ..Transform::default()
    };
    let ring = content(serde_json::json!({
        "object": {"kind": "torus", "size": 0.32, "thickness": 0.07},
        "material": {"color": "#D8D8D8", "metallic": 1, "roughness": 0.25},
        "light": {"ambient": 0.7, "sky": "#E6EEFF", "ground": "#40362E"}
    }));
    let clips = vec![
        picture(30, BACKDROP, Transform::default()),
        picture(31, CORAL, card(-0.2, -0.1)),
        picture(32, TEAL, card(0.22, 0.5)),
        object(
            33,
            ring,
            Transform {
                position_z: 0.2,
                rotation_x: 20.0,
                ..Transform::default()
            },
        ),
    ];
    with_tracks(&mut project, &timeline, clips);
    (project, timeline)
}

/// A sphere far behind a card in focus: the depth of field blurs the sphere and its edges.
pub fn defocused() -> (Project, Id) {
    let (mut project, timeline) = project();
    let sphere = content(serde_json::json!({
        "object": {"kind": "sphere", "size": 0.3},
        "material": {"color": "#3A7BD5", "roughness": 0.3}
    }));
    let clips = vec![
        picture(30, BACKDROP, Transform::default()),
        object(
            31,
            sphere,
            Transform {
                position: [0.18, -0.05],
                position_z: 1.2,
                ..Transform::default()
            },
        ),
        picture(
            32,
            CORAL,
            Transform {
                three_d: true,
                scale: 0.3,
                position: [-0.15, 0.1],
                ..Transform::default()
            },
        ),
        camera(
            33,
            CameraContent {
                depth_of_field: true,
                aperture: 0.3,
                ..CameraContent::default()
            },
        ),
    ];
    with_tracks(&mut project, &timeline, clips);
    (project, timeline)
}

/// The glTF house with its own materials, turned, on a catching floor over the planks.
pub fn house() -> (Project, Id) {
    let (mut project, timeline) = project();
    let model = content(serde_json::json!({
        "object": {"kind": "asset", "asset": "asset_0006", "size": 0.3},
        "light": {"azimuth": -50, "elevation": 45, "softness": 0.4},
        "floor": {}
    }));
    let clips = vec![
        picture(30, FLOOR, Transform::default()),
        object(
            31,
            model,
            Transform {
                position: [0.0, -0.02],
                rotation_y: 35.0,
                ..Transform::default()
            },
        ),
        camera(
            32,
            CameraContent {
                orbit: [0.0, 15.0],
                ..CameraContent::default()
            },
        ),
    ];
    with_tracks(&mut project, &timeline, clips);
    (project, timeline)
}

/// A small PNG: a blue and white checker, for the house's walls.
pub fn checker_png() -> Vec<u8> {
    let side = 32u32;
    let mut rgba = Vec::new();
    for y in 0..side {
        for x in 0..side {
            let on = (x / 8 + y / 8) % 2 == 0;
            rgba.extend_from_slice(if on {
                &[40, 90, 200, 255]
            } else {
                &[240, 240, 240, 255]
            });
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, side, side);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgba)
            .unwrap();
    }
    out
}
