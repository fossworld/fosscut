//! Filters that put space around a voice: echo, chorus and reverb. All three are finite:
//! echo repeats and the reverb's impulse response end within [`TAIL_SECONDS`], and the
//! chorus reads at most 30 ms back, so a window of a clip fed that much input before it
//! sounds exactly as the whole clip does.

use std::f64::consts::{PI, TAU};
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::{Biquad, TAIL_SECONDS, cubic, deinterleave, flush, mix_gains, sample, seconds};

// Echo

/// Repeats quieter than this (-80 dB) are left out.
const ECHO_FLOOR: f64 = 1e-4;

/// The echo's taps, as (frames after the sound, gain): one every `delay` seconds, each
/// `feedback` times the one before, faded out by a raised cosine over the second half of
/// [`TAIL_SECONDS`] (the first repeat is never faded).
pub(super) fn echo_taps(rate: f64, delay: f64, feedback: f64) -> Vec<(usize, f64)> {
    let step = ((delay * rate).round() as usize).max(1);
    let horizon = seconds(TAIL_SECONDS, rate);
    let mut taps = Vec::new();
    let mut gain = 1.0;
    let mut at = step;
    while at <= horizon {
        let t = at as f64 / horizon as f64;
        let fade = if taps.is_empty() || t <= 0.5 {
            1.0
        } else {
            0.5 + 0.5 * (PI * (t - 0.5) / 0.5).cos()
        };
        let level = gain * fade;
        if level < ECHO_FLOOR {
            break;
        }
        taps.push((at, level));
        gain *= feedback;
        at += step;
    }
    taps
}

/// The dry sound and the sum of `g_k x[n - d_k]` over the taps of [`echo_taps`], blended
/// at equal power by `mix`.
pub(super) fn echo(
    samples: &mut [f32],
    channels: usize,
    rate: f64,
    delay: f64,
    feedback: f64,
    mix: f64,
) {
    if mix <= 0.0 {
        return;
    }
    let taps = echo_taps(rate, delay, feedback);
    let (dry_gain, wet_gain) = mix_gains(mix);
    for c in 0..channels {
        let x = deinterleave(samples, channels, c);
        for (i, dry) in x.iter().enumerate() {
            let wet: f64 = taps
                .iter()
                .take_while(|(at, _)| *at <= i)
                .map(|(at, gain)| gain * x[i - at])
                .sum();
            samples[i * channels + c] = sample(dry_gain * dry + wet_gain * wet);
        }
    }
}

// Chorus

/// The chorus reads at most this far back: the longest delay, its swing and the
/// interpolation's reach.
pub(super) const CHORUS_MEMORY_SECONDS: f64 = 0.03;
/// Base delays of the three voices.
const CHORUS_DELAYS: [f64; 3] = [0.0107, 0.0153, 0.0211];
/// How far a voice's delay swings at depth 1.
const CHORUS_SWING: f64 = 0.006;

/// Three copies read from delay lines whose delays swing between their base and base plus
/// `depth * 6 ms` with sine oscillators a third of a cycle apart (a quarter cycle further on
/// in each next channel, which widens a centred voice), averaged, and blended by `mix` at
/// equal power. The
/// oscillators' phase comes from the frame's position in the stream.
pub(super) fn chorus(
    samples: &mut [f32],
    channels: usize,
    rate: f64,
    start: i64,
    hz: f64,
    depth: f64,
    mix: f64,
) {
    if mix <= 0.0 {
        return;
    }
    let swing = CHORUS_SWING * depth * rate;
    let (dry_gain, wet_gain) = mix_gains(mix);
    for c in 0..channels {
        let x = deinterleave(samples, channels, c);
        for (i, dry) in x.iter().enumerate() {
            let n = start + i as i64;
            let cycle = (n as f64 * hz / rate).fract();
            let wet: f64 = CHORUS_DELAYS
                .iter()
                .enumerate()
                .map(|(v, base)| {
                    let phase = cycle + v as f64 / 3.0 + 0.25 * c as f64;
                    let lag = base * rate + swing * 0.5 * (1.0 + (TAU * phase).sin());
                    cubic(&x, i as f64 - lag)
                })
                .sum::<f64>()
                / CHORUS_DELAYS.len() as f64;
            samples[i * channels + c] = sample(dry_gain * dry + wet_gain * wet);
        }
    }
}

// Reverb: a feedback delay network rendered into a finite impulse response.

/// Delay lines of the network at room size 0.5, in seconds (mutually prime in samples at
/// common rates, spread over a factor of 2.5).
const LINES: [f64; 8] = [
    0.029_3, 0.034_7, 0.039_1, 0.044_9, 0.051_3, 0.057_7, 0.063_1, 0.071_9,
];
/// Allpass diffusers of the input, in seconds at room size 0.5, and their gain.
const DIFFUSERS: [f64; 3] = [0.005_3, 0.003_7, 0.001_9];
const DIFFUSION: f64 = 0.6;
/// Signs of each line in the left and right outputs: two rows of a Hadamard matrix, so the
/// two channels are decorrelated.
const LEFT: [f64; 8] = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
const RIGHT: [f64; 8] = [1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];
/// Gain of the early reflections, read a third of the way along each line.
const EARLY: f64 = 0.5;
/// The impulse response is faded out by a raised cosine from this share of its length.
const FADE_FROM: f64 = 0.55;
/// Cut below this frequency in the reverberation, so low notes do not boom.
const REVERB_HIGH_PASS_HZ: f64 = 90.0;

/// Reverberation time at low frequencies (seconds to fall by 60 dB) at `room`.
pub(super) fn reverb_time(room: f64) -> f64 {
    0.3 + 2.7 * room * room
}

/// The fast Walsh-Hadamard transform of eight values, scaled by 1 / sqrt(8) so that it is
/// orthogonal (the network's lossless feedback matrix).
fn hadamard(v: &mut [f64; 8]) {
    let mut width = 1;
    while width < 8 {
        for block in (0..8).step_by(2 * width) {
            for i in block..block + width {
                let (a, b) = (v[i], v[i + width]);
                v[i] = a + b;
                v[i + width] = a - b;
            }
        }
        width *= 2;
    }
    let scale = 1.0 / 8f64.sqrt();
    for x in v.iter_mut() {
        *x *= scale;
    }
}

/// The left and right impulse responses for `room` and `damping` at `rate`, each
/// [`TAIL_SECONDS`] long with unit energy.
///
/// An impulse passes three Schroeder allpasses (`y[n] = -g x[n] + x[n - d] + g y[n - d]`)
/// and feeds eight delay lines whose outputs are mixed by an orthogonal Hadamard matrix and
/// fed back; each line is also tapped a third of the way along for early reflections. Each line's output passes a one-pole low-pass whose gains at 0 Hz and at the
/// Nyquist frequency make that line decay by 60 dB in the low and the high reverberation
/// times (Jot and Chaigne, 1991): `g = 10^(-3 m / (rate T))` for a line of `m` frames. The
/// room scales the lines (0.35 to 1.5 times [`LINES`]), the low reverberation time (0.3 to
/// 3 seconds) and the pre-delay (4 to 34 ms); damping shortens the high reverberation time to
/// as little as a fifth of the low one.
pub(super) fn impulse_response(rate: f64, room: f64, damping: f64) -> [Vec<f64>; 2] {
    let length = seconds(TAIL_SECONDS, rate);
    let size = 0.35 + 1.15 * room;
    let low_time = reverb_time(room);
    let high_time = low_time * (1.0 - 0.8 * damping);
    let predelay = seconds(0.004 + 0.03 * room, rate).min(length);

    let lengths: Vec<usize> = LINES
        .iter()
        .map(|s| ((s * size * rate).round() as usize).max(2))
        .collect();
    let absorption: Vec<(f64, f64)> = lengths
        .iter()
        .map(|m| {
            let gain = |time: f64| 10f64.powf(-3.0 * *m as f64 / (rate * time));
            let (dc, nyquist) = (gain(low_time), gain(high_time));
            // A one-pole `y = g (1 - p) x + p y[-1]` has gain g at 0 Hz and
            // g (1 - p) / (1 + p) at the Nyquist frequency.
            let r = nyquist / dc;
            let pole = (1.0 - r) / (1.0 + r);
            (dc * (1.0 - pole), pole)
        })
        .collect();
    let mut lines: Vec<Vec<f64>> = lengths.iter().map(|m| vec![0.0; *m]).collect();
    let mut filtered = [0.0f64; 8];
    let diffusers: Vec<usize> = DIFFUSERS
        .iter()
        .map(|s| ((s * (0.5 + room) * rate).round() as usize).max(1))
        .collect();
    let mut diffuser_lines: Vec<Vec<(f64, f64)>> =
        diffusers.iter().map(|d| vec![(0.0, 0.0); *d]).collect();

    let mut left = vec![0.0f64; length];
    let mut right = vec![0.0f64; length];
    for n in 0..length - predelay {
        // The input: the impulse through the diffusers.
        let mut u = if n == 0 { 1.0 } else { 0.0 };
        for (line, d) in diffuser_lines.iter_mut().zip(&diffusers) {
            let slot = n % d;
            let (x_old, y_old) = line[slot];
            let y = -DIFFUSION * u + x_old + DIFFUSION * y_old;
            line[slot] = (u, flush(y));
            u = y;
        }
        let (mut l, mut r) = (0.0, 0.0);
        for (i, line) in lines.iter().enumerate() {
            // Early reflections: each line tapped a third of the way along.
            let early = line[(n + lengths[i] - lengths[i] / 3) % lengths[i]];
            l += EARLY * LEFT[i] * early;
            r += EARLY * RIGHT[(i + 3) % 8] * early;
            let out = line[n % lengths[i]];
            let (gain, pole) = absorption[i];
            filtered[i] = flush(gain * out + pole * filtered[i]);
            l += LEFT[i] * filtered[i];
            r += RIGHT[i] * filtered[i];
        }
        let mut feedback = filtered;
        hadamard(&mut feedback);
        for (i, line) in lines.iter_mut().enumerate() {
            line[n % lengths[i]] = feedback[i] + u;
        }
        left[n + predelay] = l;
        right[n + predelay] = r;
    }
    let fade_from = (FADE_FROM * length as f64) as usize;
    let mut out = [left, right];
    for ir in &mut out {
        let mut state = [0.0; 2];
        let cut = Biquad::high_pass(rate, REVERB_HIGH_PASS_HZ, std::f64::consts::FRAC_1_SQRT_2);
        for (n, v) in ir.iter_mut().enumerate() {
            let fade = if n < fade_from {
                1.0
            } else {
                let t = (n - fade_from) as f64 / (length - fade_from) as f64;
                0.5 + 0.5 * (PI * t).cos()
            };
            *v = cut.run(&mut state, *v) * fade;
        }
        let energy: f64 = ir.iter().map(|v| v * v).sum();
        if energy > 0.0 {
            let scale = 1.0 / energy.sqrt();
            for v in ir.iter_mut() {
                *v = flush(*v * scale);
            }
        }
    }
    out
}

/// Convolution with a fixed impulse response, uniformly partitioned (overlap-add of blocks
/// of `partition` frames, transforms of twice that).
struct Convolver {
    partition: usize,
    spectra: Vec<Vec<Complex<f64>>>,
    forward: Arc<dyn Fft<f64>>,
    inverse: Arc<dyn Fft<f64>>,
}

impl Convolver {
    fn new(ir: &[f64], rate: f64, planner: &mut FftPlanner<f64>) -> Self {
        let partition = ((0.04 * rate).ceil() as usize)
            .next_power_of_two()
            .clamp(256, 8192);
        let size = 2 * partition;
        let forward = planner.plan_fft_forward(size);
        let inverse = planner.plan_fft_inverse(size);
        let spectra = ir
            .chunks(partition)
            .map(|chunk| {
                let mut buffer = vec![Complex::default(); size];
                for (z, v) in buffer.iter_mut().zip(chunk) {
                    z.re = *v;
                }
                forward.process(&mut buffer);
                buffer
            })
            .collect();
        Convolver {
            partition,
            spectra,
            forward,
            inverse,
        }
    }

    /// `x` convolved with the response, as long as `x` (the tail past its end is dropped).
    fn run(&self, x: &[f64]) -> Vec<f64> {
        let (partition, size) = (self.partition, 2 * self.partition);
        let blocks: Vec<Vec<Complex<f64>>> = x
            .chunks(partition)
            .map(|chunk| {
                let mut buffer = vec![Complex::default(); size];
                for (z, v) in buffer.iter_mut().zip(chunk) {
                    z.re = *v;
                }
                self.forward.process(&mut buffer);
                buffer
            })
            .collect();
        let mut out = vec![0.0f64; (blocks.len() + 1) * partition];
        let mut sum = vec![Complex::default(); size];
        let scale = 1.0 / size as f64;
        for j in 0..blocks.len() {
            sum.fill(Complex::default());
            for (i, response) in self.spectra.iter().enumerate().take(j + 1) {
                for ((s, a), b) in sum.iter_mut().zip(&blocks[j - i]).zip(response) {
                    *s += a * b;
                }
            }
            self.inverse.process(&mut sum);
            for (o, z) in out[j * partition..j * partition + size]
                .iter_mut()
                .zip(&sum)
            {
                *o += z.re * scale;
            }
        }
        out.truncate(x.len());
        out
    }
}

/// Convolves each channel with the left or right impulse response (alternately, so a mono
/// clip takes the left one) and blends by `mix` at equal power. The two responses and their
/// transforms depend only on the sample rate, room and damping, and are kept for the next
/// call (the mixer processes a clip block by block).
pub(super) fn reverb(
    samples: &mut [f32],
    channels: usize,
    sample_rate: u32,
    room: f64,
    damping: f64,
    mix: f64,
) {
    if mix <= 0.0 {
        return;
    }
    let convolvers = convolvers(sample_rate, room, damping);
    let (dry_gain, wet_gain) = mix_gains(mix);
    for c in 0..channels {
        let x = deinterleave(samples, channels, c);
        let wet = convolvers[c % 2].run(&x);
        for (i, (dry, wet)) in x.iter().zip(&wet).enumerate() {
            samples[i * channels + c] = sample(dry_gain * dry + wet_gain * wet);
        }
    }
}

/// Reverbs kept between calls, the most recently used first.
type Kept = Vec<((u32, u64, u64), Arc<[Convolver; 2]>)>;
static KEPT: std::sync::Mutex<Kept> = std::sync::Mutex::new(Vec::new());
/// How many reverb settings are kept.
const KEEP: usize = 4;

/// The convolvers of the left and right responses for these settings, from the kept ones
/// when they are there. Building them is deterministic, so a kept pair is the same as a new
/// one.
fn convolvers(sample_rate: u32, room: f64, damping: f64) -> Arc<[Convolver; 2]> {
    let key = (sample_rate, room.to_bits(), damping.to_bits());
    if let Ok(mut kept) = KEPT.lock()
        && let Some(at) = kept.iter().position(|(k, _)| *k == key)
    {
        let entry = kept.remove(at);
        let found = Arc::clone(&entry.1);
        kept.insert(0, entry);
        return found;
    }
    let rate = f64::from(sample_rate);
    let responses = impulse_response(rate, room, damping);
    let mut planner = FftPlanner::new();
    let built = Arc::new([
        Convolver::new(&responses[0], rate, &mut planner),
        Convolver::new(&responses[1], rate, &mut planner),
    ]);
    if let Ok(mut kept) = KEPT.lock() {
        kept.insert(0, (key, Arc::clone(&built)));
        kept.truncate(KEEP);
    }
    built
}
