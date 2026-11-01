//! Particle emitters (ADR-0038): which particles a particle clip shows at one moment, and
//! where, how large, how turned and in what colour each one is.
//!
//! Everything is a pure function of the clip and the time, so preview scrubbing, frame
//! grabs and exports agree exactly whatever was rendered before:
//!
//! - **Births:** continuous particles are numbered from the start of emission. The number
//!   born by emission time `t` is the integral of the rate (closed form for a constant
//!   rate; for a keyframed rate, the midpoint rule on a grid of 32 steps between keyframes,
//!   which depends only on the keyframes, never on the time drawn). Particle `n` is born
//!   where that count reaches `n` plus a random fraction. Bursts number their particles
//!   per burst.
//! - **Randomness:** every random choice comes from an integer hash (splitmix64) of the
//!   seed, the stream (continuous or burst), the particle's number and the choice, so no
//!   floating-point sine hashes differ between machines.
//! - **Motion:** a particle's place at age `a` is closed form: constant acceleration
//!   (gravity, wind) with exponential drag, or for `converge` an eased path to the
//!   emitter's centre. Turbulence adds the difference of a smooth hashed value-noise field
//!   between birth and now, so it starts at nothing and stays bounded.
//! - **Limits:** at most [`MAX_PARTICLES`] particles are drawn, the newest when more are
//!   alive, and at most [`MAX_INSTANCES`] sprites with motion blur's shutter samples.
//!
//! Animated values other than the rate are read at each particle's birth.

use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

use fosscut_model::{
    Aim, Clip, ColorBy, EmitterShape, KeyValue, Keyframe, MAX_LIFE, MAX_PARTICLE_RATE,
    MAX_PARTICLES, MIN_LIFE, ParticleContent, SpriteKind, evaluate,
};
use fosscut_time::{FLICKS_PER_SECOND, Flicks};

use crate::plan::SourceRef;
use crate::text::linear_color;

/// The most sprites drawn for one particle layer, counting each of motion blur's shutter
/// samples: with more particles, fewer samples are taken.
pub const MAX_INSTANCES: usize = 200_000;

/// Steps of the birth-count grid between two keyframes of a keyframed rate.
const RATE_STEPS: usize = 32;

/// How fast the turbulence field changes, in field cells per second.
const TURBULENCE_FLOW: f64 = 0.5;

/// The step back used to measure a spark's velocity, in seconds.
const VELOCITY_STEP: f64 = 1.0 / 240.0;

/// A particle clip at one moment, as the planner hands it to the renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleLayer {
    /// The emitter.
    pub content: ParticleContent,
    /// The clip's `particles.*` keyframes, times from the clip's start.
    pub keyframes: BTreeMap<String, Vec<Keyframe>>,
    /// The moment drawn, from the clip's start.
    pub time: Flicks,
    /// One frame of the timeline: how long motion blur's shutter can stay open.
    pub frame: Flicks,
    /// The frame of an `image` sprite's asset.
    pub sprite: Option<SourceRef>,
}

impl ParticleLayer {
    /// The particle layer of `clip`, showing `content`, at `time` from its start on a
    /// timeline whose frames last `frame`.
    pub fn of(clip: &Clip, content: &ParticleContent, time: Flicks, frame: Flicks) -> Self {
        ParticleLayer {
            content: content.clone(),
            keyframes: clip
                .keyframes
                .iter()
                .filter(|(path, _)| path.starts_with("particles."))
                .map(|(path, keys)| (path.clone(), keys.clone()))
                .collect(),
            time,
            frame,
            sprite: content.sprite_asset().map(|asset| SourceRef {
                asset: asset.clone(),
                time: Flicks::ZERO,
            }),
        }
    }
}

impl crate::plan::Layer {
    /// A full-opacity layer drawing the particles of `layer` over the whole canvas.
    pub fn particles(layer: ParticleLayer) -> crate::plan::Layer {
        crate::plan::Layer::with_source(crate::plan::LayerSource::Particles(Box::new(layer)))
    }
}

/// One particle as drawn, in canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    /// Its centre, from the canvas's top-left corner.
    pub center: [f32; 2],
    /// Its diameter.
    pub size: f32,
    /// A spark's streak: the distance between the centres of its round ends, along
    /// `angle`; 0 for other sprites.
    pub length: f32,
    /// The direction of its x axis in radians clockwise from rightwards: a spark's
    /// direction of travel, or the particle's rotation.
    pub angle: f32,
    /// Its width factor while it tumbles, -1 to 1 (1 when it does not).
    pub flip: f32,
    /// Linear-light colour and opacity (not premultiplied).
    pub color: [f32; 4],
}

/// The particles of `layer` at its time on a `canvas` (timeline pixels), oldest first.
pub fn particles(layer: &ParticleLayer, canvas: (u32, u32)) -> Vec<Particle> {
    let emission = Emission::new(layer, canvas);
    emission.at(emission.now)
}

/// The particles of `layer` as motion blur sees them: at `samples` moments spread evenly
/// over a shutter open for `amount` of a frame around its time, each sample's opacity
/// divided by the samples, so they average. Without a shutter, [`particles`].
pub fn particles_during(
    layer: &ParticleLayer,
    canvas: (u32, u32),
    shutter: Option<(f64, usize)>,
) -> Vec<Particle> {
    let emission = Emission::new(layer, canvas);
    let Some((amount, samples)) = shutter.filter(|(a, s)| *a > 0.0 && *s > 1) else {
        return emission.at(emission.now);
    };
    let open = amount.clamp(0.0, 1.0) * layer.frame.as_seconds_f64();
    // The middle sample tells how many particles there are; fewer samples keep the sprite
    // count within MAX_INSTANCES.
    let middle = emission.at(emission.now);
    let samples = samples.min(MAX_INSTANCES / middle.len().max(1)).max(1);
    if samples == 1 {
        return middle;
    }
    let mut all = Vec::with_capacity(middle.len() * samples);
    for i in 0..samples {
        let share = (i as f64 + 0.5) / samples as f64 - 0.5;
        for mut particle in emission.at(emission.now + share * open) {
            particle.color[3] /= samples as f32;
            all.push(particle);
        }
    }
    all
}

/// A 64-bit mix (the splitmix64 finalizer).
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The hash of one random choice: seed, stream, particle number and choice.
fn hash(seed: u32, stream: u64, index: u64, salt: u64) -> u64 {
    let a = mix(u64::from(seed) ^ 0x9E37_79B9_7F4A_7C15);
    let b = mix(a.wrapping_add(stream.wrapping_mul(0xD6E8_FEB8_6659_FD93)));
    let c = mix(b ^ index);
    mix(c.wrapping_add(salt.wrapping_mul(0xA076_1D64_78BD_642F)))
}

/// A hash as a number from 0 up to 1.
fn unit(hash: u64) -> f64 {
    (hash >> 11) as f64 / (1u64 << 53) as f64
}

/// Two channels of smooth value noise in -1 to 1 on an integer lattice, each corner's value
/// one mix of its coordinates and the channel's key, with quintic easing between corners.
fn noise(keys: [u64; 2], x: f64, y: f64, z: f64) -> [f64; 2] {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (ix, iy, iz) = (fx as i64, fy as i64, fz as i64);
    let fade = |t: f64| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v, w) = (fade(x - fx), fade(y - fy), fade(z - fz));
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let mut out = [0.0; 2];
    for (value, key) in out.iter_mut().zip(keys) {
        let corner = |dx: i64, dy: i64, dz: i64| {
            let spot = (ix.wrapping_add(dx) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ (iy.wrapping_add(dy) as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
                ^ (iz.wrapping_add(dz) as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
            unit(mix(spot ^ key)) * 2.0 - 1.0
        };
        let plane = |dz: i64| {
            lerp(
                lerp(corner(0, 0, dz), corner(1, 0, dz), u),
                lerp(corner(0, 1, dz), corner(1, 1, dz), u),
                v,
            )
        };
        *value = lerp(plane(0), plane(1), w);
    }
    out
}

/// The lattice keys of the turbulence field of `seed`: per octave, one per axis.
fn field_keys(seed: u32) -> [[u64; 2]; 2] {
    let key = |octave: u64, axis: u64| hash(seed, 0xF1E1D, octave, axis);
    [[key(0, 0), key(0, 1)], [key(1, 0), key(1, 1)]]
}

/// The turbulence field with `keys`: two octaves of noise per axis.
fn field(keys: &[[u64; 2]; 2], p: [f64; 2], z: f64) -> [f64; 2] {
    let coarse = noise(keys[0], p[0], p[1], z);
    let fine = noise(keys[1], 2.0 * p[0] + 17.3, 2.0 * p[1] - 5.1, 2.0 * z);
    [
        (coarse[0] + 0.5 * fine[0]) / 1.5,
        (coarse[1] + 0.5 * fine[1]) / 1.5,
    ]
}

/// Where along `stops` (spread evenly from 0 to 1) the value at `s` lies.
fn curve(stops: &[f64], s: f64) -> f64 {
    match stops {
        [] => 1.0,
        [only] => *only,
        _ => {
            let x = s.clamp(0.0, 1.0) * (stops.len() - 1) as f64;
            let i = (x.floor() as usize).min(stops.len() - 2);
            stops[i] + (stops[i + 1] - stops[i]) * (x - i as f64)
        }
    }
}

/// When continuous particles are born: the number born since emission began, as a
/// function of emission time.
enum Schedule {
    /// A constant rate from `start`.
    Constant { start: f64, rate: f64 },
    /// Cumulative counts at grid times, increasing; linear in between.
    Grid(Vec<(f64, f64)>),
}

impl Schedule {
    /// The number born by emission time `t`.
    fn count(&self, t: f64) -> f64 {
        match self {
            Schedule::Constant { start, rate } => ((t - start) * rate).max(0.0),
            Schedule::Grid(points) => {
                let after = points.partition_point(|(time, _)| *time <= t);
                if after == 0 {
                    return 0.0;
                }
                let (t0, c0) = points[after - 1];
                match points.get(after) {
                    Some(&(t1, c1)) if t1 > t0 => c0 + (c1 - c0) * (t - t0) / (t1 - t0),
                    _ => c0,
                }
            }
        }
    }

    /// The emission time when the count reaches `n`.
    fn time_of(&self, n: f64) -> f64 {
        match self {
            Schedule::Constant { start, rate } => start + n / rate.max(1e-12),
            Schedule::Grid(points) => {
                let at = points.partition_point(|(_, count)| *count < n);
                if at == 0 {
                    return points.first().map_or(0.0, |p| p.0);
                }
                let Some(&(t1, c1)) = points.get(at) else {
                    return points.last().map_or(0.0, |p| p.0);
                };
                let (t0, c0) = points[at - 1];
                if c1 > c0 {
                    t0 + (t1 - t0) * (n - c0) / (c1 - c0)
                } else {
                    t1
                }
            }
        }
    }
}

/// A particle layer prepared for evaluation.
struct Emission<'a> {
    content: &'a ParticleContent,
    keys: &'a BTreeMap<String, Vec<Keyframe>>,
    /// Emission time of the clip's start, in seconds.
    offset: f64,
    /// Emission time drawn, in seconds.
    now: f64,
    /// When emission began, in seconds.
    start: f64,
    canvas: (f64, f64),
    colors: Vec<[f32; 4]>,
    /// The turbulence field's keys.
    field: [[u64; 2]; 2],
}

impl<'a> Emission<'a> {
    fn new(layer: &'a ParticleLayer, canvas: (u32, u32)) -> Emission<'a> {
        let content = &layer.content;
        let offset = content.offset.as_seconds_f64();
        Emission {
            content,
            keys: &layer.keyframes,
            offset,
            now: offset + layer.time.as_seconds_f64(),
            start: -content.prewarm.as_seconds_f64(),
            canvas: (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1))),
            field: field_keys(content.seed),
            colors: content
                .colors
                .iter()
                .map(|c| linear_color(c.as_str()))
                .collect(),
        }
    }

    /// The keyframed value of `path` at emission time `at`, or its plain value.
    fn value(&self, path: &str, at: f64) -> Option<KeyValue> {
        let keys = self.keys.get(path)?;
        let offset = Flicks::new(((at - self.offset) * FLICKS_PER_SECOND as f64).round() as i64);
        evaluate(keys, offset)
    }

    fn number(&self, path: &str, plain: f64, at: f64) -> f64 {
        self.value(path, at)
            .and_then(KeyValue::number)
            .unwrap_or(plain)
    }

    fn pair(&self, path: &str, plain: [f64; 2], at: f64) -> [f64; 2] {
        self.value(path, at)
            .and_then(KeyValue::pair)
            .unwrap_or(plain)
    }

    fn rate(&self, at: f64) -> f64 {
        self.number("particles.rate", self.content.rate, at)
            .clamp(0.0, MAX_PARTICLE_RATE)
    }

    /// The birth schedule, on a grid fixed by the rate's keyframes, reaching `until`.
    fn schedule(&self, until: f64) -> Schedule {
        let Some(keys) = self.keys.get("particles.rate").filter(|k| !k.is_empty()) else {
            return Schedule::Constant {
                start: self.start,
                rate: self.rate(self.start),
            };
        };
        let times: Vec<f64> = keys
            .iter()
            .map(|k| k.time.as_seconds_f64() + self.offset)
            .collect();
        let mut grid = Vec::with_capacity(times.len() * RATE_STEPS + 1);
        for pair in times.windows(2) {
            for step in 0..RATE_STEPS {
                grid.push(pair[0] + (pair[1] - pair[0]) * step as f64 / RATE_STEPS as f64);
            }
        }
        grid.extend(times.last());
        let mut points = vec![(self.start, 0.0)];
        let (mut t, mut count) = (self.start, 0.0);
        for g in grid.into_iter().filter(|g| *g > self.start) {
            if t >= until {
                break;
            }
            count += self.rate((t + g) / 2.0) * (g - t);
            points.push((g, count));
            t = g;
        }
        if t < until {
            // After the last keyframe the rate holds, so any end gives the same births.
            count += self.rate(until) * (until - t);
            points.push((until, count));
        }
        Schedule::Grid(points)
    }

    /// The particles alive at emission time `now`, oldest first.
    fn at(&self, now: f64) -> Vec<Particle> {
        let c = self.content;
        let longest = c.longest_life();
        let mut births: Vec<(f64, u64, u64)> = Vec::new();
        if now >= self.start {
            let schedule = self.schedule(now);
            let born = schedule.count(now);
            let first = (schedule.count(now - longest).floor() as u64).saturating_sub(1);
            let last = born.floor() as u64;
            let first = first.max(last.saturating_sub(MAX_PARTICLES as u64));
            for n in first..=last {
                let jitter = unit(hash(c.seed, 0, n, 0));
                let at = n as f64 + jitter;
                if at > born {
                    continue;
                }
                let birth = schedule.time_of(at);
                if birth <= now && now - birth < longest {
                    births.push((birth, 0, n));
                }
            }
        }
        for (i, burst) in c.bursts.iter().enumerate() {
            let at = burst.at.as_seconds_f64();
            if at <= now && now - at < longest {
                births.extend((0..u64::from(burst.count)).map(|j| (at, 1 + i as u64, j)));
            }
        }
        births.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        if births.len() > MAX_PARTICLES {
            births.drain(..births.len() - MAX_PARTICLES);
        }
        births
            .into_iter()
            .filter_map(|(birth, stream, index)| self.particle(stream, index, birth, now))
            .collect()
    }

    /// A point of the emitter's shape relative to its centre, in canvas heights, from two
    /// random numbers.
    fn emitter_point(&self, a: f64, b: f64) -> [f64; 2] {
        let e = &self.content.emitter;
        let aspect = self.canvas.0 / self.canvas.1;
        let (w, h) = (e.size[0] * aspect, e.size[1]);
        let local = match e.shape {
            EmitterShape::Point => [0.0, 0.0],
            EmitterShape::Line => [(a - 0.5) * w, 0.0],
            EmitterShape::Circle => {
                let r = if e.edge { 1.0 } else { b.sqrt() };
                let (sin, cos) = (TAU * a).sin_cos();
                [cos * r * w / 2.0, sin * r * h / 2.0]
            }
            EmitterShape::Rectangle if e.edge => {
                // Around the outline, evenly by length.
                let d = a * 2.0 * (w + h);
                if d < w {
                    [d - w / 2.0, -h / 2.0]
                } else if d < w + h {
                    [w / 2.0, d - w - h / 2.0]
                } else if d < 2.0 * w + h {
                    [w / 2.0 - (d - w - h), h / 2.0]
                } else {
                    [-w / 2.0, h / 2.0 - (d - 2.0 * w - h)]
                }
            }
            EmitterShape::Rectangle => [(a - 0.5) * w, (b - 0.5) * h],
        };
        let (sin, cos) = e.angle.to_radians().sin_cos();
        [
            local[0] * cos - local[1] * sin,
            local[0] * sin + local[1] * cos,
        ]
    }

    /// Particle `index` of `stream`, born at `birth`, at emission time `now`; `None` once
    /// it has died.
    fn particle(&self, stream: u64, index: u64, birth: f64, now: f64) -> Option<Particle> {
        let c = self.content;
        let r = |salt: u64| unit(hash(c.seed, stream, index, salt));
        let either = |salt: u64| 2.0 * r(salt) - 1.0;
        let age = now - birth;
        let life = (c.life + c.life_variance * either(1)).clamp(MIN_LIFE, MAX_LIFE);
        if !(0.0..life).contains(&age) {
            return None;
        }
        let s = age / life;
        // What the particle is born with.
        let speed = (self.number("particles.speed", c.speed, birth) + c.speed_variance * either(2))
            .max(0.0);
        let direction = self
            .number("particles.direction", c.direction, birth)
            .to_radians();
        let spread = self
            .number("particles.spread", c.spread, birth)
            .clamp(0.0, 360.0)
            .to_radians();
        let gravity = self.number("particles.gravity", c.gravity, birth);
        let wind = self.number("particles.wind", c.wind, birth);
        let turbulence = self
            .number("particles.turbulence", c.turbulence, birth)
            .max(0.0);
        let size =
            (self.number("particles.size", c.size, birth) + c.size_variance * either(6)).max(0.0);
        let opacity = self
            .number("particles.opacity", c.opacity, birth)
            .clamp(0.0, 1.0);
        let aspect = self.canvas.0 / self.canvas.1;
        let centre = self.pair("particles.emitter.position", c.emitter.position, birth);
        let centre = [centre[0] * aspect, centre[1]];
        let local = self.emitter_point(r(3), r(4));
        let p0 = [centre[0] + local[0], centre[1] + local[1]];
        let away = (local[0].hypot(local[1]) > 1e-9).then(|| local[1].atan2(local[0]));
        let base = match c.aim {
            Aim::Direction | Aim::Converge => direction,
            Aim::Outward => away.unwrap_or(direction),
            Aim::Inward => away.map_or(direction, |a| a + PI),
        };
        let theta = base + (r(5) - 0.5) * spread;
        let v0 = [speed * theta.cos(), speed * theta.sin()];
        let accel = [wind, gravity];
        let k = c.drag;
        let scale = c.turbulence_scale.max(0.01);
        let field_at = |p: [f64; 2], t: f64| {
            field(
                &self.field,
                [p[0] / scale, p[1] / scale],
                t * TURBULENCE_FLOW,
            )
        };
        let start_field = (turbulence > 0.0).then(|| field_at(p0, birth));
        // The place at age `a`.
        let place = |a: f64| -> [f64; 2] {
            let (path, settle) = if c.aim == Aim::Converge {
                let t = (a / life).clamp(0.0, 1.0);
                let eased = t * t * (3.0 - 2.0 * t);
                (
                    [
                        p0[0] + (centre[0] - p0[0]) * eased,
                        p0[1] + (centre[1] - p0[1]) * eased,
                    ],
                    1.0 - eased,
                )
            } else if k > 1e-9 {
                let decay = (1.0 - (-k * a).exp()) / k;
                let axis = |i: usize| {
                    let terminal = accel[i] / k;
                    p0[i] + terminal * a + (v0[i] - terminal) * decay
                };
                ([axis(0), axis(1)], 1.0)
            } else {
                let axis = |i: usize| p0[i] + v0[i] * a + 0.5 * accel[i] * a * a;
                ([axis(0), axis(1)], 1.0)
            };
            match start_field {
                Some(from) => {
                    let to = field_at(path, birth + a);
                    [
                        path[0] + turbulence * settle * (to[0] - from[0]),
                        path[1] + turbulence * settle * (to[1] - from[1]),
                    ]
                }
                None => path,
            }
        };
        let here = place(age);
        let sprite = &c.sprite;
        let height = self.canvas.1;
        let (angle, length) = if sprite.kind == SpriteKind::Spark {
            let back = (age - VELOCITY_STEP).max(0.0);
            let before = place(back);
            let dt = age - back;
            let velocity = if dt > 1e-9 {
                [(here[0] - before[0]) / dt, (here[1] - before[1]) / dt]
            } else {
                v0
            };
            let moving = velocity[0].hypot(velocity[1]);
            let angle = if moving > 1e-9 {
                velocity[1].atan2(velocity[0])
            } else {
                theta
            };
            (angle, moving * sprite.stretch * height)
        } else {
            let rotation = c.rotation + c.rotation_variance * either(7);
            let spin = c.spin + c.spin_variance * either(8);
            ((rotation + spin * age).to_radians(), 0.0)
        };
        let flip = if sprite.flip > 0.0 {
            (TAU * (sprite.flip * (0.5 + r(9)) * age + r(10))).cos()
        } else {
            1.0
        };
        let color = match (c.color_by, self.colors.as_slice()) {
            (_, []) => [1.0; 4],
            (_, [only]) => *only,
            (ColorBy::Random, colors) => {
                colors[((r(11) * colors.len() as f64) as usize).min(colors.len() - 1)]
            }
            (ColorBy::Life, colors) => {
                let x = s * (colors.len() - 1) as f64;
                let i = (x.floor() as usize).min(colors.len() - 2);
                let t = (x - i as f64) as f32;
                let (a, b) = (colors[i], colors[i + 1]);
                [0, 1, 2, 3].map(|ch| a[ch] + (b[ch] - a[ch]) * t)
            }
        };
        let alpha = (opacity * curve(&c.opacity_over_life, s)) as f32 * color[3];
        let diameter = size * curve(&c.size_over_life, s) * height;
        if alpha <= 0.0 || diameter <= 0.0 {
            return None;
        }
        Some(Particle {
            center: [
                (self.canvas.0 / 2.0 + here[0] * height) as f32,
                (self.canvas.1 / 2.0 + here[1] * height) as f32,
            ],
            size: diameter as f32,
            length: length as f32,
            angle: angle as f32,
            flip: flip as f32,
            color: [color[0], color[1], color[2], alpha],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_model::{Burst, Interpolation};

    const CANVAS: (u32, u32) = (1920, 1080);

    fn layer(content: ParticleContent, seconds: f64) -> ParticleLayer {
        ParticleLayer {
            content,
            keyframes: BTreeMap::new(),
            time: Flicks::from_seconds_f64(seconds).unwrap(),
            frame: fosscut_time::Rate::FPS_30.frame_duration(),
            sprite: None,
        }
    }

    fn seconds(s: f64) -> Flicks {
        Flicks::from_seconds_f64(s).unwrap()
    }

    #[test]
    fn hashes_and_noise_are_fixed_numbers() {
        // Fixed values: a change to the hash would silently change every project.
        assert_eq!(mix(0), 0);
        assert_eq!(hash(0, 0, 0, 0), hash(0, 0, 0, 0));
        assert_ne!(hash(1, 0, 0, 0), hash(0, 0, 0, 0));
        assert_ne!(hash(0, 1, 0, 0), hash(0, 0, 1, 0));
        let u = unit(hash(7, 0, 3, 2));
        assert!((0.0..1.0).contains(&u));
        let mean = (0..10_000u64).map(|i| unit(hash(3, 0, i, 5))).sum::<f64>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.01, "uniform: {mean}");
        for i in 0..200 {
            let x = i as f64 * 0.173;
            let [a, b] = noise([1, 2], x, -x * 0.7, 0.3);
            assert!((-1.0..=1.0).contains(&a) && (-1.0..=1.0).contains(&b));
            // Smooth: a tiny step changes it a little.
            let near = noise([1, 2], x + 1e-4, -x * 0.7, 0.3);
            assert!((a - near[0]).abs() < 1e-2 && (b - near[1]).abs() < 1e-2);
        }
        let channels = noise([1, 2], 2.5, 3.5, 4.5);
        assert_ne!(channels[0], channels[1], "independent channels");
        // Fixed numbers: changing the hash would silently change every project.
        assert_eq!(hash(0, 0, 0, 0), 1_826_112_205_991_530_872);
        let keys = field_keys(3);
        assert_eq!(
            field(&keys, [0.25, 0.5], 1.0),
            field(&keys, [0.25, 0.5], 1.0)
        );
        assert_ne!(field_keys(4), keys);
        assert_eq!(curve(&[1.0, 0.0], 0.25), 0.75);
        assert_eq!(curve(&[0.0, 1.0, 0.0], 0.5), 1.0);
        assert_eq!(curve(&[0.4], 0.9), 0.4);
    }

    #[test]
    fn a_constant_rate_gives_birth_steadily() {
        let content = ParticleContent {
            rate: 50.0,
            life: 10.0,
            opacity_over_life: vec![1.0],
            ..ParticleContent::default()
        };
        for (t, expected) in [(0.0, 0usize), (1.0, 50), (2.0, 100)] {
            let n = particles(&layer(content.clone(), t), CANVAS).len();
            assert!(n.abs_diff(expected) <= 1, "{t}: {n}");
        }
        // Particles live their life and no longer.
        let short = ParticleContent {
            life: 1.0,
            ..content.clone()
        };
        let n = particles(&layer(short, 5.0), CANVAS).len();
        assert!(n.abs_diff(50) <= 1, "{n}");
        // Prewarm fills the clip from its first frame.
        let warm = ParticleContent {
            prewarm: seconds(2.0),
            ..content
        };
        let n = particles(&layer(warm, 0.0), CANVAS).len();
        assert!(n.abs_diff(100) <= 1, "{n}");
    }

    #[test]
    fn frames_do_not_depend_on_what_came_before() {
        let mut content = ParticleContent {
            rate: 200.0,
            turbulence: 0.2,
            gravity: 0.4,
            drag: 0.8,
            life_variance: 0.5,
            speed_variance: 0.1,
            size_variance: 0.01,
            ..ParticleContent::default()
        };
        content.sprite.kind = SpriteKind::Spark;
        content.bursts.push(Burst {
            at: seconds(0.5),
            count: 300,
        });
        let alone = particles(&layer(content.clone(), 1.25), CANVAS);
        for t in [0.0, 0.5, 1.0, 3.0] {
            let _ = particles(&layer(content.clone(), t), CANVAS);
        }
        assert_eq!(particles(&layer(content.clone(), 1.25), CANVAS), alone);
        content.seed = 1;
        let other = particles(&layer(content, 1.25), CANVAS);
        assert_ne!(other, alone, "another seed, other particles");
        assert!(other.len().abs_diff(alone.len()) < 30);
    }

    #[test]
    fn particles_follow_gravity_drag_and_converge() {
        // One particle, straight right at 0.5 canvas heights per second from the centre.
        let mut content = ParticleContent {
            rate: 0.0,
            speed: 0.5,
            direction: 0.0,
            spread: 0.0,
            life: 10.0,
            opacity_over_life: vec![1.0],
            bursts: vec![Burst {
                at: Flicks::ZERO,
                count: 1,
            }],
            ..ParticleContent::default()
        };
        let at =
            |content: &ParticleContent, t: f64| particles(&layer(content.clone(), t), CANVAS)[0];
        let p = at(&content, 2.0);
        assert!((p.center[0] - (960.0 + 1080.0)).abs() < 0.01, "{p:?}");
        assert!((p.center[1] - 540.0).abs() < 0.01);
        content.gravity = 0.5;
        let p = at(&content, 2.0);
        // y = 0.5 * g * t^2 = 1 canvas height.
        assert!((p.center[1] - 1620.0).abs() < 0.01, "{p:?}");
        content.gravity = 0.0;
        content.drag = 1.0;
        let p = at(&content, 2.0);
        let expected = 960.0 + 0.5 * (1.0 - (-2.0f64).exp()) * 1080.0;
        assert!((f64::from(p.center[0]) - expected).abs() < 0.01, "{p:?}");
        // Converge: from a ring towards its centre, arriving at the end of life.
        content.aim = Aim::Converge;
        content.emitter.shape = EmitterShape::Circle;
        content.emitter.edge = true;
        content.emitter.size = [0.5, 0.5];
        content.life = 2.0;
        let start = at(&content, 0.0);
        let near_end = at(&content, 1.99);
        let distance = |p: Particle| (p.center[0] - 960.0).hypot(p.center[1] - 540.0);
        assert!(distance(start) > 200.0, "{start:?}");
        assert!(distance(near_end) < 1.0, "{near_end:?}");
    }

    #[test]
    fn keyframed_rates_and_emitters_change_births_only() {
        let mut content = ParticleContent {
            rate: 0.0,
            life: 20.0,
            opacity_over_life: vec![1.0],
            ..ParticleContent::default()
        };
        content.speed = 0.0;
        let mut layer = layer(content, 0.0);
        let key = |t: f64, v: KeyValue| Keyframe {
            time: seconds(t),
            value: v,
            interpolation: Interpolation::Linear,
        };
        // From 0 to 100 per second over 2 seconds: 100 born by then.
        layer.keyframes.insert(
            "particles.rate".into(),
            vec![
                key(0.0, KeyValue::Number(0.0)),
                key(2.0, KeyValue::Number(100.0)),
            ],
        );
        let count = |layer: &mut ParticleLayer, t: f64| {
            layer.time = seconds(t);
            particles(layer, CANVAS).len()
        };
        assert!(count(&mut layer, 1.0).abs_diff(25) <= 1);
        assert!(count(&mut layer, 2.0).abs_diff(100) <= 1);
        assert!(count(&mut layer, 3.0).abs_diff(200) <= 1, "holds after");
        // A particle stays where it was born when the emitter moves on.
        layer.keyframes.insert(
            "particles.emitter.position".into(),
            vec![
                key(0.0, KeyValue::Pair([-0.4, 0.0])),
                key(3.0, KeyValue::Pair([0.4, 0.0])),
            ],
        );
        layer.time = seconds(1.5);
        let early = particles(&layer, CANVAS);
        layer.time = seconds(3.0);
        let later = particles(&layer, CANVAS);
        assert_eq!(later[..early.len()], early[..], "old particles stay put");
        assert!(later.last().unwrap().center[0] > early.last().unwrap().center[0] + 100.0);
    }

    #[test]
    fn splits_continue_the_same_emission() {
        let content = ParticleContent {
            rate: 80.0,
            turbulence: 0.1,
            ..ParticleContent::default()
        };
        let whole = particles(&layer(content.clone(), 3.0), CANVAS);
        let right = ParticleContent {
            offset: seconds(2.0),
            ..content
        };
        assert_eq!(particles(&layer(right, 1.0), CANVAS), whole);
    }

    #[test]
    fn motion_blur_spreads_each_particle_over_the_shutter() {
        let content = ParticleContent {
            rate: 0.0,
            speed: 1.0,
            spread: 0.0,
            direction: 0.0,
            opacity_over_life: vec![1.0],
            bursts: vec![Burst {
                at: Flicks::ZERO,
                count: 1,
            }],
            ..ParticleContent::default()
        };
        let layer = layer(content, 1.0);
        let sharp = particles_during(&layer, CANVAS, None);
        let blurred = particles_during(&layer, CANVAS, Some((0.5, 4)));
        assert_eq!(sharp.len(), 1);
        assert_eq!(blurred.len(), 4);
        assert!((blurred.iter().map(|p| p.color[3]).sum::<f32>() - 1.0).abs() < 1e-6);
        let xs: Vec<f32> = blurred.iter().map(|p| p.center[0]).collect();
        assert!(xs.windows(2).all(|w| w[1] > w[0]), "{xs:?}");
        let middle = (xs[1] + xs[2]) / 2.0;
        assert!((middle - sharp[0].center[0]).abs() < 0.01);
    }

    #[test]
    fn the_newest_particles_are_drawn_beyond_the_limit() {
        let content = ParticleContent {
            rate: 2000.0,
            life: 30.0,
            opacity_over_life: vec![1.0],
            ..ParticleContent::default()
        };
        let drawn = particles(&layer(content, 20.0), CANVAS);
        assert_eq!(drawn.len(), MAX_PARTICLES);
    }
}
