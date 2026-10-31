//! Voice filters: sound effects that change the character of a voice, applied by the mixer
//! like the other sound effects (ADR-0026). Their registry specs and the voice presets live
//! in `fosscut_model::voice_filters`.
//!
//! - **Pitch** ([`VoiceEffect::Pitch`]): waveform-similarity overlap-add (Verhelst and
//!   Roelands, 1993) of resampled grains, so the pitch moves while the length stays; with
//!   formants kept, the spectral envelope of the input, estimated by cepstral smoothing
//!   (Oppenheim and Schafer, *Discrete-Time Signal Processing*, homomorphic processing), is
//!   put back onto the shifted sound frame by frame. See `pitch.rs`.
//! - **Robot**: ring modulation (a product with a sine carrier), then a feed-forward comb
//!   tuned to the carrier. **Telephone** and **megaphone**: Butterworth band limits built
//!   from RBJ cookbook biquads (R. Bristow-Johnson, *Audio EQ Cookbook*), a tanh saturator
//!   driven relative to the signal's recent peak, and for the megaphone a horn band and a
//!   short feedback comb. **Broken**: sample-and-hold rate reduction, bit reduction relative
//!   to the recent peak, cut-outs and crackle placed by integer hashing. **Whisper**: a
//!   channel vocoder (H. Dudley, 1939) whose carrier is white noise. See `character.rs`.
//! - **Echo**: a multi-tap delay whose taps fall geometrically like a feedback delay line's
//!   repeats. **Chorus**: three delay lines modulated by phase-shifted sine oscillators, read
//!   with cubic interpolation (J. Dattorro, "Effect design, part 2: delay-line modulation and
//!   chorus", Journal of the Audio Engineering Society 45(10), 1997). **Reverb**: a feedback
//!   delay network (J.-M. Jot and A. Chaigne, "Digital delay networks for designing
//!   artificial reverberators", AES 90th Convention, 1991) with a Hadamard feedback matrix,
//!   per-line absorption filters set from two reverberation times and allpass diffusion of
//!   the input (M. R. Schroeder, "Natural sounding artificial reverberation", JAES 10(3),
//!   1962), rendered once into an impulse response that is faded out before
//!   [`TAIL_SECONDS`] and applied by partitioned FFT convolution. See `space.rs`.
//!
//! **Same sound in every block.** The mixer processes a window of a clip from a fresh state
//! (ADR-0026, item 5). Every filter here is a function of the stream that looks at a bounded
//! stretch of input: its memory ([`VoiceEffect::warm_up_frames`]) before a frame and its
//! look-ahead ([`VoiceEffect::lookahead_frames`]) after it, with every oscillator, noise
//! sample, cut-out and analysis frame placed by the frame's position in the stream rather
//! than by where the buffer starts. Echo repeats and the reverb's tail end within
//! [`TAIL_SECONDS`], and the pitch shifter restarts its splice search at dips in the sound
//! and at least every 0.576 seconds. A window fed that much input on each side therefore
//! comes out as the whole clip does, to float rounding, and the mixer feeds a chain of voice
//! filters only the warm-up it needs (`effects::warm_up_frames`).
//!
//! **Levels.** Saturation is scaled to the recent peak, so it squashes but never lifts a
//! passage above its own peaks. Wet effects blend dry and wet at equal power (`mix` `m`
//! gives `cos(m pi / 2)` of the dry sound and `sin(m pi / 2)` of the wet), so a reverb or an
//! echo, which is uncorrelated with the voice it follows, adds space without making the
//! voice quieter; the whisper blends the same way. Silence in gives exact silence out: no
//! filter adds noise to silence, states are flushed to zero below 1e-25, and no output
//! sample is subnormal.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use fosscut_model::ParamValue;
use fosscut_model::voice_filters as names;
use rustfft::num_complex::Complex64 as Complex;

mod character;
mod pitch;
mod space;
#[cfg(test)]
mod tests;

/// Echo repeats and the reverb's tail end within this many seconds of the sound.
pub const TAIL_SECONDS: f64 = 1.5;

/// One voice filter with its parameters, clamped to their ranges when applied; NaN counts as
/// the lowest value of a range, and as 0 semitones for the pitch.
#[derive(Clone, Debug, PartialEq)]
pub enum VoiceEffect {
    /// Shifts the pitch by `semitones` (-12 to 12; NaN counts as 0) without changing the
    /// length. With `keep_formants`, the spectral envelope stays where it was, so the voice
    /// keeps its speaker's character; without, it moves with the pitch.
    Pitch {
        /// Semitones up (positive) or down.
        semitones: f32,
        /// Keep the spectral envelope (formants) in place.
        keep_formants: bool,
    },
    /// Ring modulation by a sine of `frequency` Hz (20 to 1000) with `depth` (0 to 1), then
    /// a comb tuned to the same frequency.
    Robot {
        /// Carrier frequency in Hz.
        frequency: f32,
        /// 0 leaves the voice unchanged, 1 modulates it fully.
        depth: f32,
    },
    /// A phone line: 300 to 3400 Hz at `narrow` 0, 600 to 2400 Hz at 1, falling by at least
    /// 36 dB per octave outside, with `crunch` (0 to 1) saturation.
    Telephone {
        /// How narrow the band is, 0 to 1.
        narrow: f32,
        /// Saturation, 0 to 1.
        crunch: f32,
    },
    /// A loudhailer: 450 to 4200 Hz, a horn band at 1.8 kHz, `drive` (0 to 1) saturation and
    /// a short resonance.
    Megaphone {
        /// Saturation, 0 to 1.
        drive: f32,
    },
    /// Repeats after `delay_ms` (20 to 1000), each `feedback` (0 to 0.9) times the one
    /// before, blended by `mix` (0 to 1).
    Echo {
        /// Time between repeats, in milliseconds.
        delay_ms: f32,
        /// Level of each repeat relative to the one before.
        feedback: f32,
        /// 0 only the dry sound, 1 only the repeats.
        mix: f32,
    },
    /// A room of `room` size (0 to 1) whose highs die away faster by `damping` (0 to 1),
    /// blended by `mix` (0 to 1).
    Reverb {
        /// 0 a small room, 1 a large hall.
        room: f32,
        /// 0 bright, 1 dark.
        damping: f32,
        /// 0 only the dry sound, 1 only the reverberation.
        mix: f32,
    },
    /// Three copies drifting at `rate` Hz (0.05 to 5) by up to `depth` × 6 ms (0 to 1),
    /// blended by `mix` (0 to 1).
    Chorus {
        /// Modulation rate in Hz.
        rate: f32,
        /// Modulation depth, 0 to 1.
        depth: f32,
        /// 0 only the dry sound, 1 only the copies.
        mix: f32,
    },
    /// A failing microphone, `amount` 0 (unchanged) to 1.
    Broken {
        /// How broken, 0 to 1.
        amount: f32,
    },
    /// Speech turned into breath noise, `amount` 0 (unchanged) to 1 (only whisper).
    Whisper {
        /// How much becomes whisper, 0 to 1.
        amount: f32,
    },
}

impl VoiceEffect {
    /// The filter for an effect of type `effect_type` with every parameter in `values` (as
    /// `fosscut_model::Effect::values_at` gives them), or `None` for another type. Missing
    /// numbers count as NaN and missing flags as off.
    pub fn from_values(
        effect_type: &str,
        values: &BTreeMap<String, ParamValue>,
    ) -> Option<VoiceEffect> {
        let number = |name: &str| {
            values
                .get(name)
                .and_then(ParamValue::number)
                .map_or(f32::NAN, |v| v as f32)
        };
        let flag = |name: &str| matches!(values.get(name), Some(ParamValue::Flag(true)));
        Some(match effect_type {
            names::PITCH => VoiceEffect::Pitch {
                semitones: number("semitones"),
                keep_formants: flag("keep_formants"),
            },
            names::ROBOT => VoiceEffect::Robot {
                frequency: number("frequency"),
                depth: number("depth"),
            },
            names::TELEPHONE => VoiceEffect::Telephone {
                narrow: number("narrow"),
                crunch: number("crunch"),
            },
            names::MEGAPHONE => VoiceEffect::Megaphone {
                drive: number("drive"),
            },
            names::ECHO => VoiceEffect::Echo {
                delay_ms: number("delay"),
                feedback: number("feedback"),
                mix: number("mix"),
            },
            names::REVERB => VoiceEffect::Reverb {
                room: number("room"),
                damping: number("damping"),
                mix: number("mix"),
            },
            names::CHORUS => VoiceEffect::Chorus {
                rate: number("rate"),
                depth: number("depth"),
                mix: number("mix"),
            },
            names::BROKEN => VoiceEffect::Broken {
                amount: number("amount"),
            },
            names::WHISPER => VoiceEffect::Whisper {
                amount: number("amount"),
            },
            _ => return None,
        })
    }

    /// Frames of input after a frame that its output depends on, at `sample_rate`.
    pub fn lookahead_frames(&self, sample_rate: u32) -> usize {
        match *self {
            VoiceEffect::Pitch {
                semitones,
                keep_formants,
            } => pitch::ratio(semitones).map_or(0, |ratio| {
                pitch::lookahead(sample_rate, ratio, keep_formants)
            }),
            _ => 0,
        }
    }

    /// Frames of input before a frame that its output depends on, at `sample_rate`: a window
    /// fed this much before the wanted range sounds exactly as the whole clip does there.
    pub fn warm_up_frames(&self, sample_rate: u32) -> usize {
        let rate = f64::from(sample_rate);
        match *self {
            VoiceEffect::Pitch {
                semitones,
                keep_formants,
            } => pitch::ratio(semitones)
                .map_or(0, |ratio| pitch::warm_up(sample_rate, ratio, keep_formants)),
            VoiceEffect::Robot { frequency, .. } => {
                character::robot_period(rate, frequency_hz(frequency)) + 1
            }
            VoiceEffect::Telephone { .. } | VoiceEffect::Megaphone { .. } => {
                seconds(character::FILTER_SETTLE_SECONDS, rate)
            }
            VoiceEffect::Broken { .. } => seconds(character::BROKEN_MEMORY_SECONDS, rate),
            VoiceEffect::Whisper { .. } => seconds(character::WHISPER_MEMORY_SECONDS, rate),
            VoiceEffect::Echo {
                delay_ms, feedback, ..
            } => space::echo_taps(rate, delay(delay_ms), unit(feedback, 0.9))
                .last()
                .map_or(0, |(at, _)| *at),
            VoiceEffect::Chorus { .. } => seconds(space::CHORUS_MEMORY_SECONDS, rate),
            VoiceEffect::Reverb { .. } => seconds(TAIL_SECONDS, rate) + 1,
        }
    }

    /// Applies the filter in place to whole interleaved frames whose first frame is frame
    /// `start` of the stream.
    pub(crate) fn apply(&self, samples: &mut [f32], channels: usize, sample_rate: u32, start: i64) {
        if channels == 0 || sample_rate == 0 || samples.len() < channels {
            return;
        }
        let rate = f64::from(sample_rate);
        match *self {
            VoiceEffect::Pitch {
                semitones,
                keep_formants,
            } => {
                if let Some(ratio) = pitch::ratio(semitones) {
                    pitch::shift(samples, channels, sample_rate, start, ratio, keep_formants);
                }
            }
            VoiceEffect::Robot { frequency, depth } => character::robot(
                samples,
                channels,
                rate,
                start,
                frequency_hz(frequency),
                unit(depth, 1.0),
            ),
            VoiceEffect::Telephone { narrow, crunch } => character::telephone(
                samples,
                channels,
                rate,
                unit(narrow, 1.0),
                unit(crunch, 1.0),
            ),
            VoiceEffect::Megaphone { drive } => {
                character::megaphone(samples, channels, rate, unit(drive, 1.0));
            }
            VoiceEffect::Broken { amount } => {
                character::broken(samples, channels, rate, start, unit(amount, 1.0));
            }
            VoiceEffect::Whisper { amount } => {
                character::whisper(samples, channels, rate, start, unit(amount, 1.0));
            }
            VoiceEffect::Echo {
                delay_ms,
                feedback,
                mix,
            } => space::echo(
                samples,
                channels,
                rate,
                delay(delay_ms),
                unit(feedback, 0.9),
                unit(mix, 1.0),
            ),
            VoiceEffect::Chorus {
                rate: hz,
                depth,
                mix,
            } => space::chorus(
                samples,
                channels,
                rate,
                start,
                clamp(hz, 0.05, 5.0),
                unit(depth, 1.0),
                unit(mix, 1.0),
            ),
            VoiceEffect::Reverb { room, damping, mix } => space::reverb(
                samples,
                channels,
                sample_rate,
                unit(room, 1.0),
                unit(damping, 1.0),
                unit(mix, 1.0),
            ),
        }
    }
}

/// `x` clamped to `lo..=hi`, with NaN as `lo`.
fn clamp(x: f32, lo: f64, hi: f64) -> f64 {
    if x.is_nan() {
        lo
    } else {
        f64::from(x).clamp(lo, hi)
    }
}

/// `x` clamped to `0..=hi`, with NaN as 0.
fn unit(x: f32, hi: f64) -> f64 {
    clamp(x, 0.0, hi)
}

/// A robot carrier frequency in Hz.
fn frequency_hz(x: f32) -> f64 {
    clamp(x, 20.0, 1000.0)
}

/// An echo delay in seconds.
fn delay(delay_ms: f32) -> f64 {
    clamp(delay_ms, 20.0, 1000.0) / 1000.0
}

/// Whole frames (rounded up) in `seconds` at `rate`.
fn seconds(seconds: f64, rate: f64) -> usize {
    (seconds * rate).ceil() as usize
}

/// Values smaller than this are flushed to zero, so filter states never go subnormal.
const TINY: f64 = 1e-25;

/// `x`, or zero when it is tiny.
fn flush(x: f64) -> f64 {
    if x.abs() < TINY { 0.0 } else { x }
}

/// An output sample: tiny values become zero, so no output is subnormal.
fn sample(x: f64) -> f32 {
    flush(x) as f32
}

/// An input sample as a number, with non-finite values as silence.
fn input(x: f32) -> f64 {
    if x.is_finite() { f64::from(x) } else { 0.0 }
}

/// Channel `c` of interleaved `samples`, as numbers.
fn deinterleave(samples: &[f32], channels: usize, c: usize) -> Vec<f64> {
    samples
        .iter()
        .skip(c)
        .step_by(channels)
        .map(|v| input(*v))
        .collect()
}

/// The gains of the dry and the wet sound at `mix` (0 to 1), at equal power: exactly the dry
/// sound at 0 and exactly the wet at 1.
fn mix_gains(mix: f64) -> (f64, f64) {
    if mix <= 0.0 {
        (1.0, 0.0)
    } else if mix >= 1.0 {
        (0.0, 1.0)
    } else {
        let angle = mix * std::f64::consts::FRAC_PI_2;
        (angle.cos(), angle.sin())
    }
}

/// A uniform number in 0 to 1 for `position` and `seed`, from the SplitMix64 finalizer
/// (G. Steele, D. Lea and C. Flood, "Fast splittable pseudorandom number generators",
/// OOPSLA 2014): the same for the same inputs on every call and machine.
fn uniform(position: i64, seed: u64) -> f64 {
    let mut z = (position as u64)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(seed.wrapping_mul(0xd1b5_4a32_d192_ed03));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// A uniform number in -1 to 1 for `position` and `seed`.
fn signed(position: i64, seed: u64) -> f64 {
    2.0 * uniform(position, seed) - 1.0
}

/// `x` at fractional index `at`, by Catmull-Rom cubic interpolation; silence outside.
fn cubic(x: &[f64], at: f64) -> f64 {
    let base = at.floor();
    let t = at - base;
    let i = base as i64;
    let get = |j: i64| {
        if j < 0 {
            0.0
        } else {
            x.get(j as usize).copied().unwrap_or(0.0)
        }
    };
    let (p0, p1, p2, p3) = (get(i - 1), get(i), get(i + 1), get(i + 2));
    p1 + 0.5
        * t
        * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

/// Quality factors of the second-order sections of Butterworth filters of order 4 and 6.
const BUTTERWORTH_4: [f64; 2] = [0.541_196_100_1, 1.306_562_964_9];
const BUTTERWORTH_6: [f64; 3] = [
    0.517_638_090_2,
    std::f64::consts::FRAC_1_SQRT_2,
    1.931_851_652_6,
];

/// A second-order filter from the RBJ Audio EQ Cookbook, normalised so that `a0` is 1, run
/// in transposed direct form II in double precision with its state flushed below [`TINY`].
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    /// The cookbook's `cos(w0)` and `alpha` for `f0` Hz, kept between 10 Hz and 0.45 of
    /// `rate`.
    fn prepare(rate: f64, f0: f64, q: f64) -> (f64, f64) {
        let f0 = f0.clamp(10.0, 0.45 * rate);
        let w0 = 2.0 * PI * f0 / rate;
        (w0.cos(), w0.sin() / (2.0 * q))
    }

    fn normalized(b: [f64; 3], a: [f64; 3]) -> Self {
        Biquad {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
        }
    }

    fn low_pass(rate: f64, f0: f64, q: f64) -> Self {
        let (cos, alpha) = Self::prepare(rate, f0, q);
        let b = (1.0 - cos) / 2.0;
        Self::normalized([b, 1.0 - cos, b], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    fn high_pass(rate: f64, f0: f64, q: f64) -> Self {
        let (cos, alpha) = Self::prepare(rate, f0, q);
        let b = (1.0 + cos) / 2.0;
        Self::normalized([b, -(1.0 + cos), b], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    /// A band-pass with a peak gain of 1 at `f0`.
    fn band_pass(rate: f64, f0: f64, q: f64) -> Self {
        let (cos, alpha) = Self::prepare(rate, f0, q);
        Self::normalized([alpha, 0.0, -alpha], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    /// The frequency response at `w` radians per sample.
    fn response(&self, w: f64) -> Complex {
        let z1 = Complex::from_polar(1.0, -w);
        let z2 = z1 * z1;
        (self.b0 + self.b1 * z1 + self.b2 * z2) / (1.0 + self.a1 * z1 + self.a2 * z2)
    }

    /// Filters one sample, with `state` the filter's two delays. The state is cleared only
    /// when both delays are tiny: clearing one alone would kick a resonant filter into a
    /// limit cycle around the threshold.
    fn run(&self, state: &mut [f64; 2], x: f64) -> f64 {
        let y = self.b0 * x + state[0];
        let first = self.b1 * x - self.a1 * y + state[1];
        let second = self.b2 * x - self.a2 * y;
        *state = if first.abs() < TINY && second.abs() < TINY {
            [0.0; 2]
        } else {
            [first, second]
        };
        y
    }
}

/// Filters in series, each with its own state.
#[derive(Clone, Debug)]
struct Chain {
    filters: Vec<Biquad>,
    states: Vec<[f64; 2]>,
}

impl Chain {
    fn new(filters: Vec<Biquad>) -> Self {
        let states = vec![[0.0; 2]; filters.len()];
        Chain { filters, states }
    }

    fn run(&mut self, x: f64) -> f64 {
        self.filters
            .iter()
            .zip(self.states.iter_mut())
            .fold(x, |x, (filter, state)| filter.run(state, x))
    }
}

/// The largest absolute value over the last `window` values pushed (the current one
/// included), with a monotonic queue.
struct SlidingMax {
    window: usize,
    queue: std::collections::VecDeque<(usize, f64)>,
    count: usize,
}

impl SlidingMax {
    fn new(window: usize) -> Self {
        SlidingMax {
            window: window.max(1),
            queue: std::collections::VecDeque::new(),
            count: 0,
        }
    }

    fn push(&mut self, x: f64) -> f64 {
        let x = x.abs();
        while self.queue.back().is_some_and(|&(_, v)| v <= x) {
            self.queue.pop_back();
        }
        self.queue.push_back((self.count, x));
        self.count += 1;
        while self
            .queue
            .front()
            .is_some_and(|&(i, _)| i + self.window < self.count)
        {
            self.queue.pop_front();
        }
        self.queue.front().map_or(0.0, |&(_, v)| v)
    }
}

/// A saturator driven relative to the signal's recent peak: `e * tanh(k x / e) / tanh(k)`,
/// with `e` the peak over the last [`SATURATION_PEAK_SECONDS`], rising at once and falling
/// smoothly. The curve is the same at any recording level, it squashes the waveform towards
/// its peak, and since `|x| <= e` the output never rises above the recent peak. It tends to
/// the identity as `k` falls to 0.
struct Saturator {
    k: f64,
    norm: f64,
    peak: SlidingMax,
    release: f64,
    envelope: f64,
}

/// Window of the saturator's peak, and the time constant of its fall.
const SATURATION_PEAK_SECONDS: f64 = 0.03;
const SATURATION_RELEASE_SECONDS: f64 = 0.01;

impl Saturator {
    fn new(rate: f64, k: f64) -> Self {
        Saturator {
            k,
            norm: if k > 1e-6 { 1.0 / k.tanh() } else { 0.0 },
            peak: SlidingMax::new(seconds(SATURATION_PEAK_SECONDS, rate)),
            release: (-1.0 / (SATURATION_RELEASE_SECONDS * rate)).exp(),
            envelope: 0.0,
        }
    }

    fn run(&mut self, x: f64) -> f64 {
        let peak = self.peak.push(x);
        self.envelope = flush(peak.max(self.release * self.envelope + (1.0 - self.release) * peak));
        if self.norm == 0.0 || self.envelope <= TINY {
            return x;
        }
        self.envelope * (self.k * x / self.envelope).tanh() * self.norm
    }
}
