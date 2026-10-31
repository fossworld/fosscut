//! Audio effects on clips: noise reduction, voice enhancement, isolating or removing the
//! centre of a stereo mix, a limiter, and the voice filters of [`crate::voice`].
//!
//! The effects are pure functions of their input: [`process`] runs a chain over an
//! interleaved buffer from a fresh state and returns a buffer of the same length, aligned
//! with the input. Nothing allocates per sample.
//!
//! The mixer applies effects to a window of a clip without processing the clip from its
//! start. It feeds up to [`WARM_UP_SECONDS`] of audio before the wanted range, so that
//! noise estimates, envelopes and filters settle, and [`lookahead_frames`] after it (the
//! effects' latency, compensated inside), and keeps the wanted part. The output at a frame
//! depends only on the input in that window around it, up to a small tolerance from state
//! that has not fully settled. The denoiser works on frames of audio placed on a fixed grid;
//! [`process_at`] anchors that grid at a stream position, so that windows cut anywhere from
//! the same stream line up exactly.
//!
//! Non-finite input samples are treated as silence by every effect that changes the audio.

use std::f64::consts::{PI, TAU};
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// Which vocal part [`AudioEffect::Vocals`] keeps.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VocalMode {
    /// Keeps what is panned to the centre, band-limited to the voice range.
    Isolate,
    /// Removes what is panned to the centre and keeps the rest.
    Remove,
}

/// One audio effect with its static parameters.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioEffect {
    /// Reduces steady background noise (hiss, hum, room tone); `amount` 0 to 1.
    ///
    /// Spectral suppression with a noise estimate from minimum statistics: whatever stays
    /// steady in a frequency band for longer than about 1.4 seconds counts as noise, so a
    /// sustained tone held that long (a hum, but also a long sung note) fades out too.
    /// `amount` sets how far noise is lowered, from not at all (0) to 24 dB (1).
    Denoise {
        /// How strongly noise is reduced, 0 (off) to 1; NaN counts as 0.
        amount: f32,
    },
    /// Makes speech clearer: rumble cut, mud cut, presence lift, gentle compression with
    /// makeup gain; `amount` 0 to 1.
    ///
    /// At amount 1: a high-pass at 80 Hz, -3 dB around 300 Hz, +4 dB around 3 kHz, then 3:1
    /// compression above an RMS level of -20 dB (soft knee) with makeup gain that keeps
    /// speech at -16 dB RMS as loud as it was. Lower amounts scale the cutoff, the gains and
    /// the ratio towards no change.
    EnhanceVoice {
        /// How strongly the voice is shaped, 0 (off) to 1; NaN counts as 0.
        amount: f32,
    },
    /// Keeps or removes what is panned to the centre of a stereo mix (usually the lead
    /// vocal), the classical mid/side method; mono input is returned unchanged.
    ///
    /// The mid signal is `(L + R) / 2` and the side signal `(L - R) / 2`. Removing returns
    /// the side signal (`L = side`, `R = -side`), so the centre cancels; isolating returns
    /// the mid signal on both channels, band-limited to 120 Hz to 8 kHz. This only works on
    /// stereo mixes whose voice is centred and identical in both channels: reverb, stereo
    /// effects and off-centre panning leave parts of it behind, centred bass and drums go
    /// with it, and a removed mix cancels to silence when summed to mono. Any channel count
    /// other than two is returned unchanged.
    Vocals {
        /// Which part to keep.
        mode: VocalMode,
    },
    /// Raises the level by `gain_db` and keeps every sample peak at or under `ceiling_db`
    /// (dBFS), so a clip can be made louder than its peaks would otherwise allow.
    ///
    /// A look-ahead peak limiter: the gain each sample needs is held at its lowest over the
    /// next 5 ms and averaged over 5 ms, so it falls smoothly before a peak arrives and never
    /// lets one through, then recovers over 80 ms. `gain_db` is clamped to 0 to 24 and
    /// `ceiling_db` to -12 to 0; NaN counts as 0.
    Limiter {
        /// Gain applied before limiting, in dB.
        gain_db: f32,
        /// The highest sample peak let through, in dBFS.
        ceiling_db: f32,
    },
    /// A voice filter (pitch, robot, telephone, megaphone, echo, reverb, chorus, broken
    /// microphone, whisper): see [`crate::voice`].
    Voice(crate::voice::VoiceEffect),
}

/// Seconds of audio before the wanted range that the caller feeds so the effects' state
/// settles (noise estimates, envelopes, filters), at most: see [`warm_up_frames`].
pub const WARM_UP_SECONDS: f64 = 2.0;

/// Frames of audio before the wanted range that `effects` need, at `sample_rate`: the sum of
/// what each effect remembers, at most [`WARM_UP_SECONDS`]. The voice filters remember a
/// bounded stretch and ask for just that; the other effects ask for the whole warm-up.
pub fn warm_up_frames(effects: &[AudioEffect], sample_rate: u32) -> usize {
    let most = (WARM_UP_SECONDS * f64::from(sample_rate)).ceil() as usize;
    effects
        .iter()
        .map(|effect| match effect {
            AudioEffect::Voice(voice) => voice.warm_up_frames(sample_rate),
            _ => most,
        })
        .fold(0usize, usize::saturating_add)
        .min(most)
}

/// Frames of audio after the wanted range the effects need (their latency), at
/// `sample_rate`.
pub fn lookahead_frames(effects: &[AudioEffect], sample_rate: u32) -> usize {
    if sample_rate == 0 {
        return 0;
    }
    effects
        .iter()
        .map(|effect| match *effect {
            AudioEffect::Denoise { amount } if clamp_amount(amount) > 0.0 => {
                let geometry = Geometry::new(sample_rate);
                geometry.len - 1 + geometry.init_frames * geometry.hop
            }
            AudioEffect::Limiter { .. } => limiter_window(sample_rate),
            AudioEffect::Voice(ref voice) => voice.lookahead_frames(sample_rate),
            _ => 0,
        })
        .sum()
}

/// Runs `effects` in order over interleaved `input` (`channels` channels, `sample_rate`
/// Hz) from a fresh state and returns a buffer of the same length, time-aligned with the
/// input (latency compensated internally: treat audio beyond the end of `input` as
/// silence).
///
/// The same as [`process_at`] with the input starting at position 0. Windows of one
/// stream that start at different positions place the denoiser's frames differently, so
/// where their outputs overlap they agree only approximately; [`process_at`] makes them
/// agree to within the warm-up tolerance.
pub fn process(
    effects: &[AudioEffect],
    input: &[f32],
    channels: u16,
    sample_rate: u32,
) -> Vec<f32> {
    process_at(effects, input, channels, sample_rate, 0)
}

/// Like [`process`], for `input` whose first frame is frame `start` of a longer stream
/// (for example the timeline sample of the first frame, or frames since the clip's start;
/// any fixed origin works). The denoiser places its analysis frames on a grid anchored at
/// frame 0 of that stream, so two calls that cover a frame with [`WARM_UP_SECONDS`] of
/// audio before it and [`lookahead_frames`] after it give the same result there, however
/// the stream was cut. Calls whose input starts at the same frame (such as the clip's start)
/// agree exactly, given the lookahead.
///
/// A trailing partial frame (when the length is not a multiple of `channels`) is returned
/// unchanged, and so is everything when `channels` or `sample_rate` is 0.
pub fn process_at(
    effects: &[AudioEffect],
    input: &[f32],
    channels: u16,
    sample_rate: u32,
    start: i64,
) -> Vec<f32> {
    let mut buffer = input.to_vec();
    let channels = usize::from(channels);
    if channels == 0 || sample_rate == 0 {
        return buffer;
    }
    let frames = buffer.len() / channels;
    if frames == 0 {
        return buffer;
    }
    let body = &mut buffer[..frames * channels];
    for effect in effects {
        match *effect {
            AudioEffect::Denoise { amount } => {
                let amount = clamp_amount(amount);
                if amount > 0.0 {
                    let floor = 10f32.powf(-DENOISE_RANGE_DB * amount / 20.0);
                    Denoiser::new(sample_rate, channels).run(body, floor, start);
                }
            }
            AudioEffect::EnhanceVoice { amount } => {
                let amount = clamp_amount(amount);
                if amount > 0.0 {
                    enhance_voice(body, channels, sample_rate, amount);
                }
            }
            AudioEffect::Vocals { mode } => {
                if channels == 2 {
                    vocals(body, sample_rate, mode);
                }
            }
            AudioEffect::Limiter {
                gain_db,
                ceiling_db,
            } => {
                let gain = finite(gain_db).clamp(0.0, 24.0);
                let ceiling = finite(ceiling_db).clamp(-12.0, 0.0);
                limit(body, channels, sample_rate, gain, ceiling);
            }
            AudioEffect::Voice(ref voice) => voice.apply(body, channels, sample_rate, start),
        }
    }
    buffer
}

/// `amount` clamped to 0 to 1, with NaN as 0.
fn clamp_amount(amount: f32) -> f32 {
    if amount.is_nan() {
        0.0
    } else {
        amount.clamp(0.0, 1.0)
    }
}

/// `x`, or silence when it is not finite.
fn finite(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

// Denoise
//
// Short-time Fourier transform with a periodic Hann window at 75% overlap, analysed and
// resynthesised with the same window (weighted overlap-add), which reconstructs the input
// exactly when every gain is 1 (the squared windows sum to 3/2 at every sample).
//
// The noise power in each frequency bin is estimated by minimum statistics (R. Martin,
// "Noise power spectral density estimation based on optimal smoothing and minimum
// statistics", IEEE Transactions on Speech and Audio Processing 9(5), 2001): the power is
// smoothed over time, its minimum is tracked over a sliding window of about 1.4 seconds
// kept as eight sub-window minima plus the current one, and the minimum is scaled by a bias
// factor, because the minimum of a fluctuating power lies below its mean. Martin's
// time-varying smoothing and his fast tracking of rising noise are left out: the smoothing
// constant is fixed and the bias factor was calibrated on white noise for this smoothing and
// window.
//
// The gain is the Wiener gain of the a priori signal-to-noise ratio, estimated by the
// decision-directed rule (Y. Ephraim and D. Malah, "Speech enhancement using a minimum
// mean-square error short-time spectral amplitude estimator", IEEE Transactions on
// Acoustics, Speech and Signal Processing 32(6), 1984), with a lower limit on that ratio
// against musical noise (O. Cappé, "Elimination of the musical noise phenomenon with the
// Ephraim and Malah noise suppressor", IEEE Transactions on Speech and Audio Processing
// 2(2), 1994). The gain is then smoothed across neighbouring bins, held briefly when it
// falls, and kept above a floor set by the amount.
//
// All channels share one gain per bin, computed from their average power, so the stereo
// image is unchanged. Channels are transformed in pairs, packed into one complex signal.

/// Target analysis frame length, rounded up to a power of two (1024 to 2048 samples at
/// 32 to 48 kHz, 21 to 46 ms).
const FRAME_SECONDS: f64 = 0.032;
/// Frames overlap by 75%.
const OVERLAP: usize = 4;
/// Time constant of the smoothed power that minimum statistics tracks.
const SMOOTHING_SECONDS: f64 = 0.04;
/// Length of one sub-window of the minimum search.
const SUBWINDOW_SECONDS: f64 = 0.16;
/// Completed sub-windows kept besides the current one.
const SUBWINDOWS: usize = 8;
/// The noise estimate starts from the average power over this much audio from the start of
/// the input, so the first minima are not single noisy frames.
const INIT_SECONDS: f64 = 0.2;
/// Bias compensation of the minimum (the ratio of the mean smoothed noise power to its
/// windowed minimum, measured on white noise).
const MIN_BIAS: f32 = 3.1;
/// Weight of the previous frame in the decision-directed a priori SNR.
const DD_ALPHA: f32 = 0.98;
/// Lowest a priori SNR (-25 dB).
const XI_MIN: f32 = 0.003_162_277_7;
/// Time constant with which a falling gain follows.
const RELEASE_SECONDS: f64 = 0.03;
/// Noise reduction at amount 1: the lowest gain is -24 dB.
const DENOISE_RANGE_DB: f32 = 24.0;
/// Noise power below which a bin counts as digital silence.
const TINY_POWER: f32 = 1e-30;

/// Frame sizes of the denoiser at a sample rate.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    /// Samples per analysis frame (a power of two).
    len: usize,
    /// Samples between frames.
    hop: usize,
    /// Frames averaged for the initial noise estimate.
    init_frames: usize,
}

impl Geometry {
    fn new(sample_rate: u32) -> Self {
        let rate = f64::from(sample_rate);
        let len = ((rate * FRAME_SECONDS).ceil() as usize)
            .next_power_of_two()
            .clamp(256, 16_384);
        let hop = len / OVERLAP;
        let hop_seconds = hop as f64 / rate;
        Geometry {
            len,
            hop,
            init_frames: frames_in(INIT_SECONDS, hop_seconds),
        }
    }
}

/// Whole frames (at least one) in `seconds` with frames every `hop_seconds`.
fn frames_in(seconds: f64, hop_seconds: f64) -> usize {
    ((seconds / hop_seconds).round() as usize).max(1)
}

/// One run of the denoiser over a buffer: transforms, noise estimate and gains.
struct Denoiser {
    geometry: Geometry,
    /// Frequency bins kept (`len / 2 + 1`); the others mirror them.
    bins: usize,
    channels: usize,
    window: Vec<f32>,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Weight of the previous smoothed power.
    smoothing: f32,
    /// Weight of the previous gain while the gain falls.
    release: f32,
    /// Frames per minimum sub-window.
    subwindow: i64,
    /// Smoothed power per bin.
    smooth: Vec<f32>,
    /// Minimum of the smoothed power in the current sub-window.
    current_min: Vec<f32>,
    /// Minima of the completed sub-windows, `SUBWINDOWS` rows of `bins`.
    history: Vec<f32>,
    /// Minimum over `history`.
    history_min: Vec<f32>,
    /// Row of `history` written next.
    history_next: usize,
    /// Estimated clean power of the previous frame (decision-directed memory).
    clean: Vec<f32>,
    /// Gain after release smoothing, before the floor.
    held: Vec<f32>,
    /// Average power per channel of the current frame.
    power: Vec<f32>,
    /// Wiener gain of the current frame.
    raw: Vec<f32>,
    /// Gain applied to the current frame.
    gain: Vec<f32>,
    /// Spectra of the current frame, one per channel pair.
    spectra: Vec<Complex<f32>>,
}

impl Denoiser {
    fn new(sample_rate: u32, channels: usize) -> Self {
        let geometry = Geometry::new(sample_rate);
        let len = geometry.len;
        let bins = len / 2 + 1;
        let hop_seconds = geometry.hop as f64 / f64::from(sample_rate);
        let mut planner = FftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(len);
        let inverse = planner.plan_fft_inverse(len);
        let scratch_len = forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len());
        let window = (0..len)
            .map(|n| (0.5 - 0.5 * (TAU * n as f64 / len as f64).cos()) as f32)
            .collect();
        let pairs = channels.div_ceil(2);
        Denoiser {
            geometry,
            bins,
            channels,
            window,
            forward,
            inverse,
            scratch: vec![Complex::default(); scratch_len],
            smoothing: (-hop_seconds / SMOOTHING_SECONDS).exp() as f32,
            release: (-hop_seconds / RELEASE_SECONDS).exp() as f32,
            subwindow: frames_in(SUBWINDOW_SECONDS, hop_seconds) as i64,
            smooth: vec![0.0; bins],
            current_min: vec![f32::INFINITY; bins],
            history: vec![f32::INFINITY; SUBWINDOWS * bins],
            history_min: vec![f32::INFINITY; bins],
            history_next: 0,
            clean: vec![0.0; bins],
            held: vec![0.0; bins],
            power: vec![0.0; bins],
            raw: vec![0.0; bins],
            gain: vec![0.0; bins],
            spectra: vec![Complex::default(); pairs * len],
        }
    }

    /// Denoises `samples` (whole frames of `channels`) in place, with gains at least
    /// `floor`; the first frame is frame `start` of the stream, which anchors the frame grid.
    fn run(&mut self, samples: &mut [f32], floor: f32, start: i64) {
        let channels = self.channels;
        let frames = samples.len() / channels;
        if frames == 0 {
            return;
        }
        let hop = self.geometry.hop as i64;
        let behind = OVERLAP as i64 - 1;
        let offset = start.rem_euclid(hop);
        // Frame `j` starts at buffer sample `first_at + j * hop`; the first frames reach
        // back before the buffer so that every sample is covered by OVERLAP frames.
        let first_index = start.div_euclid(hop) - behind;
        let first_at = -(behind * hop + offset);
        let count = (frames as i64 - first_at + hop - 1) / hop;
        // Frames that start inside the buffer update the noise estimate; earlier ones,
        // partly before the buffer, only use it. Without such frames, every frame does.
        let inside = if offset == 0 { behind } else { behind + 1 };
        let updates_from = if inside < count { inside } else { 0 };

        // The initial estimate: the average power of the first frames.
        let init_end = (updates_from + self.geometry.init_frames as i64).min(count);
        let mut average = vec![0.0f32; self.bins];
        for j in updates_from..init_end {
            self.analyze(samples, frames, first_at + j * hop);
            for (sum, p) in average.iter_mut().zip(&self.power) {
                *sum += p;
            }
        }
        let averaged = (init_end - updates_from).max(1) as f32;
        for (k, sum) in average.iter().enumerate() {
            self.smooth[k] = sum / averaged;
            self.current_min[k] = sum / averaged;
        }

        let mut out = vec![0.0f32; samples.len()];
        for j in 0..count {
            let at = first_at + j * hop;
            self.analyze(samples, frames, at);
            self.update_gains(j >= updates_from, first_index + j, floor);
            self.synthesize(&mut out, frames, at);
        }
        samples.copy_from_slice(&out);
    }

    /// Range of window positions of the frame at buffer sample `at` that fall inside a
    /// buffer of `frames` frames.
    fn inside(&self, frames: usize, at: i64) -> (usize, usize) {
        let len = self.geometry.len as i64;
        let lo = (-at).clamp(0, len);
        let hi = (frames as i64 - at).clamp(lo, len);
        (lo as usize, hi as usize)
    }

    /// Transforms the frame at buffer sample `at` into `spectra` and its average power per
    /// channel into `power`; samples outside the buffer are silence.
    fn analyze(&mut self, samples: &[f32], frames: usize, at: i64) {
        let len = self.geometry.len;
        let channels = self.channels;
        let (lo, hi) = self.inside(frames, at);
        self.power.fill(0.0);
        for (pair, spectrum) in self.spectra.chunks_exact_mut(len).enumerate() {
            let left = 2 * pair;
            let right = left + 1;
            spectrum.fill(Complex::default());
            // Buffer frames `at + lo..at + hi` lie in 0..frames, by `inside`.
            let first = (at + lo as i64) as usize;
            let window = &self.window[lo..hi];
            for ((z, w), frame) in spectrum[lo..hi].iter_mut().zip(window).zip(first..) {
                let base = frame * channels;
                let re = finite(samples[base + left]) * w;
                let im = if right < channels {
                    finite(samples[base + right]) * w
                } else {
                    0.0
                };
                *z = Complex::new(re, im);
            }
            self.forward
                .process_with_scratch(spectrum, &mut self.scratch);
            // The two packed channels' powers at bin k sum to (|Z(k)|^2 + |Z(N-k)|^2) / 2.
            for (k, p) in self.power.iter_mut().enumerate() {
                let mirror = (len - k) % len;
                *p += 0.5 * (spectrum[k].norm_sqr() + spectrum[mirror].norm_sqr());
            }
        }
        let scale = 1.0 / channels as f32;
        for p in &mut self.power {
            *p *= scale;
        }
    }

    /// Computes the gains of the current frame (stream frame `index`) into `gain`, updating
    /// the noise estimate and the gain memory when `update` is set.
    fn update_gains(&mut self, update: bool, index: i64, floor: f32) {
        let bins = self.bins;
        let alpha = self.smoothing;
        for k in 0..bins {
            let p = self.power[k];
            let minimum = if update {
                let s = alpha * self.smooth[k] + (1.0 - alpha) * p;
                self.smooth[k] = s;
                let m = self.current_min[k].min(s);
                self.current_min[k] = m;
                m
            } else {
                self.current_min[k]
            };
            let noise = (MIN_BIAS * minimum.min(self.history_min[k])).max(TINY_POWER);
            let gamma = p / noise;
            let xi = (DD_ALPHA * self.clean[k] / noise + (1.0 - DD_ALPHA) * (gamma - 1.0).max(0.0))
                .max(XI_MIN);
            let g = xi / (1.0 + xi);
            if update {
                self.clean[k] = g * g * p;
            }
            self.raw[k] = g;
        }
        let release = self.release;
        for k in 0..bins {
            // Mirrored at the edges: bin -1 is bin 1, bin `bins` is bin `bins - 2`.
            let below = self.raw[if k == 0 { 1 } else { k - 1 }];
            let above = self.raw[if k + 1 == bins { bins - 2 } else { k + 1 }];
            let smoothed = 0.25 * (below + above) + 0.5 * self.raw[k];
            let previous = self.held[k];
            let held = if smoothed >= previous {
                smoothed
            } else {
                release * previous + (1.0 - release) * smoothed
            };
            if update {
                self.held[k] = held;
            }
            self.gain[k] = held.max(floor);
        }
        // Sub-windows end on fixed stream frames, so any cut of the stream sees the same.
        if update && (index + 1).rem_euclid(self.subwindow) == 0 {
            let row = self.history_next * bins;
            self.history[row..row + bins].copy_from_slice(&self.current_min);
            self.history_next = (self.history_next + 1) % SUBWINDOWS;
            self.history_min.fill(f32::INFINITY);
            for minima in self.history.chunks_exact(bins) {
                for (m, h) in self.history_min.iter_mut().zip(minima) {
                    *m = m.min(*h);
                }
            }
            self.current_min.fill(f32::INFINITY);
        }
    }

    /// Applies `gain` to the current spectra and overlap-adds the frame at buffer sample `at`
    /// into `out`.
    fn synthesize(&mut self, out: &mut [f32], frames: usize, at: i64) {
        let len = self.geometry.len;
        let channels = self.channels;
        let (lo, hi) = self.inside(frames, at);
        // The inverse transform is unnormalised (a factor `len`) and the squared windows of
        // OVERLAP frames sum to OVERLAP * 3 / 8.
        let scale = 1.0 / (len as f32 * OVERLAP as f32 * 0.375);
        for (pair, spectrum) in self.spectra.chunks_exact_mut(len).enumerate() {
            for (k, z) in spectrum.iter_mut().enumerate() {
                *z *= self.gain[k.min(len - k)];
            }
            self.inverse
                .process_with_scratch(spectrum, &mut self.scratch);
            let left = 2 * pair;
            let right = left + 1;
            let first = (at + lo as i64) as usize;
            let window = &self.window[lo..hi];
            for ((z, w), frame) in spectrum[lo..hi].iter().zip(window).zip(first..) {
                let base = frame * channels;
                let w = w * scale;
                out[base + left] += z.re * w;
                if right < channels {
                    out[base + right] += z.im * w;
                }
            }
        }
    }
}

// Biquad filters from the RBJ Audio EQ Cookbook (Robert Bristow-Johnson), run in
// transposed direct form II in double precision.

/// A second-order filter, normalised so that `a0` is 1.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    /// The cookbook's `cos(w0)` and `alpha` for `f0` Hz at `rate`, or `None` when `f0` is
    /// not safely below the Nyquist frequency.
    fn prepare(rate: f64, f0: f64, q: f64) -> Option<(f64, f64)> {
        if !(f0 > 0.0 && f0 < 0.45 * rate && q > 0.0) {
            return None;
        }
        let w0 = 2.0 * PI * f0 / rate;
        Some((w0.cos(), w0.sin() / (2.0 * q)))
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

    fn high_pass(rate: f64, f0: f64, q: f64) -> Option<Self> {
        let (cos, alpha) = Self::prepare(rate, f0, q)?;
        let b = (1.0 + cos) / 2.0;
        Some(Self::normalized(
            [b, -(1.0 + cos), b],
            [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        ))
    }

    fn low_pass(rate: f64, f0: f64, q: f64) -> Option<Self> {
        let (cos, alpha) = Self::prepare(rate, f0, q)?;
        let b = (1.0 - cos) / 2.0;
        Some(Self::normalized(
            [b, 1.0 - cos, b],
            [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        ))
    }

    fn peaking(rate: f64, f0: f64, q: f64, gain_db: f64) -> Option<Self> {
        let (cos, alpha) = Self::prepare(rate, f0, q)?;
        let a = 10f64.powf(gain_db / 40.0);
        Some(Self::normalized(
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        ))
    }

    /// Filters one sample, with `state` the filter's two delays.
    fn run(&self, state: &mut [f64; 2], x: f64) -> f64 {
        let y = self.b0 * x + state[0];
        state[0] = self.b1 * x - self.a1 * y + state[1];
        state[1] = self.b2 * x - self.a2 * y;
        y
    }
}

/// Runs `filters` in series on one sample, with `states` one per filter.
fn run_chain(filters: &[Biquad], states: &mut [[f64; 2]], x: f64) -> f64 {
    filters
        .iter()
        .zip(states.iter_mut())
        .fold(x, |x, (filter, state)| filter.run(state, x))
}

// EnhanceVoice
//
// Three cookbook filters (rumble high-pass, mud cut, presence lift), then a feed-forward
// compressor on the channel-linked RMS level with a soft knee and attack and release
// smoothing of the gain reduction in decibels (D. Giannoulis, M. Massberg and J. D. Reiss,
// "Digital dynamic range compressor design: a tutorial and analysis", Journal of the Audio
// Engineering Society 60(6), 2012). Levels are 10 log10 of the mean square, so a full-scale
// square wave is 0 dB and a full-scale sine -3 dB. The amount scales the high-pass cutoff,
// the two peaking gains and the ratio's distance from 1:1. The makeup gain undoes the
// reduction at a reference speech level, so speech near that level keeps its loudness while
// louder passages come down and quieter ones come up.

/// Rumble high-pass cutoff at amount 1.
const RUMBLE_HZ: f64 = 80.0;
/// Mud cut centre, quality and gain at amount 1.
const MUD_HZ: f64 = 300.0;
const MUD_Q: f64 = 1.0;
const MUD_DB: f64 = -3.0;
/// Presence lift centre, quality and gain at amount 1.
const PRESENCE_HZ: f64 = 3000.0;
const PRESENCE_Q: f64 = 1.0;
const PRESENCE_DB: f64 = 4.0;
/// Compressor threshold and knee width, in dB of mean square.
const THRESHOLD_DB: f64 = -20.0;
const KNEE_DB: f64 = 6.0;
/// Compression ratio at amount 1.
const RATIO: f64 = 3.0;
/// Speech level at which the makeup gain cancels the reduction.
const REFERENCE_DB: f64 = -16.0;
/// Time constants of the RMS detector and of the gain reduction's attack and release.
const RMS_SECONDS: f64 = 0.01;
const ATTACK_SECONDS: f64 = 0.005;
const RELEASE_COMPRESSOR_SECONDS: f64 = 0.1;
/// Butterworth quality.
const BUTTERWORTH_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Gain reduction in dB of the soft-knee compressor at `level` dB.
fn gain_reduction(level: f64, ratio: f64) -> f64 {
    let over = level - THRESHOLD_DB;
    let slope = 1.0 - 1.0 / ratio;
    if 2.0 * over <= -KNEE_DB {
        0.0
    } else if 2.0 * over < KNEE_DB {
        let into = over + KNEE_DB / 2.0;
        slope * into * into / (2.0 * KNEE_DB)
    } else {
        slope * over
    }
}

fn enhance_voice(samples: &mut [f32], channels: usize, sample_rate: u32, amount: f32) {
    let rate = f64::from(sample_rate);
    let amount = f64::from(amount);
    let filters: Vec<Biquad> = [
        Biquad::high_pass(rate, RUMBLE_HZ * amount, BUTTERWORTH_Q),
        Biquad::peaking(rate, MUD_HZ, MUD_Q, MUD_DB * amount),
        Biquad::peaking(rate, PRESENCE_HZ, PRESENCE_Q, PRESENCE_DB * amount),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut states = vec![[0.0f64; 2]; filters.len() * channels];
    let mut shaped = vec![0.0f64; channels];

    let ratio = 1.0 + (RATIO - 1.0) * amount;
    let makeup = gain_reduction(REFERENCE_DB, ratio);
    let rms = 1.0 - (-1.0 / (RMS_SECONDS * rate)).exp();
    let attack = (-1.0 / (ATTACK_SECONDS * rate)).exp();
    let release = (-1.0 / (RELEASE_COMPRESSOR_SECONDS * rate)).exp();
    // Below the knee there is no reduction, so the logarithm can be skipped.
    let knee_start = 10f64.powf((THRESHOLD_DB - KNEE_DB / 2.0) / 10.0);
    let to_gain = std::f64::consts::LN_10 / 20.0;
    let mut mean_square = 0.0f64;
    let mut reduction = 0.0f64;

    for frame in samples.chunks_exact_mut(channels) {
        let mut sum = 0.0;
        for (c, (sample, out)) in frame.iter().zip(shaped.iter_mut()).enumerate() {
            let state = &mut states[c * filters.len()..(c + 1) * filters.len()];
            let y = run_chain(&filters, state, f64::from(finite(*sample)));
            *out = y;
            sum += y * y;
        }
        mean_square += rms * (sum / channels as f64 - mean_square);
        let target = if mean_square > knee_start {
            gain_reduction(10.0 * mean_square.log10(), ratio)
        } else {
            0.0
        };
        let coefficient = if target > reduction { attack } else { release };
        reduction = target + coefficient * (reduction - target);
        let gain = ((makeup - reduction) * to_gain).exp();
        for (sample, y) in frame.iter_mut().zip(&shaped) {
            *sample = (y * gain) as f32;
        }
    }
}

// Vocals

/// Voice band of the isolated centre.
const VOICE_LOW_HZ: f64 = 120.0;
const VOICE_HIGH_HZ: f64 = 8000.0;

/// The classical mid/side centre-channel method on interleaved stereo `samples`.
fn vocals(samples: &mut [f32], sample_rate: u32, mode: VocalMode) {
    match mode {
        VocalMode::Remove => {
            for frame in samples.as_chunks_mut::<2>().0 {
                let side = 0.5 * (finite(frame[0]) - finite(frame[1]));
                frame[0] = side;
                frame[1] = -side;
            }
        }
        VocalMode::Isolate => {
            let rate = f64::from(sample_rate);
            let filters: Vec<Biquad> = [
                Biquad::high_pass(rate, VOICE_LOW_HZ, BUTTERWORTH_Q),
                Biquad::low_pass(rate, VOICE_HIGH_HZ, BUTTERWORTH_Q),
            ]
            .into_iter()
            .flatten()
            .collect();
            let mut states = vec![[0.0f64; 2]; filters.len()];
            for frame in samples.as_chunks_mut::<2>().0 {
                let mid = 0.5 * (f64::from(finite(frame[0])) + f64::from(finite(frame[1])));
                let y = run_chain(&filters, &mut states, mid) as f32;
                frame[0] = y;
                frame[1] = y;
            }
        }
    }
}

// Limiter

/// How far the limiter looks ahead, and the length of its gain smoothing.
const LIMITER_LOOKAHEAD_SECONDS: f64 = 0.005;
/// How long the limiter takes to let the level back up after a peak.
const LIMITER_RELEASE_SECONDS: f64 = 0.08;

/// The limiter's look-ahead in frames at `sample_rate`.
fn limiter_window(sample_rate: u32) -> usize {
    ((LIMITER_LOOKAHEAD_SECONDS * f64::from(sample_rate)).round() as usize).max(1)
}

/// Applies `gain_db` to interleaved `samples` and limits their peaks to `ceiling_db`.
///
/// With `r` the gain each frame may have (1, or less where the amplified peak would pass
/// the ceiling), the gain used is the release-smoothed mean over the last `L` frames of the
/// minimum of `r` over the next `L` frames. Every frame in that mean's window looks at the
/// current frame, so the mean is never above the current frame's `r`, and no peak passes.
fn limit(samples: &mut [f32], channels: usize, sample_rate: u32, gain_db: f32, ceiling_db: f32) {
    let frames = samples.len() / channels;
    let window = limiter_window(sample_rate);
    let input_gain = 10f64.powf(f64::from(gain_db) / 20.0);
    let ceiling = 10f64.powf(f64::from(ceiling_db) / 20.0);
    let allowed: Vec<f64> = samples
        .chunks_exact(channels)
        .map(|frame| {
            let peak = frame
                .iter()
                .map(|v| f64::from(finite(*v)).abs())
                .fold(0.0, f64::max)
                * input_gain;
            if peak > ceiling { ceiling / peak } else { 1.0 }
        })
        .collect();
    // The minimum of `allowed` over frames i..=i+window, with a monotonic queue.
    let mut ahead = vec![1.0f64; frames];
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    let mut next = 0;
    for (i, slot) in ahead.iter_mut().enumerate() {
        while next < frames && next <= i + window {
            while queue.back().is_some_and(|&j| allowed[j] >= allowed[next]) {
                queue.pop_back();
            }
            queue.push_back(next);
            next += 1;
        }
        while queue.front().is_some_and(|&j| j < i) {
            queue.pop_front();
        }
        *slot = queue.front().map_or(1.0, |&j| allowed[j]);
    }
    // The mean of the last `window` values (fewer at the start), then the release.
    let release = 1.0 - (-1.0 / (LIMITER_RELEASE_SECONDS * f64::from(sample_rate))).exp();
    let (mut sum, mut envelope) = (0.0f64, 1.0f64);
    for i in 0..frames {
        sum += ahead[i];
        if i >= window {
            sum -= ahead[i - window];
        }
        let mean = sum / (i + 1).min(window) as f64;
        envelope = if mean < envelope {
            mean
        } else {
            envelope + (mean - envelope) * release
        };
        let gain = input_gain * envelope.min(allowed[i]);
        for value in &mut samples[i * channels..(i + 1) * channels] {
            *value = (f64::from(finite(*value)) * gain) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// A small deterministic generator (xorshift64*), uniform in -1 to 1.
    struct Noise(u64);

    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let bits = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40;
            (bits as f64 / (1u64 << 23) as f64 - 1.0) as f32
        }
    }

    /// Peak amplitude for `db` dBFS.
    fn amplitude(db: f64) -> f32 {
        10f64.powf(db / 20.0) as f32
    }

    /// Uniform white noise with an RMS of `db` dBFS.
    fn white(seed: u64, db: f64, frames: usize) -> Vec<f32> {
        let mut noise = Noise(seed);
        let scale = amplitude(db) * 3f32.sqrt();
        (0..frames).map(|_| noise.next() * scale).collect()
    }

    fn sine(freq: f64, amp: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| amp * (TAU * freq * i as f64 / f64::from(RATE)).sin() as f32)
            .collect()
    }

    fn seconds(s: f64) -> usize {
        (s * f64::from(RATE)).round() as usize
    }

    fn interleave(left: &[f32], right: &[f32]) -> Vec<f32> {
        left.iter().zip(right).flat_map(|(l, r)| [*l, *r]).collect()
    }

    fn channel(x: &[f32], channels: usize, c: usize) -> Vec<f32> {
        x.iter().skip(c).step_by(channels).copied().collect()
    }

    fn mean_square(x: &[f32]) -> f64 {
        x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64
    }

    fn db(ratio: f64) -> f64 {
        10.0 * ratio.log10()
    }

    /// Power of the `freq` Hz component of `x` (Goertzel).
    fn tone_power(x: &[f32], freq: f64) -> f64 {
        let w = TAU * freq / f64::from(RATE);
        let coefficient = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for v in x {
            let s = f64::from(*v) + coefficient * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        let n = x.len() as f64;
        (s1 * s1 + s2 * s2 - coefficient * s1 * s2) / (n * n)
    }

    fn max_difference(a: &[f32], b: &[f32]) -> f32 {
        assert_eq!(a.len(), b.len());
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f32::max)
    }

    /// Ten seconds of a speech-like centred voice (harmonics of 150 Hz in syllables with
    /// pauses), different tones on each side and independent noise in each channel.
    fn noisy_stereo(frames: usize) -> Vec<f32> {
        let noise_left = white(11, -30.0, frames);
        let noise_right = white(23, -30.0, frames);
        let rate = f64::from(RATE);
        let mut out = Vec::with_capacity(2 * frames);
        for i in 0..frames {
            let t = i as f64 / rate;
            let syllable = (TAU * 2.5 * t).sin().max(0.0).sqrt();
            let envelope = if t % 2.0 > 1.6 { 0.0 } else { syllable };
            let voice: f64 = (1..=6)
                .map(|h| 0.3 / h as f64 * (TAU * 150.0 * h as f64 * t + h as f64).sin())
                .sum::<f64>()
                * envelope;
            let left = voice + 0.1 * (TAU * 220.0 * t).sin();
            let right = voice + 0.1 * (TAU * 330.0 * t).sin();
            out.push(left as f32 + noise_left[i]);
            out.push(right as f32 + noise_right[i]);
        }
        out
    }

    #[test]
    fn denoise_lowers_noise_and_keeps_the_tone() {
        let frames = seconds(4.0);
        let noise = white(7, -30.0, frames);
        let tone = sine(440.0, amplitude(-12.0), frames);
        let input: Vec<f32> = (0..frames)
            .map(|i| noise[i] + if i >= seconds(2.0) { tone[i] } else { 0.0 })
            .collect();
        let output = process(&[AudioEffect::Denoise { amount: 1.0 }], &input, 1, RATE);
        assert_eq!(output.len(), input.len());

        // Noise only, from the very start of the clip.
        let quiet = 0..seconds(1.9);
        let reduction = db(mean_square(&output[quiet.clone()]) / mean_square(&input[quiet]));
        // The tone, measured while the minimum search still sees the noise before it: a
        // tone held steady for longer than the search window counts as noise (hum).
        let held = seconds(2.2)..seconds(3.2);
        let tone_change =
            db(tone_power(&output[held.clone()], 440.0) / tone_power(&input[held.clone()], 440.0));
        println!("noise {reduction:.2} dB, tone {tone_change:.3} dB");
        assert!(reduction <= -10.0, "noise changed by {reduction:.2} dB");
        assert!(
            tone_change.abs() < 1.5,
            "tone changed by {tone_change:.3} dB"
        );
    }

    #[test]
    fn amount_zero_returns_the_input() {
        let frames = seconds(1.0);
        let input = interleave(&white(3, -20.0, frames), &sine(500.0, 0.5, frames));
        for effect in [
            AudioEffect::Denoise { amount: 0.0 },
            AudioEffect::EnhanceVoice { amount: 0.0 },
            AudioEffect::Denoise { amount: f32::NAN },
            AudioEffect::EnhanceVoice { amount: f32::NAN },
            AudioEffect::Denoise { amount: -3.0 },
        ] {
            let output = process(std::slice::from_ref(&effect), &input, 2, RATE);
            assert!(max_difference(&output, &input) <= 1e-5, "{effect:?}");
            assert_eq!(lookahead_frames(&[effect], RATE), 0);
        }
    }

    #[test]
    fn amounts_above_one_act_as_one() {
        let frames = seconds(1.0);
        let input = interleave(&white(5, -25.0, frames), &white(6, -25.0, frames));
        for (over, one) in [
            (
                AudioEffect::Denoise { amount: 7.0 },
                AudioEffect::Denoise { amount: 1.0 },
            ),
            (
                AudioEffect::EnhanceVoice { amount: 2.0 },
                AudioEffect::EnhanceVoice { amount: 1.0 },
            ),
        ] {
            assert_eq!(
                process(&[over], &input, 2, RATE),
                process(&[one], &input, 2, RATE)
            );
        }
    }

    #[test]
    fn denoise_reconstructs_when_the_gain_is_one() {
        // The transform alone: every gain held at 1 gives the input back, from the first
        // sample to the last, for both packed channels.
        let frames = seconds(1.0) + 123;
        let input = interleave(&sine(440.0, 0.5, frames), &sine(1234.0, 0.25, frames));
        let mut output = input.clone();
        Denoiser::new(RATE, 2).run(&mut output, 1.0, 0);
        let error = max_difference(&output, &input);
        println!("unity gain error {error:e}");
        assert!(error < 1e-3, "error {error}");

        // At full amount, a clean tone over digital silence has nothing to remove.
        let tone = sine(1000.0, 0.5, seconds(1.0));
        let mut clean = vec![0.0f32; seconds(0.5)];
        clean.extend(&tone);
        clean.resize(seconds(2.0), 0.0);
        let output = process(&[AudioEffect::Denoise { amount: 1.0 }], &clean, 1, RATE);
        let first_window = Geometry::new(RATE).len;
        let error = max_difference(&output[first_window..], &clean[first_window..]);
        println!("clean tone error {error:e}");
        assert!(error < 1e-3, "error {error}");
    }

    #[test]
    fn enhance_voice_shapes_the_spectrum() {
        // Quiet tones, below the compressor's knee, so only the filters and makeup act.
        let frames = seconds(2.0);
        let level = amplitude(-30.0);
        let (low, mud, presence) = (
            sine(40.0, level, frames),
            sine(300.0, level, frames),
            sine(3000.0, level, frames),
        );
        let input: Vec<f32> = (0..frames).map(|i| low[i] + mud[i] + presence[i]).collect();
        let output = process(
            &[AudioEffect::EnhanceVoice { amount: 1.0 }],
            &input,
            1,
            RATE,
        );
        let steady = seconds(1.0)..frames;
        let gain =
            |freq: f64| {
                db(tone_power(&output[steady.clone()], freq)
                    / tone_power(&input[steady.clone()], freq))
            };
        let (g40, g300, g3000) = (gain(40.0), gain(300.0), gain(3000.0));
        println!("40 Hz {g40:.2} dB, 300 Hz {g300:.2} dB, 3 kHz {g3000:.2} dB");
        assert!(g40 <= -6.0, "40 Hz changed by {g40:.2} dB");
        assert!(g3000 > 0.0, "3 kHz changed by {g3000:.2} dB");
        assert!(
            g3000 - g300 >= 4.0,
            "3 kHz {g3000:.2} dB, 300 Hz {g300:.2} dB"
        );
    }

    #[test]
    fn enhance_voice_compresses_loud_passages() {
        let frames = seconds(1.5);
        let steady = seconds(1.0)..frames;
        let level_change = |db_fs: f64| {
            let input = sine(1000.0, amplitude(db_fs), frames);
            let output = process(
                &[AudioEffect::EnhanceVoice { amount: 1.0 }],
                &input,
                1,
                RATE,
            );
            db(mean_square(&output[steady.clone()]) / mean_square(&input[steady.clone()]))
        };
        let (quiet, loud) = (level_change(-40.0), level_change(-3.0));
        println!("quiet {quiet:.2} dB, loud {loud:.2} dB");
        // 37 dB of input range shrink by the reduction above the threshold.
        assert!(quiet > 1.0, "quiet changed by {quiet:.2} dB");
        assert!(quiet - loud > 8.0, "quiet {quiet:.2} dB, loud {loud:.2} dB");
    }

    #[test]
    fn vocals_remove_and_isolate_the_centre() {
        let frames = seconds(1.0);
        let steady = seconds(0.1)..frames;
        let x = sine(1000.0, 0.5, frames);
        let silent = vec![0.0f32; frames];
        let inverted: Vec<f32> = x.iter().map(|v| -v).collect();
        let centre = interleave(&x, &x);
        let left = interleave(&x, &silent);
        let anti = interleave(&x, &inverted);
        let remove = [AudioEffect::Vocals {
            mode: VocalMode::Remove,
        }];
        let isolate = [AudioEffect::Vocals {
            mode: VocalMode::Isolate,
        }];
        let power = |signal: &[f32], c: usize| mean_square(&channel(signal, 2, c)[steady.clone()]);

        let removed = process(&remove, &centre, 2, RATE);
        assert!(power(&removed, 0) + power(&removed, 1) <= 1e-4 * power(&centre, 0));

        let removed = process(&remove, &left, 2, RATE);
        let kept = db(power(&removed, 0) / power(&left, 0));
        assert!((kept + 6.02).abs() < 0.05, "hard left kept at {kept:.2} dB");
        assert!(
            channel(&removed, 2, 0)
                .iter()
                .zip(channel(&removed, 2, 1))
                .all(|(l, r)| *l == -r)
        );

        let isolated = process(&isolate, &centre, 2, RATE);
        for c in 0..2 {
            let change = db(power(&isolated, c) / power(&centre, c));
            assert!(change.abs() < 1.0, "centre changed by {change:.2} dB");
        }
        let isolated = process(&isolate, &anti, 2, RATE);
        assert!(power(&isolated, 0) + power(&isolated, 1) <= 1e-4 * power(&anti, 0));

        let bass = sine(50.0, 0.5, frames);
        let bass = interleave(&bass, &bass);
        let isolated = process(&isolate, &bass, 2, RATE);
        let change = db(power(&isolated, 0) / power(&bass, 0));
        assert!(change <= -10.0, "centred bass changed by {change:.2} dB");

        let mono = sine(440.0, 0.5, frames);
        let surround = white(9, -20.0, 6 * 1000);
        for effects in [&remove, &isolate] {
            assert_eq!(process(effects, &mono, 1, RATE), mono);
            assert_eq!(process(effects, &surround, 6, RATE), surround);
        }
    }

    /// The chain the streaming test runs.
    fn chain() -> Vec<AudioEffect> {
        vec![
            AudioEffect::Denoise { amount: 1.0 },
            AudioEffect::EnhanceVoice { amount: 1.0 },
            AudioEffect::Vocals {
                mode: VocalMode::Isolate,
            },
        ]
    }

    #[test]
    fn windows_match_the_whole_clip() {
        let effects = chain();
        let frames = seconds(10.0);
        let input = noisy_stereo(frames);
        let whole = process(&effects, &input, 2, RATE);
        let warm_up = seconds(WARM_UP_SECONDS);
        let lookahead = lookahead_frames(&effects, RATE);
        assert!(lookahead >= Geometry::new(RATE).len - 1);

        // Fed the way the mixer feeds a window: warm-up before (clamped at the clip's start),
        // lookahead after (silence past the clip's end).
        let window = |from: f64, to: f64| {
            let (wanted_start, wanted_end) = (seconds(from), seconds(to));
            let first = wanted_start.saturating_sub(warm_up);
            let last = wanted_end + lookahead;
            let mut fed = input[2 * first..2 * last.min(frames)].to_vec();
            fed.resize(2 * (last - first), 0.0);
            let out = process_at(&effects, &fed, 2, RATE, first as i64);
            let got = &out[2 * (wanted_start - first)..2 * (wanted_end - first)];
            max_difference(got, &whole[2 * wanted_start..2 * wanted_end])
        };
        for (from, to) in [(4.0, 4.5), (7.3, 7.8), (9.5, 10.0)] {
            let difference = window(from, to);
            println!("window at {from} s differs by {difference:e}");
            assert!(
                difference <= 1e-3,
                "window at {from} s differs by {difference}"
            );
        }
        // Fed from the clip's start, a window sees exactly what the whole clip saw.
        for (from, to) in [(0.0, 0.01), (1.0, 1.5)] {
            assert_eq!(window(from, to), 0.0, "window at {from} s");
        }
    }

    #[test]
    fn processing_is_deterministic_and_keeps_the_length() {
        let effects = chain();
        let input = noisy_stereo(seconds(3.0));
        let first = process(&effects, &input, 2, RATE);
        assert_eq!(first.len(), input.len());
        assert_eq!(first, process(&effects, &input, 2, RATE));
        assert!(first.iter().all(|v| v.is_finite()));
        assert_ne!(first, input);

        assert!(process(&effects, &[], 2, RATE).is_empty());
        // A trailing partial frame is passed through; nothing happens without channels.
        let mut odd = input[..2001].to_vec();
        odd[2000] = 0.75;
        let out = process(&effects, &odd, 2, RATE);
        assert_eq!(out.len(), 2001);
        assert_eq!(out[2000], 0.75);
        assert_eq!(process(&effects, &odd, 0, RATE), odd);
        assert_eq!(process(&effects, &odd, 2, 0), odd);
        // Short buffers, shorter than one analysis frame, still come back whole.
        for len in [1, 2, 100, 1000] {
            let short = &input[..2 * len];
            let out = process(&effects, short, 2, RATE);
            assert_eq!(out.len(), short.len());
            assert!(out.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn non_finite_samples_are_silence() {
        let frames = seconds(1.0);
        let clean = interleave(&sine(300.0, 0.3, frames), &sine(700.0, 0.3, frames));
        let mut broken = clean.clone();
        broken[1000] = f32::NAN;
        broken[5001] = f32::INFINITY;
        let mut zeroed = clean.clone();
        zeroed[1000] = 0.0;
        zeroed[5001] = 0.0;
        let effects = chain();
        let out = process(&effects, &broken, 2, RATE);
        assert!(out.iter().all(|v| v.is_finite()));
        assert_eq!(out, process(&effects, &zeroed, 2, RATE));
    }

    #[test]
    fn lookahead_covers_the_denoiser() {
        let geometry = Geometry::new(RATE);
        assert_eq!(geometry.len, 2048);
        assert_eq!(Geometry::new(44_100).len, 2048);
        assert_eq!(Geometry::new(16_000).len, 512);
        let denoise = lookahead_frames(&[AudioEffect::Denoise { amount: 0.5 }], RATE);
        assert_eq!(denoise, 2047 + geometry.init_frames * geometry.hop);
        assert_eq!(lookahead_frames(&chain(), RATE), denoise);
        assert_eq!(
            lookahead_frames(
                &[
                    AudioEffect::Denoise { amount: 1.0 },
                    AudioEffect::Denoise { amount: 1.0 }
                ],
                RATE
            ),
            2 * denoise
        );
        assert_eq!(lookahead_frames(&chain(), 0), 0);
    }

    #[test]
    #[ignore = "timing benchmark: cargo test --release -p fosscut-audio -- --ignored"]
    fn sixty_seconds_of_stereo_take_under_a_second() {
        let frames = seconds(60.0);
        let base = noisy_stereo(seconds(10.0));
        let input: Vec<f32> = base.iter().cycle().take(2 * frames).copied().collect();
        let effects = chain();
        let started = std::time::Instant::now();
        let output = process(&effects, &input, 2, RATE);
        let elapsed = started.elapsed();
        println!("60 s of 48 kHz stereo through all three effects: {elapsed:?}");
        assert_eq!(output.len(), input.len());
        assert!(elapsed.as_secs_f64() < 1.0, "took {elapsed:?}");
    }

    #[test]
    fn the_limiter_raises_the_level_and_holds_peaks_at_the_ceiling() {
        // A quiet tone with a loud burst in the middle, in stereo.
        let frames = 4 * RATE as usize;
        let mut input = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let t = i as f32 / RATE as f32;
            let level = if (1.5..1.7).contains(&t) { 0.9 } else { 0.1 };
            let v = level * (std::f32::consts::TAU * 220.0 * t).sin();
            input.extend([v, v * 0.5]);
        }
        let chain = [AudioEffect::Limiter {
            gain_db: 12.0,
            ceiling_db: -1.0,
        }];
        let out = process(&chain, &input, 2, RATE);
        let ceiling = 10f32.powf(-1.0 / 20.0);
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak <= ceiling * 1.0001, "peak {peak}");
        assert!(
            peak > ceiling * 0.98,
            "the burst reaches the ceiling: {peak}"
        );
        // Away from the burst the quiet tone is 12 dB louder, untouched by the limiter.
        let rms = |s: &[f32], from: f32, to: f32| {
            let (a, b) = (
                (from * RATE as f32) as usize * 2,
                (to * RATE as f32) as usize * 2,
            );
            (s[a..b].iter().step_by(2).map(|v| v * v).sum::<f32>() / ((b - a) / 2) as f32).sqrt()
        };
        let gain = rms(&out, 0.5, 1.0) / rms(&input, 0.5, 1.0);
        assert!((20.0 * gain.log10() - 12.0).abs() < 0.05, "{gain}");
        let after = rms(&out, 3.0, 3.5) / rms(&input, 3.0, 3.5);
        assert!(
            (20.0 * after.log10() - 12.0).abs() < 0.05,
            "recovered: {after}"
        );

        // No gain and nothing over the ceiling: unchanged. Windows match the whole.
        let still = process(
            &[AudioEffect::Limiter {
                gain_db: 0.0,
                ceiling_db: 0.0,
            }],
            &input,
            2,
            RATE,
        );
        assert!(still.iter().zip(&input).all(|(a, b)| (a - b).abs() < 1e-6));
        let look = lookahead_frames(&chain, RATE);
        assert_eq!(look, 240);
        let warm = (WARM_UP_SECONDS * f64::from(RATE)) as usize;
        let (from, to) = (RATE as usize * 3 - warm, RATE as usize * 3 + 4800);
        let window = process_at(
            &chain,
            &input[from * 2..(to + look) * 2],
            2,
            RATE,
            from as i64,
        );
        let skip = (3 * RATE as usize - from) * 2;
        let worst = window[skip..skip + 9600]
            .iter()
            .zip(&out[RATE as usize * 6..RATE as usize * 6 + 9600])
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst < 1e-4, "{worst}");
    }
}
