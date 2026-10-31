//! Filters that change a voice's timbre: robot, telephone, megaphone, broken microphone and
//! whisper. Each keeps at most a few hundred milliseconds of memory, and whatever varies
//! over time (the robot's carrier, cut-outs, crackle, the whisper's noise) is placed by the
//! frame's position in the stream.

use std::f64::consts::TAU;

use super::Complex;

use super::{
    BUTTERWORTH_4, BUTTERWORTH_6, Biquad, Chain, Saturator, SlidingMax, deinterleave, flush, input,
    mix_gains, sample, seconds, signed, uniform,
};

/// Time after which the band filters of the telephone and megaphone have forgotten their
/// past to well under 1e-7 (the slowest pole, at 300 Hz with Q 1.9, decays by e every 2 ms,
/// and the saturator's peak by e every 10 ms after its 30 ms window).
pub(super) const FILTER_SETTLE_SECONDS: f64 = 0.25;

// Robot: ring modulation, then a comb tuned to the carrier.

/// Gain of the comb's delayed copy at full depth.
const ROBOT_COMB: f64 = 0.5;

/// The comb's delay: one period of the carrier, in frames.
pub(super) fn robot_period(rate: f64, frequency: f64) -> usize {
    ((rate / frequency).round() as usize).max(1)
}

/// `y = x (1 - d + d cos(2 pi f n / rate))`, then `(y[n] + g y[n - P]) / (1 + g)` with `P`
/// one carrier period and `g = 0.5 d`. Neither stage is louder than its input.
pub(super) fn robot(
    samples: &mut [f32],
    channels: usize,
    rate: f64,
    start: i64,
    frequency: f64,
    depth: f64,
) {
    if depth <= 0.0 {
        return;
    }
    let frames = samples.len() / channels;
    let period = robot_period(rate, frequency);
    let comb = ROBOT_COMB * depth;
    let carrier: Vec<f64> = (0..frames)
        .map(|i| {
            let n = start + i as i64;
            let phase = (n as f64 * frequency / rate).fract();
            1.0 - depth + depth * (TAU * phase).cos()
        })
        .collect();
    for c in 0..channels {
        let ring: Vec<f64> = deinterleave(samples, channels, c)
            .iter()
            .zip(&carrier)
            .map(|(x, m)| x * m)
            .collect();
        for (i, y) in ring.iter().enumerate() {
            let delayed = if i >= period { ring[i - period] } else { 0.0 };
            samples[i * channels + c] = sample((y + comb * delayed) / (1.0 + comb));
        }
    }
}

/// The quality of a critically damped section, whose impulse response is never negative.
const CRITICAL_Q: f64 = 0.5;
/// The softening low-pass after a saturator sits this far above the band's top.
const SOFTEN_ABOVE: f64 = 1.5;

// Telephone

/// The telephone band at `narrow` 0 and 1.
const PHONE_LOW_HZ: f64 = 300.0;
const PHONE_HIGH_HZ: f64 = 3400.0;

/// Sixth-order Butterworth high- and low-pass edges, then the saturator, then a critically
/// damped low-pass (`Q = 0.5`, a double real pole) half an octave above the band that softens
/// the distortion's harmonics. Its impulse response is never negative and sums to 1, so it
/// averages and cannot ring above the squashed peaks.
pub(super) fn telephone(samples: &mut [f32], channels: usize, rate: f64, narrow: f64, crunch: f64) {
    let low = PHONE_LOW_HZ * 2f64.powf(narrow);
    let high = PHONE_HIGH_HZ * 2f64.powf(-0.5 * narrow);
    let before = Chain::new(
        BUTTERWORTH_6
            .iter()
            .flat_map(|q| {
                [
                    Biquad::high_pass(rate, low, *q),
                    Biquad::low_pass(rate, high, *q),
                ]
            })
            .collect(),
    );
    let after = Chain::new(vec![Biquad::low_pass(
        rate,
        high * SOFTEN_ABOVE,
        CRITICAL_Q,
    )]);
    for c in 0..channels {
        let (mut before, mut after) = (before.clone(), after.clone());
        let mut saturator = Saturator::new(rate, 3.0 * crunch);
        for frame in samples.chunks_exact_mut(channels) {
            let y = saturator.run(before.run(input(frame[c])));
            frame[c] = sample(after.run(y));
        }
    }
}

// Megaphone

/// The megaphone's band, its horn resonance and its comb.
const HORN_LOW_HZ: f64 = 450.0;
const HORN_HIGH_HZ: f64 = 4200.0;
const HORN_HZ: f64 = 1800.0;
const HORN_Q: f64 = 1.0;
const HORN_COMB_SECONDS: f64 = 0.0011;
const HORN_COMB_FEEDBACK: f64 = 0.35;

/// Fourth-order band edges, half the signal through a band-pass at the horn's resonance (so
/// the response peaks at 1 there and is 0.5 to 1 elsewhere), the saturator, a feedback comb
/// `y[n] = (1 - g) v[n] + g y[n - D]` of about 1 ms, and a critically damped low-pass. The comb
/// and the low-pass both have impulse responses that are never negative and sum to 1, so
/// neither lifts the squashed peaks.
pub(super) fn megaphone(samples: &mut [f32], channels: usize, rate: f64, drive: f64) {
    let band = Chain::new(vec![
        Biquad::high_pass(rate, HORN_LOW_HZ, BUTTERWORTH_4[0]),
        Biquad::high_pass(rate, HORN_LOW_HZ, BUTTERWORTH_4[1]),
        Biquad::low_pass(rate, HORN_HIGH_HZ, BUTTERWORTH_4[0]),
        Biquad::low_pass(rate, HORN_HIGH_HZ, BUTTERWORTH_4[1]),
    ]);
    let horn = Chain::new(vec![Biquad::band_pass(rate, HORN_HZ, HORN_Q)]);
    let soften = Chain::new(vec![Biquad::low_pass(
        rate,
        HORN_HIGH_HZ * SOFTEN_ABOVE,
        CRITICAL_Q,
    )]);
    let delay = seconds(HORN_COMB_SECONDS, rate).max(1);
    let frames = samples.len() / channels;
    for c in 0..channels {
        let (mut band, mut horn, mut soften) = (band.clone(), horn.clone(), soften.clone());
        let mut saturator = Saturator::new(rate, 7.0 * drive);
        let mut comb = vec![0.0f64; frames];
        for i in 0..frames {
            let at = i * channels + c;
            let y = band.run(input(samples[at]));
            let u = 0.5 * (y + horn.run(y));
            let v = saturator.run(u);
            let back = if i >= delay { comb[i - delay] } else { 0.0 };
            comb[i] = flush((1.0 - HORN_COMB_FEEDBACK) * v + HORN_COMB_FEEDBACK * back);
            samples[at] = sample(soften.run(comb[i]));
        }
    }
}

// Broken microphone

/// What the broken microphone remembers: its 30 ms peak and its crackle's decay.
pub(super) const BROKEN_MEMORY_SECONDS: f64 = 0.1;
/// Window of the peak that sets the quantizer's step and the crackle's level.
const BROKEN_PEAK_SECONDS: f64 = 0.03;
/// Length of the stretches that may cut out, and the ramp into and out of a cut.
const DROPOUT_SECONDS: f64 = 0.05;
const DROPOUT_RAMP_SECONDS: f64 = 0.001;
/// Chance that a stretch cuts out at amount 1, and the most a cut lets through.
const DROPOUT_CHANCE: f64 = 0.3;
const DROPOUT_LEVEL: f64 = 0.08;
/// Crackles per second at amount 1, and the time constant of each one's decay.
const CRACKLES_PER_SECOND: f64 = 60.0;
const CRACKLE_SECONDS: f64 = 0.000_25;
/// Level at which a crackle has ended.
const CRACKLE_END: f64 = 1e-3;
/// Share of the output given to crackle at amount 1.
const CRACKLE_SHARE: f64 = 0.35;
/// Seeds of the hashed decisions.
const SEED_DROP: u64 = 1;
const SEED_DROP_LEVEL: u64 = 2;
const SEED_CRACKLE: u64 = 3;
const SEED_CRACKLE_LEVEL: u64 = 4;
const SEED_CRACKLE_NOISE: u64 = 5;

/// The gain of the dropout stretch `index` at `amount`.
fn dropout_gain(index: i64, amount: f64) -> f64 {
    if uniform(index, SEED_DROP) < DROPOUT_CHANCE * amount {
        DROPOUT_LEVEL * uniform(index, SEED_DROP_LEVEL)
    } else {
        1.0
    }
}

/// Sample-and-hold to a lower rate on a grid of stream positions, truncation towards zero to
/// a step set relative to the recent peak (16 bits at amount 0, 4 at 1), stretches of 50 ms
/// that cut out by chance, and crackle: bursts of noise at hashed positions, scaled by the
/// recent peak. Truncation never enlarges a sample and the crackle shares the output with
/// the signal, so nothing passes the recent peak.
pub(super) fn broken(samples: &mut [f32], channels: usize, rate: f64, start: i64, amount: f64) {
    if amount <= 0.0 {
        return;
    }
    let frames = samples.len() / channels;
    let target = 22_050.0 * (1.0 - amount) + 4_000.0 * amount;
    let hold = ((rate / target).round() as i64).max(1);
    let bits = 16.0 - 12.0 * amount;
    let step = 2f64.powf(1.0 - bits);
    let stretch = (seconds(DROPOUT_SECONDS, rate) as i64).max(1);
    let ramp = (seconds(DROPOUT_RAMP_SECONDS, rate) as i64).clamp(1, stretch);
    let crackle_chance = CRACKLES_PER_SECOND * amount / rate;
    let decay = (-1.0 / (CRACKLE_SECONDS * rate)).exp();
    let share = CRACKLE_SHARE * amount;

    // The recent peak over all channels, and the crackle's envelope, per frame.
    let mut peak = SlidingMax::new(seconds(BROKEN_PEAK_SECONDS, rate));
    let mut burst = 0.0f64;
    let mut levels = Vec::with_capacity(frames);
    let mut crackle = Vec::with_capacity(frames);
    let mut gains = Vec::with_capacity(frames);
    for (i, frame) in samples.chunks_exact(channels).enumerate() {
        let n = start + i as i64;
        let loudest = frame.iter().map(|v| input(*v).abs()).fold(0.0, f64::max);
        levels.push(peak.push(loudest));
        let click = if uniform(n, SEED_CRACKLE) < crackle_chance {
            0.4 + 0.6 * uniform(n, SEED_CRACKLE_LEVEL)
        } else {
            0.0
        };
        // A crackle lasts until it has fallen by 60 dB.
        burst = (decay * burst).max(click);
        if burst < CRACKLE_END {
            burst = 0.0;
        }
        crackle.push(burst * signed(n, SEED_CRACKLE_NOISE));
        let index = n.div_euclid(stretch);
        let into = n - index * stretch;
        let gain = dropout_gain(index, amount);
        gains.push(if into < ramp {
            let before = dropout_gain(index - 1, amount);
            before + (gain - before) * (into as f64 / ramp as f64)
        } else {
            gain
        });
    }
    let original = samples.to_vec();
    for i in 0..frames {
        let n = start + i as i64;
        // The held frame: the first of its group of `hold` on the stream's grid.
        let held = i as i64 - (n - n.div_euclid(hold) * hold);
        let q = levels[i] * step;
        for c in 0..channels {
            let x = if held >= 0 {
                input(original[held as usize * channels + c])
            } else {
                0.0
            };
            let truncated = if q > 0.0 {
                x.signum() * (x.abs() / q).floor() * q
            } else {
                0.0
            };
            let signal = gains[i] * truncated;
            samples[i * channels + c] =
                sample((1.0 - share) * signal + share * levels[i] * crackle[i]);
        }
    }
}

// Whisper: a channel vocoder with a noise carrier.

/// What the whisper remembers: its slowest band filter rings for about 8 ms per e.
pub(super) const WHISPER_MEMORY_SECONDS: f64 = 0.3;
/// Bands, from the lowest centre to the highest (or 0.42 of the sample rate).
const WHISPER_BANDS: usize = 16;
const WHISPER_LOW_HZ: f64 = 160.0;
const WHISPER_HIGH_HZ: f64 = 7500.0;
/// Time constant of each band's envelope.
const WHISPER_ENVELOPE_SECONDS: f64 = 0.005;
/// Seed of the noise carrier.
const SEED_BREATH: u64 = 6;

/// The vocoder's gains, from the bands' frequency responses sampled on a grid of
/// [`RESPONSE_POINTS`]: for each band, the factor that gives its noise unit RMS (noise uniform
/// in -1 to 1 has variance 1/3, and by Parseval a filter passes `mean |H|^2` of white noise's
/// power); and one output level that undoes the bands' overlap. A sine of power `P` gives band
/// `b` the level `|H_b|^2 P`, and the bands' noises are correlated where the bands overlap,
/// so the whisper has power `P * sum over b, b' of |H_b| |H_b'| rho(b, b')`; the level is the
/// inverse root of that sum's mean over 64 frequencies spread evenly in pitch across the
/// bands, so a voice keeps its level.
fn whisper_gains(filters: &[Biquad], rate: f64) -> (Vec<f64>, f64) {
    let responses: Vec<Vec<Complex>> = filters
        .iter()
        .map(|f| {
            (0..RESPONSE_POINTS)
                .map(|m| {
                    f.response(std::f64::consts::PI * (m as f64 + 0.5) / RESPONSE_POINTS as f64)
                })
                .collect()
        })
        .collect();
    let energy: Vec<f64> = responses
        .iter()
        .map(|h| h.iter().map(Complex::norm_sqr).sum::<f64>() / RESPONSE_POINTS as f64)
        .collect();
    let norms = energy.iter().map(|e| 1.0 / (e / 3.0).sqrt()).collect();
    let bands = filters.len();
    let mut rho = vec![0.0f64; bands * bands];
    for b in 0..bands {
        for c in 0..bands {
            let cross: f64 = responses[b]
                .iter()
                .zip(&responses[c])
                .map(|(x, y)| x.re * y.re + x.im * y.im)
                .sum::<f64>()
                / RESPONSE_POINTS as f64;
            rho[b * bands + c] = cross / (energy[b] * energy[c]).sqrt();
        }
    }
    let high = WHISPER_HIGH_HZ.min(0.42 * rate);
    let mut mean = 0.0;
    for t in 0..64 {
        let hz = WHISPER_LOW_HZ * (high / WHISPER_LOW_HZ).powf(f64::from(t) / 63.0);
        let w = 2.0 * std::f64::consts::PI * hz / rate;
        let magnitude: Vec<f64> = filters.iter().map(|f| f.response(w).norm()).collect();
        let mut sum = 0.0;
        for b in 0..bands {
            for c in 0..bands {
                sum += magnitude[b] * magnitude[c] * rho[b * bands + c];
            }
        }
        mean += sum / 64.0;
    }
    (norms, if mean > 0.0 { 1.0 / mean.sqrt() } else { 1.0 })
}

/// Points of the frequency grid [`whisper_gains`] samples the bands on.
const RESPONSE_POINTS: usize = 2048;

/// Splits the input into 16 bands with band-passes, follows each band's RMS level, and
/// gives that level to the same band of white noise normalised to unit RMS; the bands of
/// noise sum to the whisper, blended with the input by `amount` at equal power. The noise
/// is the same in
/// every channel, so a centred voice stays centred.
pub(super) fn whisper(samples: &mut [f32], channels: usize, rate: f64, start: i64, amount: f64) {
    if amount <= 0.0 {
        return;
    }
    let frames = samples.len() / channels;
    let high = WHISPER_HIGH_HZ.min(0.42 * rate);
    let ratio = (high / WHISPER_LOW_HZ).powf(1.0 / (WHISPER_BANDS - 1) as f64);
    let q = 1.0 / (ratio.sqrt() - 1.0 / ratio.sqrt());
    let filters: Vec<Biquad> = (0..WHISPER_BANDS)
        .map(|b| Biquad::band_pass(rate, WHISPER_LOW_HZ * ratio.powi(b as i32), q))
        .collect();
    let (norms, level) = whisper_gains(&filters, rate);
    let smoothing = 1.0 - (-1.0 / (WHISPER_ENVELOPE_SECONDS * rate)).exp();
    let (dry, wet) = mix_gains(amount);

    // The bands of noise, shared by every channel.
    let mut noise_states = vec![[0.0f64; 2]; WHISPER_BANDS];
    let mut breath = vec![0.0f64; frames * WHISPER_BANDS];
    for i in 0..frames {
        let white = signed(start + i as i64, SEED_BREATH);
        for (b, (filter, state)) in filters.iter().zip(noise_states.iter_mut()).enumerate() {
            breath[i * WHISPER_BANDS + b] = filter.run(state, white) * norms[b];
        }
    }
    for c in 0..channels {
        let mut states = vec![[0.0f64; 2]; WHISPER_BANDS];
        let mut power = [0.0f64; WHISPER_BANDS];
        for i in 0..frames {
            let at = i * channels + c;
            let x = input(samples[at]);
            let mut out = 0.0;
            for (b, (filter, state)) in filters.iter().zip(states.iter_mut()).enumerate() {
                let y = filter.run(state, x);
                power[b] = flush(power[b] + smoothing * (y * y - power[b]));
                out += power[b].sqrt() * breath[i * WHISPER_BANDS + b];
            }
            samples[at] = sample(dry * x + wet * level * out);
        }
    }
}
