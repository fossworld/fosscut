//! Transitions between clips and clip animations (ADR-0011, items 1 and 4).
//!
//! A transition belongs to the clip it leaves: it plays over the cut to the next clip on
//! the same track, centred on the cut, while both clips show. Outside the clips' ends the
//! media beyond them is used, or the last and first frames hold.
//!
//! Animations move a clip in, out, or all the time it shows (combo), from presets written
//! for FossCut; they change placement and opacity only, so they stack with keyframes.

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::effect::{ParamDefault, ParamKind, ParamSpec, ParamValue};
use crate::is_default;
use std::collections::BTreeMap;

/// A transition from a clip to the next one on its track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Transition {
    /// The transition type, such as `dissolve` or `wipe`.
    #[serde(rename = "type")]
    pub transition_type: String,
    /// How long it lasts, centred on the cut.
    pub duration: Flicks,
    /// Parameters that differ from their defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamValue>,
}

/// A built-in transition type.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TransitionSpec {
    /// Its name, such as `push`; `type` in JSON.
    #[serde(rename = "type")]
    pub name: &'static str,
    /// Display name.
    pub title: &'static str,
    /// What it looks like.
    pub summary: &'static str,
    /// Its parameters.
    pub params: Vec<ParamSpec>,
}

impl TransitionSpec {
    /// The parameter called `name`.
    pub fn param(&self, name: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.name == name)
    }
}

const DIRECTIONS: &[&str] = &["left", "right", "up", "down"];

fn direction() -> ParamSpec {
    ParamSpec {
        name: "direction",
        kind: ParamKind::Choice,
        default: ParamDefault::Name("left"),
        min: None,
        max: None,
        unit: "",
        choices: DIRECTIONS,
        animatable: false,
        meaning: "where the movement goes: left, right, up or down",
    }
}

fn softness() -> ParamSpec {
    ParamSpec {
        name: "softness",
        kind: ParamKind::Number,
        default: ParamDefault::Number(0.1),
        min: Some(0.0),
        max: Some(1.0),
        unit: "",
        choices: &[],
        animatable: false,
        meaning: "how soft the moving edge is",
    }
}

/// Every built-in transition type.
pub fn transition_types() -> Vec<TransitionSpec> {
    let spec = |name, title, summary, params| TransitionSpec {
        name,
        title,
        summary,
        params,
    };
    vec![
        spec(
            "dissolve",
            "Dissolve",
            "The first clip fades into the second.",
            vec![],
        ),
        spec(
            "dip",
            "Dip to color",
            "Fades through a color, black by default.",
            vec![ParamSpec {
                name: "color",
                kind: ParamKind::Color,
                default: ParamDefault::Name("#000000"),
                min: None,
                max: None,
                unit: "",
                choices: &[],
                animatable: false,
                meaning: "the color it fades through",
            }],
        ),
        spec(
            "wipe",
            "Wipe",
            "An edge sweeps across, revealing the second clip.",
            vec![direction(), softness()],
        ),
        spec(
            "slide",
            "Slide",
            "The second clip slides in over the first.",
            vec![direction()],
        ),
        spec(
            "push",
            "Push",
            "The second clip pushes the first out of the frame.",
            vec![direction()],
        ),
        spec(
            "zoom",
            "Zoom",
            "The first clip zooms in and fades into the second.",
            vec![],
        ),
        spec(
            "blur",
            "Blur",
            "The first clip blurs into the second.",
            vec![],
        ),
        spec(
            "iris",
            "Iris",
            "A circle opens from the middle, revealing the second clip.",
            vec![softness()],
        ),
        spec(
            "clock",
            "Clock wipe",
            "A hand sweeps around the middle, revealing the second clip.",
            vec![softness()],
        ),
        spec(
            "spin",
            "Spin",
            "The first clip turns and shrinks away as the second turns in.",
            vec![],
        ),
    ]
}

/// The built-in transition called `name`.
pub fn transition_type(name: &str) -> Option<TransitionSpec> {
    transition_types().into_iter().find(|t| t.name == name)
}

/// One of a clip's animations: a preset and how long it takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Animation {
    /// The preset, such as `fade` or `slide_left`.
    pub preset: String,
    /// How long it takes; combo animations repeat with this period.
    pub duration: Flicks,
    /// How long it waits (0 when omitted): an entrance starts this long after the clip's
    /// start and holds its first state until then, an exit ends this long before the
    /// clip's end and holds its last state after, and a combo starts its cycles this late.
    /// Staggered animations give each clip a growing delay.
    #[serde(default, skip_serializing_if = "is_default")]
    pub delay: Flicks,
}

/// How a clip moves in, out and while it shows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClipAnimations {
    /// Plays from the clip's start.
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub entrance: Option<Animation>,
    /// Plays up to the clip's end.
    #[serde(default, rename = "out", skip_serializing_if = "Option::is_none")]
    pub exit: Option<Animation>,
    /// Repeats for as long as the clip shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combo: Option<Animation>,
}

impl ClipAnimations {
    /// True when the clip has no animation.
    pub fn is_empty(&self) -> bool {
        is_default(self)
    }
}

/// Built-in animations: name, where they apply (`in`, `out` or `combo`) and what they do.
pub const ANIMATION_PRESETS: &[(&str, &str, &str)] = &[
    ("fade", "in or out", "fades from or to transparent"),
    (
        "slide_left",
        "in or out",
        "slides in from the right towards the left, or out to the left",
    ),
    (
        "slide_right",
        "in or out",
        "slides in from the left towards the right, or out to the right",
    ),
    (
        "slide_up",
        "in or out",
        "slides in from below, or out upwards",
    ),
    (
        "slide_down",
        "in or out",
        "slides in from above, or out downwards",
    ),
    ("zoom_in", "in or out", "grows from small, or shrinks away"),
    ("zoom_out", "in or out", "shrinks from large, or grows away"),
    ("spin", "in or out", "turns a full circle while fading"),
    (
        "pop",
        "in or out",
        "springs in past full size and settles, or the reverse",
    ),
    ("drop", "in or out", "falls in from above with a bounce"),
    ("pulse", "combo", "breathes larger and smaller"),
    ("swing", "combo", "rocks from side to side"),
    ("float", "combo", "drifts up and down"),
    ("shake", "combo", "shakes quickly"),
    (
        "blur_in",
        "in",
        "comes into focus from a soft blur while fading in",
    ),
    ("blur_out", "out", "blurs away while fading out"),
    (
        "pop_in",
        "in",
        "grows from 60% past full size and settles, quickly opaque",
    ),
    (
        "pop_out",
        "out",
        "swells slightly, then shrinks away to 60% while fading",
    ),
    (
        "rise_in",
        "in",
        "rises a short way into place from below while fading in",
    ),
    ("rise_out", "out", "sinks a short way while fading out"),
    (
        "swoop_in",
        "in",
        "sweeps in on a curve from the lower right, turning upright and growing to size",
    ),
    (
        "swoop_out",
        "out",
        "sweeps away on a curve to the lower right, turning and shrinking",
    ),
    (
        "glow_in",
        "in",
        "fades in with a pulse of light that flares and settles (light.glow)",
    ),
    (
        "glow_out",
        "out",
        "flares with light while fading out (light.glow)",
    ),
    (
        "flip_in",
        "in",
        "turns over around its vertical axis from edge-on to face the viewer (3D)",
    ),
    (
        "flip_out",
        "out",
        "turns away around its vertical axis until edge-on (3D)",
    ),
    (
        "swing_in",
        "in",
        "swings in around its vertical axis like a door, past facing the viewer, and settles (3D)",
    ),
    (
        "swing_out",
        "out",
        "swings away around its vertical axis like a door (3D)",
    ),
    (
        "tilt_in",
        "in",
        "tips up from lying back around its horizontal axis while rising and fading in (3D)",
    ),
    (
        "tilt_out",
        "out",
        "tips back around its horizontal axis while sinking and fading out (3D)",
    ),
    (
        "fly_in_3d",
        "in",
        "flies in from deep behind the canvas while turning to face the viewer (3D)",
    ),
    (
        "fly_out_3d",
        "out",
        "flies away into the distance while turning (3D)",
    ),
    (
        "turntable",
        "combo",
        "turns around its vertical axis, a full turn per cycle (3D)",
    ),
    (
        "sway_3d",
        "combo",
        "rocks gently around its vertical axis (3D)",
    ),
    (
        "draw_on",
        "in",
        "draws a shape clip's outline on from its start to its end, by its trim (ADR-0036)",
    ),
    (
        "draw_off",
        "out",
        "erases a shape clip's outline from its start to its end, by its trim (ADR-0036)",
    ),
];

/// The change an animation makes to a clip's placement at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationDelta {
    /// Added to the position, as fractions of the canvas.
    pub offset: [f64; 2],
    /// Multiplies the scale.
    pub scale: f64,
    /// Added to the rotation, in degrees.
    pub rotation: f64,
    /// Multiplies the opacity.
    pub opacity: f64,
    /// Added to the depth (`transform.position_z`), as a fraction of the canvas width
    /// (ADR-0037).
    pub depth: f64,
    /// Added to the tilt around the horizontal axis (`transform.rotation_x`), in degrees.
    pub rotation_x: f64,
    /// Added to the turn around the vertical axis (`transform.rotation_y`), in degrees.
    pub rotation_y: f64,
    /// A blur over the picture (one standard deviation) as a fraction of the canvas height,
    /// measured at the clip's own size so it scales with it; 0 for none. The renderer plans
    /// it as a blur that spreads past the picture's edges.
    pub blur: f64,
    /// Light around the picture, 0 (none) to 1 (a strong flare); the renderer plans it as a
    /// `light.glow` effect.
    pub glow: f64,
    /// The part of a shape clip's trimmed outline that shows, `[from, to]` as fractions of
    /// it: `[0, 1]` shows all of it (ADR-0036). Combined animations keep what both show.
    pub draw: [f64; 2],
}

impl Default for AnimationDelta {
    fn default() -> AnimationDelta {
        AnimationDelta {
            offset: [0.0, 0.0],
            scale: 1.0,
            rotation: 0.0,
            opacity: 1.0,
            depth: 0.0,
            rotation_x: 0.0,
            rotation_y: 0.0,
            blur: 0.0,
            glow: 0.0,
            draw: [0.0, 1.0],
        }
    }
}

impl AnimationDelta {
    fn then(self, other: AnimationDelta) -> AnimationDelta {
        AnimationDelta {
            offset: [
                self.offset[0] + other.offset[0],
                self.offset[1] + other.offset[1],
            ],
            scale: self.scale * other.scale,
            rotation: self.rotation + other.rotation,
            opacity: self.opacity * other.opacity,
            depth: self.depth + other.depth,
            rotation_x: self.rotation_x + other.rotation_x,
            rotation_y: self.rotation_y + other.rotation_y,
            blur: self.blur + other.blur,
            glow: self.glow + other.glow,
            draw: [
                self.draw[0].max(other.draw[0]),
                self.draw[1].min(other.draw[1]),
            ],
        }
    }
}

fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

/// An entrance at `t` from 0 (start) to 1 (fully in); exits use `1 - t`.
fn entrance(preset: &str, t: f64) -> AnimationDelta {
    if let Some(d) = crate::camera::entrance_3d(preset, t) {
        return d;
    }
    let t = t.clamp(0.0, 1.0);
    let e = ease_out(t);
    let away = 1.0 - e;
    let mut d = AnimationDelta::default();
    match preset {
        "fade" => d.opacity = t,
        "slide_left" => d.offset[0] = away,
        "slide_right" => d.offset[0] = -away,
        "slide_up" => d.offset[1] = away,
        "slide_down" => d.offset[1] = -away,
        "zoom_in" => {
            d.scale = 0.3 + 0.7 * e;
            d.opacity = t.min(1.0);
        }
        "zoom_out" => {
            d.scale = 1.0 + 0.8 * away;
            d.opacity = t;
        }
        "spin" => {
            d.rotation = -360.0 * away;
            d.scale = 0.5 + 0.5 * e;
            d.opacity = t;
        }
        "pop" => {
            // Overshoots to 110% and settles: a damped spring.
            let spring = 1.0 - (-6.0 * t).exp() * (8.0 * t).cos();
            d.scale = spring.max(0.0);
            d.opacity = (t * 3.0).min(1.0);
        }
        "drop" => {
            let bounce = (-5.0 * t).exp() * (10.0 * t).cos().abs();
            d.offset[1] = -0.6 * bounce;
            d.opacity = (t * 4.0).min(1.0);
        }
        "blur_in" | "blur_out" => {
            d.blur = 0.03 * (1.0 - t).powi(2);
            d.scale = 1.0 + 0.06 * away;
            d.opacity = (t * 1.6).min(1.0);
        }
        "pop_in" | "pop_out" => {
            d.scale = 0.6 + 0.4 * back_out(t);
            d.opacity = (t * 4.0).min(1.0);
        }
        "rise_in" | "rise_out" => {
            d.offset[1] = 0.08 * away;
            d.opacity = (t * 1.5).min(1.0);
        }
        "swoop_in" | "swoop_out" => {
            d.offset = [0.35 * away, 0.2 * away * away];
            d.rotation = 15.0 * away;
            d.scale = 0.7 + 0.3 * e;
            d.opacity = (t * 2.5).min(1.0);
        }
        "glow_in" | "glow_out" => {
            d.opacity = (t * 2.0).min(1.0);
            d.glow = (4.0 * t * (1.0 - t)).powf(1.5);
            d.scale = 0.97 + 0.03 * e;
        }
        // Shape outlines draw on from their start, and erase from their start on the way
        // out (ADR-0036).
        "draw_on" => d.draw = [0.0, e],
        "draw_off" => d.draw = [away, 1.0],
        _ => {}
    }
    d
}

/// Overshoots past 1 and settles back, from 0 at `t` 0 to 1 at `t` 1: the "back" ease-out
/// of Robert Penner's easing equations, overshooting by about 10%.
fn back_out(t: f64) -> f64 {
    const C1: f64 = 1.70158;
    const C3: f64 = C1 + 1.0;
    let u = t - 1.0;
    1.0 + C3 * u * u * u + C1 * u * u
}

/// A combo animation at phase `p` (cycles, any real number).
fn combo(preset: &str, p: f64) -> AnimationDelta {
    if let Some(d) = crate::camera::combo_3d(preset, p) {
        return d;
    }
    let wave = (p * std::f64::consts::TAU).sin();
    let mut d = AnimationDelta::default();
    match preset {
        "pulse" => d.scale = 1.0 + 0.06 * wave,
        "swing" => d.rotation = 6.0 * wave,
        "float" => d.offset[1] = -0.02 * wave,
        "shake" => {
            let jitter = |k: f64| ((p * 37.0 + k).sin() * 43_758.545_3).rem_euclid(1.0) - 0.5;
            d.offset = [0.02 * jitter(1.0), 0.02 * jitter(7.0)];
        }
        _ => {}
    }
    d
}

/// The change a clip's animations make at `offset` from its start, for a clip
/// `duration` long.
pub fn animate(animations: &ClipAnimations, offset: Flicks, duration: Flicks) -> AnimationDelta {
    let seconds = |f: Flicks| f.as_seconds_f64();
    let mut delta = AnimationDelta::default();
    // Before an entrance's delay it holds its first state; after an exit, its last.
    if let Some(a) = &animations.entrance
        && a.duration.is_positive()
    {
        let into = (offset - a.delay).max(Flicks::ZERO);
        if into < a.duration {
            delta = delta.then(entrance(&a.preset, seconds(into) / seconds(a.duration)));
        }
    }
    if let Some(a) = &animations.exit
        && a.duration.is_positive()
    {
        let left = (duration - offset - a.delay).max(Flicks::ZERO);
        if left < a.duration {
            delta = delta.then(entrance(&a.preset, seconds(left) / seconds(a.duration)));
        }
    }
    if let Some(a) = &animations.combo
        && a.duration.is_positive()
        && offset >= a.delay
    {
        delta = delta.then(combo(
            &a.preset,
            seconds(offset - a.delay) / seconds(a.duration),
        ));
    }
    delta
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(seconds: f64) -> Flicks {
        Flicks::from_seconds_f64(seconds).unwrap()
    }

    fn with(entrance: Option<&str>, exit: Option<&str>, combo: Option<&str>) -> ClipAnimations {
        let a = |preset: &str| Animation {
            preset: preset.into(),
            duration: s(1.0),
            delay: Flicks::ZERO,
        };
        ClipAnimations {
            entrance: entrance.map(a),
            exit: exit.map(a),
            combo: combo.map(a),
        }
    }

    #[test]
    fn entrances_and_exits_reach_rest_and_combos_repeat() {
        let fade = with(Some("fade"), Some("fade"), None);
        assert_eq!(animate(&fade, s(0.0), s(4.0)).opacity, 0.0);
        assert!((animate(&fade, s(0.5), s(4.0)).opacity - 0.5).abs() < 1e-9);
        assert_eq!(
            animate(&fade, s(2.0), s(4.0)),
            AnimationDelta::default(),
            "at rest between"
        );
        assert!((animate(&fade, s(3.75), s(4.0)).opacity - 0.25).abs() < 1e-9);
        let slide = with(Some("slide_left"), None, None);
        assert!(
            (animate(&slide, s(0.0), s(4.0)).offset[0] - 1.0).abs() < 1e-9,
            "starts off to the right"
        );
        assert!(animate(&slide, s(0.99), s(4.0)).offset[0].abs() < 1e-3);
        let pop = with(Some("pop"), None, None);
        let peak = (0..100)
            .map(|i| animate(&pop, s(f64::from(i) / 100.0), s(4.0)).scale)
            .fold(0.0, f64::max);
        assert!(peak > 1.05, "pop overshoots: {peak}");
        let pulse = with(None, None, Some("pulse"));
        let a = animate(&pulse, s(0.25), s(4.0)).scale;
        let b = animate(&pulse, s(1.25), s(4.0)).scale;
        assert!((a - b).abs() < 1e-9 && a > 1.0, "a cycle per second");
        let shake = with(None, None, Some("shake"));
        assert_ne!(
            animate(&shake, s(0.1), s(4.0)).offset,
            animate(&shake, s(0.2), s(4.0)).offset
        );
        for (name, _, _) in ANIMATION_PRESETS {
            let any = ClipAnimations {
                entrance: Some(Animation {
                    preset: name.to_string(),
                    duration: s(1.0),
                    delay: Flicks::ZERO,
                }),
                ..Default::default()
            };
            let rest = animate(&any, s(1.0), s(4.0));
            assert_eq!(rest, AnimationDelta::default(), "{name} ends at rest");
        }
        assert!(ClipAnimations::default().is_empty());
        let json = serde_json::to_value(with(Some("fade"), None, None)).unwrap();
        assert_eq!(json["in"]["preset"], "fade");
        assert!(json.get("out").is_none());
    }

    fn delayed(place: &str, preset: &str, delay: f64) -> ClipAnimations {
        let animation = Some(Animation {
            preset: preset.into(),
            duration: s(0.5),
            delay: s(delay),
        });
        match place {
            "in" => ClipAnimations {
                entrance: animation,
                ..Default::default()
            },
            "out" => ClipAnimations {
                exit: animation,
                ..Default::default()
            },
            _ => ClipAnimations {
                combo: animation,
                ..Default::default()
            },
        }
    }

    #[test]
    fn delays_hold_the_first_and_last_states() {
        let late = delayed("in", "fade", 1.0);
        assert_eq!(
            animate(&late, s(0.0), s(4.0)).opacity,
            0.0,
            "hidden until its turn"
        );
        assert_eq!(animate(&late, s(0.99), s(4.0)).opacity, 0.0);
        assert!((animate(&late, s(1.25), s(4.0)).opacity - 0.5).abs() < 1e-9);
        assert_eq!(animate(&late, s(1.5), s(4.0)), AnimationDelta::default());
        // An exit with a delay ends that long before the clip does, and stays gone.
        let early = delayed("out", "fade", 1.0);
        assert_eq!(animate(&early, s(2.4), s(4.0)), AnimationDelta::default());
        assert!((animate(&early, s(2.75), s(4.0)).opacity - 0.5).abs() < 1e-9);
        assert_eq!(animate(&early, s(3.5), s(4.0)).opacity, 0.0);
        // A combo starts its cycles after the delay, so a stagger makes a wave.
        let wave = delayed("combo", "pulse", 0.25);
        assert_eq!(animate(&wave, s(0.2), s(4.0)), AnimationDelta::default());
        let first = delayed("combo", "pulse", 0.0);
        assert!(
            (animate(&wave, s(0.375), s(4.0)).scale - animate(&first, s(0.125), s(4.0)).scale)
                .abs()
                < 1e-9
        );
        // Delays are sparse in JSON and default to zero.
        let json = serde_json::to_value(&late).unwrap();
        assert_eq!(json["in"]["delay"], serde_json::json!(s(1.0)));
        let plain: Animation =
            serde_json::from_value(serde_json::json!({"preset": "fade", "duration": 1})).unwrap();
        assert_eq!(plain.delay, Flicks::ZERO);
        assert!(serde_json::to_value(&plain).unwrap().get("delay").is_none());
    }

    #[test]
    fn motion_design_presets_move_blur_and_glow() {
        let at = |preset: &str, t: f64| animate(&delayed("in", preset, 0.0), s(t), s(4.0));
        let leaving = |preset: &str, t: f64| animate(&delayed("out", preset, 0.0), s(t), s(4.0));
        // blur_in starts soft and transparent and comes into focus.
        let start = at("blur_in", 0.0);
        assert!(start.blur > 0.025 && start.opacity == 0.0 && start.scale > 1.0);
        assert!(at("blur_in", 0.25).blur < start.blur && at("blur_in", 0.25).blur > 0.0);
        assert!(leaving("blur_out", 3.9).blur > 0.0, "blurs as it leaves");
        // pop_in overshoots and settles.
        let peak = (0..50)
            .map(|i| at("pop_in", f64::from(i) / 100.0).scale)
            .fold(0.0, f64::max);
        assert!(peak > 1.03 && peak < 1.15, "{peak}");
        assert!((at("pop_in", 0.0).scale - 0.6).abs() < 1e-9);
        // rise_in comes up from a little below; swoop_in from the lower right, turned.
        assert!((at("rise_in", 0.0).offset[1] - 0.08).abs() < 1e-9);
        let swoop = at("swoop_in", 0.0);
        assert!(swoop.offset[0] > 0.3 && swoop.offset[1] > 0.1 && swoop.rotation > 10.0);
        assert!(at("swoop_in", 0.25).offset[0] < swoop.offset[0]);
        // glow_in flares in the middle and ends without light.
        assert_eq!(at("glow_in", 0.0).glow, 0.0);
        assert!(
            (at("glow_in", 0.25).glow - 1.0).abs() < 1e-9,
            "the flare peaks halfway"
        );
        assert!(leaving("glow_out", 3.75).glow > 0.9);
        // Every new preset rests at its end and the exits mirror the entrances.
        for name in ["blur", "pop", "rise", "swoop", "glow"] {
            let entrance = format!("{name}_in");
            let exit = format!("{name}_out");
            assert_eq!(at(&entrance, 0.5), AnimationDelta::default(), "{entrance}");
            assert_eq!(leaving(&exit, 3.5), AnimationDelta::default(), "{exit}");
            assert_eq!(
                at(&entrance, 0.1),
                leaving(&exit, 3.9),
                "{exit} mirrors {entrance}"
            );
            let places: Vec<&str> = ANIMATION_PRESETS
                .iter()
                .filter(|(n, ..)| *n == entrance || *n == exit)
                .map(|(_, p, _)| *p)
                .collect();
            assert_eq!(places, ["in", "out"], "{name}");
        }
    }

    #[test]
    fn the_transition_registry_is_consistent() {
        let types = transition_types();
        assert!(types.len() >= 10);
        let mut names: Vec<_> = types.iter().map(|t| t.name).collect();
        names.dedup();
        assert_eq!(names.len(), types.len());
        assert_eq!(
            transition_type("wipe")
                .unwrap()
                .param("direction")
                .unwrap()
                .choices
                .len(),
            4
        );
        assert!(transition_type("morph").is_none());
        let json = serde_json::to_value(transition_type("dip").unwrap()).unwrap();
        assert_eq!(json["type"], "dip");
        assert_eq!(json["params"][0]["default"], "#000000");
    }
}
