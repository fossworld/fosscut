//! Particle clips (ADR-0038): an emitter that gives birth to sprites (dots, spark streaks,
//! stars, rings, confetti pieces or an image) which fly, fall, swirl, change size, colour
//! and opacity, and die. The picture covers the canvas and takes the clip's placement,
//! opacity, blend mode, masks and effects like any other.
//!
//! Every particle is a pure function of its birth index, the seed and the time, so a frame
//! renders the same however it is reached (`fosscut_render::particles` computes it).
//! Distances and speeds are in canvas heights, so motion looks the same on every canvas;
//! the emitter's place and size are fractions of the canvas like a clip's position.

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::animation::{Applies, KeyValue, Property, PropertyKind};
use crate::timeline::Color;
use crate::{AssetKind, Id, Issue, Project, is_default};

/// The most particles born per second.
pub const MAX_PARTICLE_RATE: f64 = 2000.0;
/// The longest a particle lives, in seconds.
pub const MAX_LIFE: f64 = 30.0;
/// The shortest a particle lives, in seconds: shorter lifetimes are lengthened to it.
pub const MIN_LIFE: f64 = 0.05;
/// The most particles one burst gives birth to.
pub const MAX_BURST: u32 = 5000;
/// The most bursts a clip has.
pub const MAX_BURSTS: usize = 64;
/// The most particles drawn at once; beyond it the newest are drawn.
pub const MAX_PARTICLES: usize = 20_000;
/// The most stops of an over-life curve or colour list.
pub const MAX_STOPS: usize = 8;
/// The longest emission before a clip's start.
pub const MAX_PREWARM_SECONDS: i64 = 30;

/// A particle emitter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticleContent {
    /// Where particles are born.
    #[serde(default, skip_serializing_if = "is_default")]
    pub emitter: Emitter,
    /// Particles born per second, 0 to 2000 (animatable as `particles.rate`).
    #[serde(default = "rate", skip_serializing_if = "is_rate")]
    pub rate: f64,
    /// Moments when many particles are born at once, in emission time (from the clip's
    /// start while `offset` is 0).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bursts: Vec<Burst>,
    /// How long particles were already being born before emission time 0, so the clip
    /// opens full instead of empty (at most 30 seconds).
    #[serde(default, skip_serializing_if = "is_default")]
    pub prewarm: Flicks,
    /// Where in its emission the clip starts. 0 for a new clip; splits and trims move it so
    /// both parts continue the same particles.
    #[serde(default, skip_serializing_if = "is_default")]
    pub offset: Flicks,
    /// Seconds a particle lives, from 0.05 to 30 (2 when omitted).
    #[serde(default = "life", skip_serializing_if = "is_life")]
    pub life: f64,
    /// How much a particle's life varies either way, in seconds.
    #[serde(default, skip_serializing_if = "is_default")]
    pub life_variance: f64,
    /// Speed at birth in canvas heights per second (animatable as `particles.speed`).
    #[serde(default = "speed", skip_serializing_if = "is_speed")]
    pub speed: f64,
    /// How much the speed varies either way.
    #[serde(default, skip_serializing_if = "is_default")]
    pub speed_variance: f64,
    /// Direction of travel at birth in degrees clockwise from rightwards: 90 is down, -90
    /// up (animatable as `particles.direction`).
    #[serde(default = "direction", skip_serializing_if = "is_direction")]
    pub direction: f64,
    /// Width of the cone of directions around `direction`, 0 to 360 degrees (animatable
    /// as `particles.spread`).
    #[serde(default = "spread", skip_serializing_if = "is_spread")]
    pub spread: f64,
    /// What the direction is measured from.
    #[serde(default, skip_serializing_if = "is_default")]
    pub aim: Aim,
    /// Downward acceleration in canvas heights per second squared; negative rises
    /// (animatable as `particles.gravity`).
    #[serde(default, skip_serializing_if = "is_default")]
    pub gravity: f64,
    /// Rightward acceleration in canvas heights per second squared; negative blows left
    /// (animatable as `particles.wind`).
    #[serde(default, skip_serializing_if = "is_default")]
    pub wind: f64,
    /// How quickly particles slow down, per second, 0 to 20: speed falls by e^(-drag) each
    /// second, so with gravity they reach a steady speed.
    #[serde(default, skip_serializing_if = "is_default")]
    pub drag: f64,
    /// How far turbulence pushes particles off their path, in canvas heights, 0 to 2
    /// (animatable as `particles.turbulence`).
    #[serde(default, skip_serializing_if = "is_default")]
    pub turbulence: f64,
    /// The size of turbulence's swirls in canvas heights, 0.01 to 10 (0.2 when omitted).
    #[serde(
        default = "turbulence_scale",
        skip_serializing_if = "is_turbulence_scale"
    )]
    pub turbulence_scale: f64,
    /// Rotation at birth in degrees clockwise.
    #[serde(default, skip_serializing_if = "is_default")]
    pub rotation: f64,
    /// How much the rotation varies either way, in degrees.
    #[serde(default, skip_serializing_if = "is_default")]
    pub rotation_variance: f64,
    /// Turning speed in degrees per second, clockwise.
    #[serde(default, skip_serializing_if = "is_default")]
    pub spin: f64,
    /// How much the turning speed varies either way.
    #[serde(default, skip_serializing_if = "is_default")]
    pub spin_variance: f64,
    /// Diameter at birth in canvas heights, 0 to 2 (animatable as `particles.size`).
    #[serde(default = "size", skip_serializing_if = "is_size")]
    pub size: f64,
    /// How much the diameter varies either way.
    #[serde(default, skip_serializing_if = "is_default")]
    pub size_variance: f64,
    /// Size multipliers from birth to death, spread evenly over the life, 1 to 8 values
    /// from 0 to 10 (`[1]` keeps the size).
    #[serde(default = "one_stop", skip_serializing_if = "is_one_stop")]
    pub size_over_life: Vec<f64>,
    /// Opacity at birth, 0 to 1 (animatable as `particles.opacity`).
    #[serde(default = "opacity", skip_serializing_if = "is_opacity")]
    pub opacity: f64,
    /// Opacity multipliers from birth to death, spread evenly over the life, 1 to 8 values
    /// from 0 to 1 (`[1, 0]`, fading out, when omitted).
    #[serde(default = "fade_out", skip_serializing_if = "is_fade_out")]
    pub opacity_over_life: Vec<f64>,
    /// Colours: from birth to death, or to pick one from per particle (`color_by`); 1 to 8
    /// (white when omitted).
    #[serde(default = "white", skip_serializing_if = "is_white")]
    pub colors: Vec<Color>,
    /// How `colors` are used.
    #[serde(default, skip_serializing_if = "is_default")]
    pub color_by: ColorBy,
    /// How overlapping particles combine with each other.
    #[serde(default, skip_serializing_if = "is_default")]
    pub blend: ParticleBlend,
    /// What each particle looks like.
    #[serde(default, skip_serializing_if = "is_default")]
    pub sprite: Sprite,
    /// Chooses the random choices: another seed gives other particles that look alike.
    #[serde(default, skip_serializing_if = "is_default")]
    pub seed: u32,
}

/// Where particles are born.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Emitter {
    /// Its shape (a point when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub shape: EmitterShape,
    /// Its centre, as an offset from the canvas centre in fractions of the canvas width and
    /// height like `transform.position`; y grows downwards (animatable as
    /// `particles.emitter.position`, which moves where new particles are born).
    #[serde(default, skip_serializing_if = "is_default")]
    pub position: [f64; 2],
    /// Width and height as fractions of the canvas's, 0 to 4; a line runs along its width.
    #[serde(default = "emitter_size", skip_serializing_if = "is_emitter_size")]
    pub size: [f64; 2],
    /// Clockwise rotation of the shape in degrees.
    #[serde(default, skip_serializing_if = "is_default")]
    pub angle: f64,
    /// Born along the outline of a circle or rectangle instead of inside it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub edge: bool,
}

impl Default for Emitter {
    fn default() -> Emitter {
        Emitter {
            shape: EmitterShape::Point,
            position: [0.0, 0.0],
            size: emitter_size(),
            angle: 0.0,
            edge: false,
        }
    }
}

/// The shape particles are born in. Emitting along a shape clip's path is left for later.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EmitterShape {
    /// One point.
    #[default]
    Point,
    /// A line as long as the emitter's width.
    Line,
    /// An ellipse of the emitter's size.
    Circle,
    /// A rectangle of the emitter's size.
    Rectangle,
}

/// Many particles born at one moment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Burst {
    /// When, in emission time.
    pub at: Flicks,
    /// How many, 1 to 5000.
    pub count: u32,
}

/// What a particle's direction is measured from.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Aim {
    /// `direction`, the same for every particle.
    #[default]
    Direction,
    /// Away from the emitter's centre, so a circle or burst sprays outwards.
    Outward,
    /// Towards the emitter's centre.
    Inward,
    /// Each particle travels from where it is born to the emitter's centre over its life,
    /// easing in and out, so sparks gather into one point; speed, direction, gravity, wind
    /// and drag do not apply.
    Converge,
}

/// How a particle's colour is chosen.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColorBy {
    /// Through the colours from birth to death.
    #[default]
    Life,
    /// One of the colours at random, kept for life.
    Random,
}

/// How overlapping particles combine with each other (the clip's own blend mode decides
/// how the whole picture combines with what is below).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParticleBlend {
    /// Their light adds up, so crowds glow.
    #[default]
    Add,
    /// Later particles cover earlier ones.
    Normal,
}

/// What a particle looks like.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sprite {
    /// The kind (a soft dot when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: SpriteKind,
    /// Edge softness from 0 (crisp) to 1 (a soft glow); 0.5 when omitted.
    #[serde(default = "softness", skip_serializing_if = "is_softness")]
    pub softness: f64,
    /// A spark's streak, as seconds of travel at its speed, 0 to 1 (0.05 when omitted).
    #[serde(default = "stretch", skip_serializing_if = "is_stretch")]
    pub stretch: f64,
    /// A star's points, 3 to 12 (4 when omitted).
    #[serde(default = "points", skip_serializing_if = "is_points")]
    pub points: u32,
    /// A square's width over its height, 0.1 to 10 (1 when omitted).
    #[serde(default = "aspect", skip_serializing_if = "is_aspect")]
    pub aspect: f64,
    /// Flips per second around the particle's own axis, so confetti tumbles, 0 to 10.
    #[serde(default, skip_serializing_if = "is_default")]
    pub flip: f64,
    /// An `image` sprite's image asset (`asset_…`): its brightness is how much it covers,
    /// tinted by the particle's colour, so light on black works best.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<Id>,
}

impl Default for Sprite {
    fn default() -> Sprite {
        Sprite {
            kind: SpriteKind::Dot,
            softness: softness(),
            stretch: stretch(),
            points: points(),
            aspect: aspect(),
            flip: 0.0,
            asset: None,
        }
    }
}

/// The kinds of sprite.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpriteKind {
    /// A round dot, crisp or soft.
    #[default]
    Dot,
    /// A streak stretched along the direction of travel, bright at its head.
    Spark,
    /// A star with thin points.
    Star,
    /// A ring.
    Ring,
    /// A rectangle, such as a piece of confetti.
    Square,
    /// An image asset (`asset`).
    Image,
}

impl SpriteKind {
    /// Every kind's name, in order.
    pub const NAMES: &'static [&'static str] = &["dot", "spark", "star", "ring", "square", "image"];
}

macro_rules! defaults {
    ($($name:ident, $is:ident: $ty:ty = $value:expr;)*) => {
        $(
            fn $name() -> $ty {
                $value
            }

            fn $is(value: &$ty) -> bool {
                *value == $name()
            }
        )*
    };
}

defaults! {
    rate, is_rate: f64 = 30.0;
    life, is_life: f64 = 2.0;
    speed, is_speed: f64 = 0.25;
    direction, is_direction: f64 = -90.0;
    spread, is_spread: f64 = 30.0;
    turbulence_scale, is_turbulence_scale: f64 = 0.2;
    size, is_size: f64 = 0.02;
    one_stop, is_one_stop: Vec<f64> = vec![1.0];
    opacity, is_opacity: f64 = 1.0;
    fade_out, is_fade_out: Vec<f64> = vec![1.0, 0.0];
    white, is_white: Vec<Color> = vec![Color::default()];
    emitter_size, is_emitter_size: [f64; 2] = [0.5, 0.5];
    softness, is_softness: f64 = 0.5;
    stretch, is_stretch: f64 = 0.05;
    points, is_points: u32 = 4;
    aspect, is_aspect: f64 = 1.0;
}

impl Default for ParticleContent {
    /// A gentle fountain of soft white dots rising from the canvas centre.
    fn default() -> ParticleContent {
        ParticleContent {
            emitter: Emitter::default(),
            rate: rate(),
            bursts: Vec::new(),
            prewarm: Flicks::ZERO,
            offset: Flicks::ZERO,
            life: life(),
            life_variance: 0.0,
            speed: speed(),
            speed_variance: 0.0,
            direction: direction(),
            spread: spread(),
            aim: Aim::Direction,
            gravity: 0.0,
            wind: 0.0,
            drag: 0.0,
            turbulence: 0.0,
            turbulence_scale: turbulence_scale(),
            rotation: 0.0,
            rotation_variance: 0.0,
            spin: 0.0,
            spin_variance: 0.0,
            size: size(),
            size_variance: 0.0,
            size_over_life: one_stop(),
            opacity: opacity(),
            opacity_over_life: fade_out(),
            colors: white(),
            color_by: ColorBy::Life,
            blend: ParticleBlend::Add,
            sprite: Sprite::default(),
            seed: 0,
        }
    }
}

/// An animatable particle setting (ADR-0038): a particle reads it when it is born and keeps
/// it for life, except `particles.rate`, whose curve decides when particles are born.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleProperty {
    /// The property path, such as `particles.rate`.
    pub path: &'static str,
    /// True for a pair `[x, y]`, false for a number.
    pub pair: bool,
    /// The lowest value allowed.
    pub min: Option<f64>,
    /// The highest value allowed.
    pub max: Option<f64>,
    /// What it means.
    pub meaning: &'static str,
}

/// The animatable particle settings.
pub const PARTICLE_PROPERTIES: &[ParticleProperty] = &[
    ParticleProperty {
        path: "particles.rate",
        pair: false,
        min: Some(0.0),
        max: Some(MAX_PARTICLE_RATE),
        meaning: "particles born per second, 0 to 2000",
    },
    ParticleProperty {
        path: "particles.speed",
        pair: false,
        min: Some(0.0),
        max: Some(20.0),
        meaning: "speed at birth in canvas heights per second",
    },
    ParticleProperty {
        path: "particles.direction",
        pair: false,
        min: None,
        max: None,
        meaning: "direction at birth in degrees clockwise from rightwards: 90 down, -90 up",
    },
    ParticleProperty {
        path: "particles.spread",
        pair: false,
        min: Some(0.0),
        max: Some(360.0),
        meaning: "width of the cone of directions in degrees",
    },
    ParticleProperty {
        path: "particles.gravity",
        pair: false,
        min: Some(-20.0),
        max: Some(20.0),
        meaning: "downward acceleration in canvas heights per second squared; negative rises",
    },
    ParticleProperty {
        path: "particles.wind",
        pair: false,
        min: Some(-20.0),
        max: Some(20.0),
        meaning: "rightward acceleration in canvas heights per second squared",
    },
    ParticleProperty {
        path: "particles.turbulence",
        pair: false,
        min: Some(0.0),
        max: Some(2.0),
        meaning: "how far turbulence pushes particles off their path, in canvas heights",
    },
    ParticleProperty {
        path: "particles.size",
        pair: false,
        min: Some(0.0),
        max: Some(2.0),
        meaning: "diameter at birth in canvas heights",
    },
    ParticleProperty {
        path: "particles.opacity",
        pair: false,
        min: Some(0.0),
        max: Some(1.0),
        meaning: "opacity at birth, 0 to 1",
    },
    ParticleProperty {
        path: "particles.emitter.position",
        pair: true,
        min: None,
        max: None,
        meaning: "[x, y]: where new particles are born, as an offset from the canvas centre in fractions of the canvas width and height",
    },
];

/// The particle properties as entries of the closed vocabulary (ADR-0011 item 3).
pub(crate) fn particle_properties() -> Vec<Property> {
    PARTICLE_PROPERTIES
        .iter()
        .map(|p| Property {
            path: p.path,
            kind: if p.pair {
                PropertyKind::Pair
            } else {
                PropertyKind::Number
            },
            applies: Applies::Particles,
            animatable: true,
            min: p.min,
            max: p.max,
            choices: &[],
            meaning: p.meaning,
        })
        .collect()
}

impl ParticleContent {
    /// The plain value of an animatable particle property (`particles.rate`, ...).
    pub fn value(&self, path: &str) -> Option<KeyValue> {
        Some(match path {
            "particles.rate" => KeyValue::Number(self.rate),
            "particles.speed" => KeyValue::Number(self.speed),
            "particles.direction" => KeyValue::Number(self.direction),
            "particles.spread" => KeyValue::Number(self.spread),
            "particles.gravity" => KeyValue::Number(self.gravity),
            "particles.wind" => KeyValue::Number(self.wind),
            "particles.turbulence" => KeyValue::Number(self.turbulence),
            "particles.size" => KeyValue::Number(self.size),
            "particles.opacity" => KeyValue::Number(self.opacity),
            "particles.emitter.position" => KeyValue::Pair(self.emitter.position),
            _ => return None,
        })
    }

    /// Sets the plain value of an animatable particle property; false when `path` is not
    /// one or `value` is of the wrong kind.
    pub fn set_value(&mut self, path: &str, value: KeyValue) -> bool {
        let field = match (path, value) {
            ("particles.emitter.position", KeyValue::Pair(v)) => {
                self.emitter.position = v;
                return true;
            }
            (_, KeyValue::Number(v)) => (path, v),
            _ => return false,
        };
        let slot = match field.0 {
            "particles.rate" => &mut self.rate,
            "particles.speed" => &mut self.speed,
            "particles.direction" => &mut self.direction,
            "particles.spread" => &mut self.spread,
            "particles.gravity" => &mut self.gravity,
            "particles.wind" => &mut self.wind,
            "particles.turbulence" => &mut self.turbulence,
            "particles.size" => &mut self.size,
            "particles.opacity" => &mut self.opacity,
            _ => return false,
        };
        *slot = field.1;
        true
    }

    /// The image asset an `image` sprite shows.
    pub fn sprite_asset(&self) -> Option<&Id> {
        match self.sprite.kind {
            SpriteKind::Image => self.sprite.asset.as_ref(),
            _ => None,
        }
    }

    /// The longest a particle can live, in seconds.
    pub fn longest_life(&self) -> f64 {
        (self.life + self.life_variance.abs()).clamp(MIN_LIFE, MAX_LIFE)
    }

    /// Problems with these settings, as messages naming the field; empty when valid. The
    /// sprite's asset is checked against the project by validation.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut need = |ok: bool, message: &str| {
            if !ok {
                problems.push(message.to_string());
            }
        };
        let within = |v: f64, low: f64, high: f64| v.is_finite() && v >= low && v <= high;
        let finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
        let e = &self.emitter;
        need(
            finite(&e.position) && finite(&[e.angle]),
            "the emitter's position and angle must be finite",
        );
        need(
            e.size.iter().all(|&v| within(v, 0.0, 4.0)),
            "the emitter's size must be from 0 to 4 times the canvas",
        );
        need(
            within(self.rate, 0.0, MAX_PARTICLE_RATE),
            "rate must be from 0 to 2000 per second",
        );
        need(
            self.bursts.len() <= MAX_BURSTS,
            "a particle clip has at most 64 bursts",
        );
        need(
            self.bursts
                .iter()
                .all(|b| b.count >= 1 && b.count <= MAX_BURST),
            "a burst gives birth to 1 to 5000 particles",
        );
        let longest = Flicks::from_seconds(MAX_PREWARM_SECONDS).unwrap_or(Flicks::ZERO);
        need(
            !self.prewarm.is_negative() && self.prewarm <= longest,
            "prewarm must be from 0 to 30 seconds",
        );
        need(
            within(self.life, MIN_LIFE, MAX_LIFE),
            "life must be from 0.05 to 30 seconds",
        );
        need(
            within(self.life_variance, 0.0, MAX_LIFE),
            "life_variance must be from 0 to 30 seconds",
        );
        need(
            within(self.speed, 0.0, 20.0) && within(self.speed_variance, 0.0, 20.0),
            "speed and speed_variance must be from 0 to 20 canvas heights per second",
        );
        need(
            finite(&[self.direction]),
            "direction must be a number of degrees",
        );
        need(
            within(self.spread, 0.0, 360.0),
            "spread must be from 0 to 360 degrees",
        );
        need(
            within(self.gravity, -20.0, 20.0) && within(self.wind, -20.0, 20.0),
            "gravity and wind must be from -20 to 20 canvas heights per second squared",
        );
        need(within(self.drag, 0.0, 20.0), "drag must be from 0 to 20");
        need(
            within(self.turbulence, 0.0, 2.0),
            "turbulence must be from 0 to 2 canvas heights",
        );
        need(
            within(self.turbulence_scale, 0.01, 10.0),
            "turbulence_scale must be from 0.01 to 10 canvas heights",
        );
        need(
            finite(&[self.rotation, self.spin])
                && self.rotation_variance >= 0.0
                && self.spin_variance >= 0.0
                && finite(&[self.rotation_variance, self.spin_variance]),
            "rotation and spin must be finite, their variances 0 or more",
        );
        need(
            within(self.size, 0.0, 2.0) && within(self.size_variance, 0.0, 2.0),
            "size and size_variance must be from 0 to 2 canvas heights",
        );
        need(
            within(self.opacity, 0.0, 1.0),
            "opacity must be from 0 to 1",
        );
        let curve = |stops: &[f64], high: f64| {
            !stops.is_empty()
                && stops.len() <= MAX_STOPS
                && stops.iter().all(|&v| within(v, 0.0, high))
        };
        need(
            curve(&self.size_over_life, 10.0),
            "size_over_life needs 1 to 8 values from 0 to 10",
        );
        need(
            curve(&self.opacity_over_life, 1.0),
            "opacity_over_life needs 1 to 8 values from 0 to 1",
        );
        need(
            !self.colors.is_empty() && self.colors.len() <= MAX_STOPS,
            "colors needs 1 to 8 colours",
        );
        let s = &self.sprite;
        need(
            within(s.softness, 0.0, 1.0),
            "the sprite's softness must be from 0 to 1",
        );
        need(
            within(s.stretch, 0.0, 1.0),
            "the sprite's stretch must be from 0 to 1 second",
        );
        need(
            (3..=12).contains(&s.points),
            "a star sprite has 3 to 12 points",
        );
        need(
            within(s.aspect, 0.1, 10.0),
            "the sprite's aspect must be from 0.1 to 10",
        );
        need(
            within(s.flip, 0.0, 10.0),
            "the sprite's flip must be from 0 to 10 per second",
        );
        match (s.kind, &s.asset) {
            (SpriteKind::Image, None) => problems.push("an image sprite needs an asset".into()),
            (SpriteKind::Image, Some(_)) | (_, None) => {}
            (_, Some(_)) => problems.push("only an image sprite takes an asset".into()),
        }
        problems
    }
}

/// Problems with a particle emitter in `project` (ADR-0038), so commands can report them
/// before they change anything: its settings, and an image sprite's asset.
pub fn particle_issues(project: &Project, content: &ParticleContent) -> Vec<Issue> {
    let issue = |message: String| Issue {
        path: "particles".into(),
        code: "fosscut.model.particles".into(),
        message,
    };
    let mut issues: Vec<Issue> = content.problems().into_iter().map(issue).collect();
    if let Some(asset) = &content.sprite.asset {
        match project.media.get(asset) {
            None => issues.push(Issue {
                code: "fosscut.model.missing_asset".into(),
                ..issue(format!("the sprite's asset {asset} does not exist"))
            }),
            Some(found) if found.kind != AssetKind::Image => issues.push(issue(format!(
                "the sprite's asset {asset} is a {:?} asset, not an image",
                found.kind
            ))),
            Some(_) => {}
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particles_round_trip_sparsely() {
        let plain = ParticleContent::default();
        assert_eq!(serde_json::to_value(&plain).unwrap(), serde_json::json!({}));
        assert_eq!(
            serde_json::from_value::<ParticleContent>(serde_json::json!({})).unwrap(),
            plain
        );
        assert!(plain.problems().is_empty(), "{:?}", plain.problems());

        let mut sparks = ParticleContent {
            rate: 120.0,
            spread: 50.0,
            gravity: 1.2,
            drag: 0.5,
            colors: vec![
                Color::parse("#FFF2C0").unwrap(),
                Color::parse("#FF6A00").unwrap(),
            ],
            bursts: vec![Burst {
                at: Flicks::from_seconds(1).unwrap(),
                count: 300,
            }],
            ..ParticleContent::default()
        };
        sparks.emitter.shape = EmitterShape::Line;
        sparks.emitter.edge = true;
        sparks.sprite.kind = SpriteKind::Spark;
        sparks.sprite.stretch = 0.08;
        let json = serde_json::to_value(&sparks).unwrap();
        assert_eq!(
            json["emitter"],
            serde_json::json!({"shape": "line", "edge": true})
        );
        assert_eq!(
            json["sprite"],
            serde_json::json!({"kind": "spark", "stretch": 0.08})
        );
        assert!(json.get("life").is_none(), "defaults are omitted");
        let back: ParticleContent = serde_json::from_value(json).unwrap();
        assert_eq!(back, sparks);
        assert!(
            serde_json::from_value::<ParticleContent>(serde_json::json!({"bogus": 1})).is_err()
        );
    }

    #[test]
    fn particle_clips_round_trip_beside_the_clip_fields() {
        // The emitter's speed, opacity and blend share names with the clip's own fields.
        let content = ParticleContent {
            speed: 0.12,
            opacity: 0.5,
            blend: ParticleBlend::Normal,
            ..ParticleContent::default()
        };
        let mut clip = crate::Clip::new(
            Id::new(crate::IdKind::Clip, 3),
            Flicks::ZERO,
            Flicks::from_seconds(2).unwrap(),
            crate::ClipContent::Particles {
                particles: Box::new(content),
            },
        );
        clip.opacity = 0.8;
        let json = serde_json::to_value(&clip).unwrap();
        assert_eq!(json["kind"], "particles");
        assert_eq!(json["particles"]["speed"], 0.12);
        assert_eq!(json["particles"]["blend"], "normal");
        assert_eq!(json["opacity"], 0.8);
        assert!(
            json.get("speed").is_none(),
            "the clip plays at normal speed"
        );
        let back: crate::Clip = serde_json::from_value(json).unwrap();
        assert_eq!(back, clip);
        assert_eq!(back.content.kind_name(), "particles");
        assert_eq!(back.content.asset(), None);
    }

    #[test]
    fn problems_name_the_field() {
        let bad = |change: fn(&mut ParticleContent)| {
            let mut content = ParticleContent::default();
            change(&mut content);
            content.problems()
        };
        assert!(bad(|c| c.rate = 5000.0)[0].contains("rate"));
        assert!(bad(|c| c.life = 0.0)[0].contains("life"));
        assert!(bad(|c| c.size_over_life.clear())[0].contains("size_over_life"));
        assert!(bad(|c| c.opacity_over_life = vec![2.0])[0].contains("opacity_over_life"));
        assert!(bad(|c| c.colors.clear())[0].contains("colors"));
        assert!(bad(|c| c.sprite.kind = SpriteKind::Image)[0].contains("asset"));
        assert!(
            bad(|c| c.sprite.asset = Some(Id::new(crate::IdKind::Asset, 1)))[0]
                .contains("only an image")
        );
        assert!(bad(|c| c.sprite.points = 2)[0].contains("star"));
        assert!(
            bad(|c| c.bursts.push(Burst {
                at: Flicks::ZERO,
                count: 0
            }))[0]
                .contains("burst")
        );
        assert!(bad(|c| c.prewarm = Flicks::from_seconds(31).unwrap())[0].contains("prewarm"));
        assert!(bad(|c| c.emitter.position = [f64::NAN, 0.0])[0].contains("emitter"));
        assert!(bad(|c| c.turbulence_scale = 0.0)[0].contains("turbulence_scale"));
    }

    #[test]
    fn animatable_values_read_and_write_by_path() {
        let mut content = ParticleContent::default();
        for &ParticleProperty { path, pair, .. } in PARTICLE_PROPERTIES {
            let value = content.value(path).unwrap_or_else(|| panic!("{path}"));
            assert_eq!(matches!(value, KeyValue::Pair(_)), pair, "{path}");
            let changed = match value {
                KeyValue::Number(v) => KeyValue::Number(v + 0.5),
                KeyValue::Pair([x, y]) => KeyValue::Pair([x + 0.5, y]),
            };
            assert!(content.set_value(path, changed), "{path}");
            assert_eq!(content.value(path), Some(changed), "{path}");
        }
        assert_eq!(content.value("particles.life"), None);
        assert!(!content.set_value("particles.rate", KeyValue::Pair([1.0, 2.0])));
        assert!(!content.set_value("particles.unknown", KeyValue::Number(1.0)));
        assert_eq!(particle_properties().len(), PARTICLE_PROPERTIES.len());
    }
}
