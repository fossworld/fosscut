//! Layers placed in 3D and cameras (ADR-0037): the properties of placement in depth, the
//! camera clip's content, and the 3D animation presets.
//!
//! A camera clip sits on a video track of a timeline. While it covers a time it is the view
//! through which that timeline's 3D layers (`transform.three_d`) are seen; the camera on the
//! highest track wins when several cover the same time. Without one, 3D layers are seen
//! through the default camera, which frames the z = 0 plane exactly like 2D.
//!
//! Units follow the clip transform: x and y as fractions of the canvas width and height
//! from its centre (y grows downwards), depth z as a fraction of the canvas width, growing
//! away from the viewer. The field of view is measured across the canvas width, so a depth
//! changes a picture's size by the same factor on every canvas aspect.

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::animation::{Applies, KeyValue, Property, PropertyKind};
use crate::is_default;
use crate::timeline::Clip;
use crate::transition::AnimationDelta;

/// The default field of view in degrees across the canvas width, close to a 50 mm lens on
/// a full-frame camera.
pub const DEFAULT_FIELD_OF_VIEW: f64 = 40.0;

/// The narrowest and widest field of view in degrees.
pub const FIELD_OF_VIEW_RANGE: (f64, f64) = (1.0, 170.0);

/// The default lens opening of the depth of field, as a fraction of the canvas width.
pub const DEFAULT_APERTURE: f64 = 0.1;

/// The default largest blur radius of the depth of field, as a fraction of the canvas
/// width.
pub const DEFAULT_MAX_BLUR: f64 = 0.02;

/// The largest `max_blur` a camera takes, as a fraction of the canvas width.
pub const MAX_BLUR_LIMIT: f64 = 0.2;

/// A camera: where it is, what it looks at, its lens and its depth of field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CameraContent {
    /// The angle of view across the canvas width, in degrees from 1 to 170 (40 when
    /// omitted). The camera keeps its framing distance (see `position_z`), so a wider
    /// angle strengthens the perspective without changing what the z = 0 plane shows.
    #[serde(
        default = "default_field_of_view",
        skip_serializing_if = "is_default_field_of_view"
    )]
    pub field_of_view: f64,
    /// Where the camera is across the canvas, as fractions of its width and height from
    /// its centre; y grows downwards.
    #[serde(default, skip_serializing_if = "is_default")]
    pub position: [f64; 2],
    /// How far the camera has moved forward from its framing distance (where the z = 0
    /// plane exactly fills the frame), as a fraction of the canvas width: positive pushes
    /// in (a dolly in), negative pulls back.
    #[serde(default, skip_serializing_if = "is_default")]
    pub position_z: f64,
    /// The point the camera looks at, as fractions of the canvas width and height from its
    /// centre.
    #[serde(default, skip_serializing_if = "is_default")]
    pub point_of_interest: [f64; 2],
    /// The depth of the point of interest, as a fraction of the canvas width; positive is
    /// away from the viewer.
    #[serde(default, skip_serializing_if = "is_default")]
    pub point_of_interest_z: f64,
    /// Turns the camera around its point of interest, in degrees: `[yaw, pitch]`, where a
    /// positive yaw moves the camera to the right and a positive pitch moves it up. It
    /// keeps looking at the point of interest.
    #[serde(default, skip_serializing_if = "is_default")]
    pub orbit: [f64; 2],
    /// Turns the view around its axis, in degrees; positive turns the picture clockwise.
    #[serde(default, skip_serializing_if = "is_default")]
    pub roll: f64,
    /// Blurs 3D layers by their distance from the plane in focus.
    #[serde(default, skip_serializing_if = "is_default")]
    pub depth_of_field: bool,
    /// How far the plane in focus lies beyond the point of interest along the view, as a
    /// fraction of the canvas width; 0 focuses on the point of interest, negative nearer
    /// the camera.
    #[serde(default, skip_serializing_if = "is_default")]
    pub focus_offset: f64,
    /// The lens opening of the depth of field, as a fraction of the canvas width, from 0
    /// to 1 (0.1 when omitted): larger blurs layers away from the focus more.
    #[serde(
        default = "default_aperture",
        skip_serializing_if = "is_default_aperture"
    )]
    pub aperture: f64,
    /// The largest blur radius of the depth of field, as a fraction of the canvas width,
    /// from 0 to 0.2 (0.02 when omitted).
    #[serde(
        default = "default_max_blur",
        skip_serializing_if = "is_default_max_blur"
    )]
    pub max_blur: f64,
}

impl Default for CameraContent {
    fn default() -> CameraContent {
        CameraContent {
            field_of_view: DEFAULT_FIELD_OF_VIEW,
            position: [0.0, 0.0],
            position_z: 0.0,
            point_of_interest: [0.0, 0.0],
            point_of_interest_z: 0.0,
            orbit: [0.0, 0.0],
            roll: 0.0,
            depth_of_field: false,
            focus_offset: 0.0,
            aperture: DEFAULT_APERTURE,
            max_blur: DEFAULT_MAX_BLUR,
        }
    }
}

fn default_field_of_view() -> f64 {
    DEFAULT_FIELD_OF_VIEW
}

fn is_default_field_of_view(value: &f64) -> bool {
    *value == DEFAULT_FIELD_OF_VIEW
}

fn default_aperture() -> f64 {
    DEFAULT_APERTURE
}

fn is_default_aperture(value: &f64) -> bool {
    *value == DEFAULT_APERTURE
}

fn default_max_blur() -> f64 {
    DEFAULT_MAX_BLUR
}

fn is_default_max_blur(value: &f64) -> bool {
    *value == DEFAULT_MAX_BLUR
}

/// Problems with a camera's values, as messages; empty when every value is in range.
pub fn camera_issues(camera: &CameraContent) -> Vec<String> {
    let mut issues = Vec::new();
    let (low, high) = FIELD_OF_VIEW_RANGE;
    if !(camera.field_of_view.is_finite() && (low..=high).contains(&camera.field_of_view)) {
        issues.push(format!(
            "field_of_view must be from {low} to {high} degrees, not {}",
            camera.field_of_view
        ));
    }
    let finite = [
        ("position", camera.position.to_vec()),
        ("position_z", vec![camera.position_z]),
        ("point_of_interest", camera.point_of_interest.to_vec()),
        ("point_of_interest_z", vec![camera.point_of_interest_z]),
        ("orbit", camera.orbit.to_vec()),
        ("roll", vec![camera.roll]),
        ("focus_offset", vec![camera.focus_offset]),
    ];
    for (name, values) in finite {
        if !values.iter().all(|v| v.is_finite()) {
            issues.push(format!("{name} must be finite"));
        }
    }
    if !(camera.aperture.is_finite() && (0.0..=1.0).contains(&camera.aperture)) {
        issues.push(format!(
            "aperture must be from 0 to 1, not {}",
            camera.aperture
        ));
    }
    if !(camera.max_blur.is_finite() && (0.0..=MAX_BLUR_LIMIT).contains(&camera.max_blur)) {
        issues.push(format!(
            "max_blur must be from 0 to {MAX_BLUR_LIMIT}, not {}",
            camera.max_blur
        ));
    }
    issues
}

/// The animatable numbers of a camera: property path and whether it holds a pair.
pub const CAMERA_ANIMATABLE: &[(&str, bool)] = &[
    ("camera.field_of_view", false),
    ("camera.position", true),
    ("camera.position_z", false),
    ("camera.point_of_interest", true),
    ("camera.point_of_interest_z", false),
    ("camera.orbit", true),
    ("camera.roll", false),
    ("camera.focus_offset", false),
    ("camera.aperture", false),
    ("camera.max_blur", false),
];

/// The properties of placement in 3D and of cameras (ADR-0037), appended to the closed
/// vocabulary of [`crate::clip_properties`].
pub(crate) fn spatial_properties() -> Vec<Property> {
    use Applies::{Camera, Picture};
    use PropertyKind::{Bool, Number, Pair};
    let property = |path, kind, applies, range: (Option<f64>, Option<f64>), meaning| Property {
        path,
        kind,
        applies,
        animatable: kind != Bool,
        min: range.0,
        max: range.1,
        choices: &[],
        meaning,
    };
    let any = (None, None);
    vec![
        property(
            "transform.three_d",
            Bool,
            Picture,
            any,
            "true places the clip in the timeline's 3D scene: seen through its camera, sorted by depth with the 3D clips on neighbouring tracks, blurred by depth of field",
        ),
        property(
            "transform.position_z",
            Number,
            Picture,
            any,
            "depth as a fraction of the canvas width; positive is away from the viewer, so the picture looks smaller",
        ),
        property(
            "transform.rotation_x",
            Number,
            Picture,
            any,
            "degrees the picture tilts around its horizontal axis; positive tips its top edge away from the viewer",
        ),
        property(
            "transform.rotation_y",
            Number,
            Picture,
            any,
            "degrees the picture turns around its vertical axis; positive turns its right edge away from the viewer",
        ),
        property(
            "transform.anchor",
            Pair,
            Picture,
            any,
            "[x, y]: the pivot that rotation and scale turn around, as fractions of the picture's width and height from its centre; [-0.5, 0] is the middle of the left edge; moving it does not move the picture",
        ),
        property(
            "camera.field_of_view",
            Number,
            Camera,
            (Some(FIELD_OF_VIEW_RANGE.0), Some(FIELD_OF_VIEW_RANGE.1)),
            "degrees across the canvas width (40 by default); the camera keeps the z = 0 plane framed, so a wider angle strengthens the perspective",
        ),
        property(
            "camera.position",
            Pair,
            Camera,
            any,
            "[x, y]: where the camera is, as fractions of the canvas width and height from its centre",
        ),
        property(
            "camera.position_z",
            Number,
            Camera,
            any,
            "how far the camera has moved forward from where the z = 0 plane fills the frame, as a fraction of the canvas width; positive pushes in",
        ),
        property(
            "camera.point_of_interest",
            Pair,
            Camera,
            any,
            "[x, y]: what the camera looks at, as fractions of the canvas width and height from its centre",
        ),
        property(
            "camera.point_of_interest_z",
            Number,
            Camera,
            any,
            "depth of the point of interest as a fraction of the canvas width",
        ),
        property(
            "camera.orbit",
            Pair,
            Camera,
            any,
            "[yaw, pitch] degrees the camera turns around its point of interest: positive yaw moves it right, positive pitch up",
        ),
        property(
            "camera.roll",
            Number,
            Camera,
            any,
            "degrees the view turns around its axis; positive turns the picture clockwise",
        ),
        property(
            "camera.depth_of_field",
            Bool,
            Camera,
            any,
            "true blurs 3D clips by their distance from the plane in focus",
        ),
        property(
            "camera.focus_offset",
            Number,
            Camera,
            any,
            "how far the plane in focus lies beyond the point of interest, as a fraction of the canvas width; 0 focuses on the point of interest",
        ),
        property(
            "camera.aperture",
            Number,
            Camera,
            (Some(0.0), Some(1.0)),
            "lens opening as a fraction of the canvas width (0.1 by default); larger blurs out-of-focus clips more",
        ),
        property(
            "camera.max_blur",
            Number,
            Camera,
            (Some(0.0), Some(MAX_BLUR_LIMIT)),
            "largest blur radius as a fraction of the canvas width (0.02 by default)",
        ),
    ]
}

impl CameraContent {
    /// The plain value of the animatable property at `path` (`camera.…`).
    pub fn value(&self, path: &str) -> Option<KeyValue> {
        Some(match path {
            "camera.field_of_view" => KeyValue::Number(self.field_of_view),
            "camera.position" => KeyValue::Pair(self.position),
            "camera.position_z" => KeyValue::Number(self.position_z),
            "camera.point_of_interest" => KeyValue::Pair(self.point_of_interest),
            "camera.point_of_interest_z" => KeyValue::Number(self.point_of_interest_z),
            "camera.orbit" => KeyValue::Pair(self.orbit),
            "camera.roll" => KeyValue::Number(self.roll),
            "camera.focus_offset" => KeyValue::Number(self.focus_offset),
            "camera.aperture" => KeyValue::Number(self.aperture),
            "camera.max_blur" => KeyValue::Number(self.max_blur),
            _ => return None,
        })
    }

    /// Sets the animatable property at `path` to `value`; false when `path` is not a
    /// camera number or `value` has the other shape.
    pub fn set_value(&mut self, path: &str, value: KeyValue) -> bool {
        match (path, value) {
            ("camera.field_of_view", KeyValue::Number(v)) => self.field_of_view = v,
            ("camera.position", KeyValue::Pair(v)) => self.position = v,
            ("camera.position_z", KeyValue::Number(v)) => self.position_z = v,
            ("camera.point_of_interest", KeyValue::Pair(v)) => self.point_of_interest = v,
            ("camera.point_of_interest_z", KeyValue::Number(v)) => self.point_of_interest_z = v,
            ("camera.orbit", KeyValue::Pair(v)) => self.orbit = v,
            ("camera.roll", KeyValue::Number(v)) => self.roll = v,
            ("camera.focus_offset", KeyValue::Number(v)) => self.focus_offset = v,
            ("camera.aperture", KeyValue::Number(v)) => self.aperture = v,
            ("camera.max_blur", KeyValue::Number(v)) => self.max_blur = v,
            _ => return false,
        }
        true
    }
}

/// A 3D entrance at `t` from 0 (start) to 1 (in place), or `None` when `preset` is not a
/// 3D preset; exits play it backwards. Every curve reaches rest exactly at 1.
pub(crate) fn entrance_3d(preset: &str, t: f64) -> Option<AnimationDelta> {
    let t = t.clamp(0.0, 1.0);
    let away = (1.0 - t).powi(3);
    let mut d = AnimationDelta::default();
    match preset {
        "flip_in" | "flip_out" => {
            d.rotation_y = -90.0 * away;
            d.opacity = (t * 4.0).min(1.0);
        }
        "swing_in" | "swing_out" => {
            // A damped swing that passes facing the viewer twice and stops there.
            let swing = (1.0 - t) * (-3.0 * t).exp() * (3.0 * std::f64::consts::PI * t).cos();
            d.rotation_y = 90.0 * swing;
            d.opacity = (t * 5.0).min(1.0);
        }
        "tilt_in" | "tilt_out" => {
            d.rotation_x = 75.0 * away;
            d.offset[1] = 0.1 * away;
            d.opacity = (t * 2.0).min(1.0);
        }
        "fly_in_3d" | "fly_out_3d" => {
            d.depth = 2.5 * away;
            d.rotation_y = 35.0 * away;
            d.opacity = (t * 3.0).min(1.0);
        }
        _ => return None,
    }
    Some(d)
}

/// A 3D combo at phase `p` (cycles), or `None` when `preset` is not a 3D combo.
pub(crate) fn combo_3d(preset: &str, p: f64) -> Option<AnimationDelta> {
    let mut d = AnimationDelta::default();
    match preset {
        "turntable" => d.rotation_y = 360.0 * p.rem_euclid(1.0),
        "sway_3d" => d.rotation_y = 12.0 * (p * std::f64::consts::TAU).sin(),
        _ => return None,
    }
    Some(d)
}

impl Clip {
    /// A camera clip's camera at `offset` from its start, with its keyframes applied;
    /// `None` for other clips.
    pub fn camera_at(&self, offset: Flicks) -> Option<CameraContent> {
        let crate::ClipContent::Camera(camera) = &self.content else {
            return None;
        };
        let mut camera = (**camera).clone();
        for (path, _) in CAMERA_ANIMATABLE {
            if let Some(value) = self.keyed(path, offset) {
                camera.set_value(path, value);
            }
        }
        Some(camera)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClipContent, Id, IdKind, Interpolation, Keyframe};

    #[test]
    fn cameras_round_trip_sparsely_and_check_their_ranges() {
        let camera = CameraContent::default();
        assert_eq!(
            serde_json::to_value(&camera).unwrap(),
            serde_json::json!({})
        );
        let moved = CameraContent {
            field_of_view: 55.0,
            position_z: 0.3,
            orbit: [20.0, -5.0],
            depth_of_field: true,
            ..CameraContent::default()
        };
        let json = serde_json::to_value(&moved).unwrap();
        assert_eq!(json["field_of_view"], 55.0);
        assert!(json.get("aperture").is_none(), "defaults are omitted");
        let back: CameraContent = serde_json::from_value(json).unwrap();
        assert_eq!(back, moved);
        assert!(serde_json::from_str::<CameraContent>(r#"{"zoom": 2}"#).is_err());
        assert!(camera_issues(&moved).is_empty());
        let wrong = CameraContent {
            field_of_view: 0.0,
            roll: f64::NAN,
            aperture: 2.0,
            max_blur: 0.5,
            ..CameraContent::default()
        };
        let issues = camera_issues(&wrong);
        assert_eq!(issues.len(), 4, "{issues:?}");
        assert!(issues[0].contains("field_of_view"));
    }

    #[test]
    fn a_camera_clip_reads_its_keyframes() {
        let mut clip = Clip::new(
            Id::new(IdKind::Clip, 1),
            Flicks::ZERO,
            Flicks::from_seconds(2).unwrap(),
            ClipContent::Camera(Box::default()),
        );
        clip.keyframes.insert(
            "camera.position_z".into(),
            vec![
                Keyframe {
                    time: Flicks::ZERO,
                    value: KeyValue::Number(0.0),
                    interpolation: Interpolation::Linear,
                },
                Keyframe {
                    time: Flicks::from_seconds(2).unwrap(),
                    value: KeyValue::Number(0.4),
                    interpolation: Interpolation::Linear,
                },
            ],
        );
        let middle = clip.camera_at(Flicks::from_seconds(1).unwrap()).unwrap();
        assert!((middle.position_z - 0.2).abs() < 1e-9);
        assert_eq!(middle.field_of_view, DEFAULT_FIELD_OF_VIEW);
        let mut camera = CameraContent::default();
        assert!(camera.set_value("camera.orbit", KeyValue::Pair([10.0, 0.0])));
        assert!(!camera.set_value("camera.orbit", KeyValue::Number(1.0)));
        assert_eq!(
            camera.value("camera.orbit"),
            Some(KeyValue::Pair([10.0, 0.0]))
        );
        assert_eq!(camera.value("camera.depth_of_field"), None);
        for (path, pair) in CAMERA_ANIMATABLE {
            let value = camera.value(path).unwrap();
            assert_eq!(matches!(value, KeyValue::Pair(_)), *pair, "{path}");
        }
        let video = Clip::new(
            Id::new(IdKind::Clip, 2),
            Flicks::ZERO,
            Flicks::from_seconds(1).unwrap(),
            ClipContent::Adjustment,
        );
        assert!(video.camera_at(Flicks::ZERO).is_none());
    }

    #[test]
    fn three_d_presets_move_in_depth_and_end_at_rest() {
        let presets = ["flip_in", "swing_in", "tilt_in", "fly_in_3d"];
        for name in presets {
            let start = entrance_3d(name, 0.0).unwrap();
            assert!(
                start.rotation_x != 0.0 || start.rotation_y != 0.0 || start.depth != 0.0,
                "{name} starts turned or deep"
            );
            assert_eq!(
                entrance_3d(name, 1.0).unwrap(),
                AnimationDelta::default(),
                "{name} ends exactly at rest"
            );
            let out = name.replace("_in", "_out");
            assert_eq!(entrance_3d(&out, 0.3), entrance_3d(name, 0.3), "{out}");
            assert!(
                crate::ANIMATION_PRESETS
                    .iter()
                    .any(|(n, places, _)| *n == name && places.contains("in"))
            );
            assert!(
                crate::ANIMATION_PRESETS
                    .iter()
                    .any(|(n, places, _)| *n == out && places.contains("out"))
            );
        }
        assert!(entrance_3d("fade", 0.5).is_none());
        // The swing passes facing the viewer and overshoots before it settles.
        let lowest = (0..100)
            .map(|i| {
                entrance_3d("swing_in", f64::from(i) / 100.0)
                    .unwrap()
                    .rotation_y
            })
            .fold(f64::INFINITY, f64::min);
        assert!(lowest < -10.0, "{lowest}");
        let quarter = combo_3d("turntable", 1.25).unwrap();
        assert!((quarter.rotation_y - 90.0).abs() < 1e-9);
        assert!(combo_3d("sway_3d", 0.25).unwrap().rotation_y > 11.0);
        assert!(combo_3d("pulse", 0.25).is_none());
        let properties = crate::clip_properties();
        for path in [
            "transform.three_d",
            "transform.position_z",
            "transform.rotation_x",
            "transform.rotation_y",
            "transform.anchor",
            "camera.depth_of_field",
        ] {
            assert!(properties.iter().any(|p| p.path == path), "{path}");
        }
        for (path, pair) in CAMERA_ANIMATABLE {
            let property = crate::clip_property(path).unwrap();
            assert!(property.animatable, "{path}");
            assert_eq!(property.applies, Applies::Camera);
            assert_eq!(property.kind == PropertyKind::Pair, *pair, "{path}");
        }
        assert!(
            !crate::clip_property("transform.three_d")
                .unwrap()
                .animatable
        );
    }
}
