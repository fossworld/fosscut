//! Built-in sound effects and music loops, synthesized by code.
//!
//! FossCut ships its own library of sounds instead of a catalogue of recordings: every sound
//! here is made at render time from oscillators, filtered noise and envelopes (subtractive
//! and FM synthesis, pitch sweeps, decaying partials for bells), so nothing is downloaded and
//! every sound is FossCut's own, free to use in any video, commercial or not. [`sounds`]
//! lists the library and [`render`] synthesizes one sound as interleaved stereo.
//!
//! Rendering is deterministic: the same name and sample rate give bit-identical samples on
//! every call and machine. Noise comes from seeded integer generators, and sines,
//! exponentials and tangents are computed here from additions, multiplications and
//! divisions, which IEEE 754 rounds the same everywhere, instead of by the platform's maths
//! library, whose last digits differ between systems.
//!
//! Effects peak at -3 dBFS. Music loops are normalized to -16 LUFS integrated loudness
//! (ITU-R BS.1770 K-weighting with its absolute and relative gates), and a soft limiter keeps
//! their peaks under -1.5 dBFS. A loop lasts a whole number of bars and repeats seamlessly:
//! whatever rings past its end (note releases, echoes, reverb) is folded back onto its start,
//! and its continuous parts complete whole cycles within it, so a loop played twice in a row
//! sounds like one performance that kept going.

use std::f64::consts::{LN_2, LN_10, TAU};

/// What a built-in sound is for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SoundKind {
    /// A short sound effect, 0.1 to 3 seconds long.
    Effect,
    /// A music loop of whole bars that repeats seamlessly.
    Music,
}

/// A built-in sound.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundSpec {
    /// Stable name, for example "whoosh" or "upbeat-loop".
    pub name: &'static str,
    /// Display title.
    pub title: &'static str,
    /// Whether it is an effect or music.
    pub kind: SoundKind,
    /// What it sounds like and what it suits, one sentence.
    pub description: &'static str,
    /// Search words.
    pub tags: &'static [&'static str],
    /// Length in seconds of what `render` returns.
    pub seconds: f64,
    /// For music: tempo in beats per minute (loops repeat seamlessly at bar boundaries).
    pub bpm: Option<f64>,
}

/// Every built-in sound, effects first.
pub fn sounds() -> Vec<SoundSpec> {
    BUILTINS.iter().map(Builtin::spec).collect()
}

/// The sound called `name`, rendered as interleaved stereo f32 at `sample_rate`, the same
/// samples on every call and machine; `None` for an unknown name.
///
/// The result holds `round(seconds × sample_rate)` frames of two channels, where `seconds`
/// is the sound's [`SoundSpec::seconds`]; a rate of 0 gives no samples. Partials and filter
/// cutoffs above 45% of the rate are left out, so very low rates sound duller.
pub fn render(name: &str, sample_rate: u32) -> Option<Vec<f32>> {
    let builtin = BUILTINS.iter().find(|builtin| builtin.name == name)?;
    let rate = f64::from(sample_rate);
    let frames = frame_count(builtin.seconds(), rate);
    if frames == 0 {
        return Some(Vec::new());
    }
    let mut rng = Rng::new(seed(name));
    let (left, right) = match builtin.body {
        Body::Effect { make, .. } => {
            let mut out = Stereo::new(rate, frames);
            make(&mut out, &mut rng);
            finish_effect(out, frames)
        }
        Body::Music { bpm, make, .. } => {
            let mut song = Song::new(rate, bpm, frames, rng);
            make(&mut song);
            song.finish()
        }
    };
    let mut samples = Vec::with_capacity(frames * 2);
    for (l, r) in left.iter().zip(&right) {
        samples.push(*l as f32);
        samples.push(*r as f32);
    }
    Some(samples)
}

/// Peak level of effects: -3 dBFS.
const EFFECT_PEAK: f64 = 0.707_945_784_384_138;
/// Integrated loudness of music loops, in LUFS.
const MUSIC_LUFS: f64 = -16.0;
/// Music samples louder than this (about -5 dBFS) are softly compressed by the limiter.
const LIMIT_KNEE: f64 = 0.56;
/// The level (about -1.5 dBFS) that the limiter approaches and never reaches.
const LIMIT_CEILING: f64 = 0.84;
/// Beats in a bar: every loop is in 4/4.
const BEATS_PER_BAR: f64 = 4.0;
/// Samples between updates of slowly changing parameters (filter cutoffs, LFOs, pans).
const CONTROL: usize = 32;
/// The highest frequency synthesized, as a share of the sample rate.
const NYQUIST_SHARE: f64 = 0.45;
/// Decays and releases end when they reach this level (-60 dB).
const SILENT: f64 = 1e-3;
/// Time constants an exponential decay takes to reach [`SILENT`]: `ln(10^3)`.
const SILENT_SPAN: f64 = 6.91;
/// Echo and reverb tails are kept down to this level (-100 dB), so what is cut off where
/// they end lies below the noise floor of 16-bit audio.
const TAIL_FLOOR: f64 = 1e-5;

// Catalogue

/// One entry of the library.
struct Builtin {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    tags: &'static [&'static str],
    body: Body,
}

/// How a sound is made.
#[derive(Clone, Copy)]
enum Body {
    /// An effect of `seconds`, drawn into a stereo buffer.
    Effect {
        seconds: f64,
        make: fn(&mut Stereo, &mut Rng),
    },
    /// A loop of `bars` bars at `bpm`.
    Music {
        bpm: f64,
        bars: u32,
        make: fn(&mut Song),
    },
}

impl Builtin {
    fn seconds(&self) -> f64 {
        match self.body {
            Body::Effect { seconds, .. } => seconds,
            Body::Music { bpm, bars, .. } => f64::from(bars) * BEATS_PER_BAR * 60.0 / bpm,
        }
    }

    fn spec(&self) -> SoundSpec {
        let (kind, bpm) = match self.body {
            Body::Effect { .. } => (SoundKind::Effect, None),
            Body::Music { bpm, .. } => (SoundKind::Music, Some(bpm)),
        };
        SoundSpec {
            name: self.name,
            title: self.title,
            kind,
            description: self.description,
            tags: self.tags,
            seconds: self.seconds(),
            bpm,
        }
    }
}

const BUILTINS: [Builtin; 22] = [
    Builtin {
        name: "whoosh",
        title: "Whoosh",
        description: "A breathy rush of air that swells and passes from left to right, for \
                      transitions and fast camera moves.",
        tags: &["transition", "air", "swish", "fast", "movement"],
        body: Body::Effect {
            seconds: 1.1,
            make: whoosh,
        },
    },
    Builtin {
        name: "swoosh-down",
        title: "Swoosh Down",
        description: "A falling sweep of air, for things dropping out of frame or a scene \
                      winding down.",
        tags: &["transition", "fall", "drop", "sweep", "exit"],
        body: Body::Effect {
            seconds: 0.8,
            make: swoosh_down,
        },
    },
    Builtin {
        name: "pop",
        title: "Pop",
        description: "A short bubbly pop, for stickers, emoji and text popping into view.",
        tags: &["appear", "bubble", "cute", "ui", "sticker"],
        body: Body::Effect {
            seconds: 0.15,
            make: pop,
        },
    },
    Builtin {
        name: "click",
        title: "Click",
        description: "A crisp interface click, for taps, cursor clicks and button presses.",
        tags: &["ui", "tap", "button", "mouse", "tick"],
        body: Body::Effect {
            seconds: 0.1,
            make: click,
        },
    },
    Builtin {
        name: "ding",
        title: "Ding",
        description: "A bright desk-bell ding, for correct answers, ideas and highlights.",
        tags: &["bell", "success", "correct", "idea", "alert"],
        body: Body::Effect {
            seconds: 2.0,
            make: ding,
        },
    },
    Builtin {
        name: "chime",
        title: "Chime",
        description: "A rising four-note chime, for reveals, achievements and happy moments.",
        tags: &["success", "magic", "reveal", "bell", "positive"],
        body: Body::Effect {
            seconds: 2.5,
            make: chime,
        },
    },
    Builtin {
        name: "riser",
        title: "Riser",
        description: "A tense build of rising noise and pitch, for the moment before a drop, \
                      a reveal or a cut.",
        tags: &["build", "tension", "transition", "suspense", "uplifter"],
        body: Body::Effect {
            seconds: 2.6,
            make: riser,
        },
    },
    Builtin {
        name: "impact",
        title: "Impact",
        description: "A deep cinematic boom, for title hits, drops and dramatic cuts.",
        tags: &["boom", "hit", "bass", "trailer", "drop"],
        body: Body::Effect {
            seconds: 2.2,
            make: impact,
        },
    },
    Builtin {
        name: "glitch",
        title: "Glitch",
        description: "Stuttering digital noise and broken tones, for glitch transitions and \
                      tech looks.",
        tags: &["digital", "error", "tech", "transition", "stutter"],
        body: Body::Effect {
            seconds: 0.7,
            make: glitch,
        },
    },
    Builtin {
        name: "typewriter",
        title: "Typewriter",
        description: "A few mechanical typewriter key strikes, for text being typed on screen.",
        tags: &["typing", "keys", "text", "retro", "mechanical"],
        body: Body::Effect {
            seconds: 1.0,
            make: typewriter,
        },
    },
    Builtin {
        name: "camera-shutter",
        title: "Camera Shutter",
        description: "The two-part click of a camera shutter, for photos, freeze frames and \
                      snapshots.",
        tags: &["photo", "snapshot", "freeze", "camera", "click"],
        body: Body::Effect {
            seconds: 0.25,
            make: camera_shutter,
        },
    },
    Builtin {
        name: "notification",
        title: "Notification",
        description: "A friendly two-note alert, for messages, comments and pop-up \
                      notifications.",
        tags: &["alert", "message", "phone", "ui", "ping"],
        body: Body::Effect {
            seconds: 0.9,
            make: notification,
        },
    },
    Builtin {
        name: "coin",
        title: "Coin",
        description: "A retro game coin pickup, for points, rewards and money moments.",
        tags: &["game", "retro", "reward", "money", "8-bit"],
        body: Body::Effect {
            seconds: 0.7,
            make: coin,
        },
    },
    Builtin {
        name: "swipe",
        title: "Swipe",
        description: "A quick airy swipe, for sliding cards, wipes and fast text moves.",
        tags: &["transition", "slide", "fast", "ui", "air"],
        body: Body::Effect {
            seconds: 0.35,
            make: swipe,
        },
    },
    Builtin {
        name: "drum-hit",
        title: "Drum Hit",
        description: "A punchy drum hit, for beats, emphasis and punchlines.",
        tags: &["drum", "hit", "beat", "punch", "accent"],
        body: Body::Effect {
            seconds: 1.0,
            make: drum_hit,
        },
    },
    Builtin {
        name: "sparkle",
        title: "Sparkle",
        description: "A shimmer of tiny high bells, for magic, shine and glamour moments.",
        tags: &["magic", "shine", "glitter", "twinkle", "fairy"],
        body: Body::Effect {
            seconds: 1.6,
            make: sparkle,
        },
    },
    Builtin {
        name: "upbeat-loop",
        title: "Upbeat Pop",
        description: "A bright four-on-the-floor pop groove with pumping chords and a catchy \
                      lead, for vlogs, promos and fast cuts.",
        tags: &["pop", "happy", "energetic", "dance", "vlog"],
        body: Body::Music {
            bpm: 120.0,
            bars: 8,
            make: upbeat,
        },
    },
    Builtin {
        name: "chill-loop",
        title: "Chill Lo-fi",
        description: "A laid-back lo-fi beat with soft drums, warm electric piano chords and \
                      vinyl crackle, for study, cooking and relaxed vlogs.",
        tags: &["lofi", "relaxed", "calm", "study", "hip-hop"],
        body: Body::Music {
            bpm: 85.0,
            bars: 8,
            make: chill,
        },
    },
    Builtin {
        name: "cinematic-loop",
        title: "Cinematic Pulse",
        description: "Slow string pads over a driving low pulse and deep drums, for trailers, \
                      documentaries and dramatic intros.",
        tags: &["epic", "trailer", "dramatic", "tension", "film"],
        body: Body::Music {
            bpm: 80.0,
            bars: 8,
            make: cinematic,
        },
    },
    Builtin {
        name: "corporate-loop",
        title: "Corporate Bright",
        description: "Bright arpeggiated plucks over a light, steady beat, for explainers, \
                      product demos and presentations.",
        tags: &["business", "positive", "tech", "explainer", "inspiring"],
        body: Body::Music {
            bpm: 110.0,
            bars: 8,
            make: corporate,
        },
    },
    Builtin {
        name: "trap-loop",
        title: "Trap Night",
        description: "A dark half-time trap beat with booming 808 bass, rolling hats and a \
                      bell melody, for streetwear, gaming and edits.",
        tags: &["hip-hop", "808", "dark", "urban", "edit"],
        body: Body::Music {
            bpm: 140.0,
            bars: 8,
            make: trap,
        },
    },
    Builtin {
        name: "ambient-loop",
        title: "Ambient Drift",
        description: "A slowly evolving pad with distant bell tones, for calm backgrounds, \
                      nature and meditation.",
        tags: &["calm", "atmosphere", "meditation", "background", "dreamy"],
        body: Body::Music {
            bpm: 70.0,
            bars: 8,
            make: ambient,
        },
    },
];

// Effects
//
// Each effect draws into a buffer of its exact length; `finish_effect` fades its last few
// milliseconds and scales it to peak at -3 dBFS.

/// The effect's buffer cut to `frames`, faded out at its end and scaled to [`EFFECT_PEAK`].
fn finish_effect(mut out: Stereo, frames: usize) -> (Vec<f64>, Vec<f64>) {
    out.left.resize(frames, 0.0);
    out.right.resize(frames, 0.0);
    let fade = frame_count(0.03, out.rate).min(frames / 10).max(1);
    for k in 0..fade {
        let gain = k as f64 / fade as f64;
        let n = frames - 1 - k;
        out.left[n] *= gain;
        out.right[n] *= gain;
    }
    let peak = out
        .left
        .iter()
        .chain(&out.right)
        .fold(0.0f64, |peak, x| peak.max(x.abs()));
    if peak > 0.0 {
        let gain = EFFECT_PEAK / peak;
        for x in out.left.iter_mut().chain(out.right.iter_mut()) {
            *x *= gain;
        }
    }
    (out.left, out.right)
}

/// A band of noise that swells and fades while its centre rises and falls and it pans
/// across the stereo field: the shape of air rushing past.
struct AirSweep {
    seconds: f64,
    /// Where the swell peaks, as a share of the length.
    peak_at: f64,
    /// Centre of the band at the start and end, in Hz.
    low: f64,
    /// How far the centre rises at the peak, in octaves.
    octaves: f64,
    q: f64,
    /// Level of low-passed noise under the band, for body.
    body: f64,
    pan_from: f64,
    pan_to: f64,
}

fn air_sweep(out: &mut Stereo, rng: &mut Rng, sweep: &AirSweep) {
    let rate = out.rate;
    let n = out.frames(sweep.seconds);
    // p (1 + k (1 - p)) runs from 0 to 1 and passes 1/2 at `peak_at`.
    let k = (0.5 / sweep.peak_at - 1.0) / (1.0 - sweep.peak_at);
    // Two band-passes in series, for skirts steep enough that the sweep is heard.
    let (mut band_1, mut band_2) = (Svf::default(), Svf::default());
    let mut body = OnePole::new(450.0, rate);
    let mut gains = (0.0, 0.0);
    let (mut left, mut right) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let p = i as f64 / n as f64;
        let swell = sin_turns(0.5 * p * (1.0 + k * (1.0 - p)));
        if i % CONTROL == 0 {
            let centre = sweep.low * exp2(sweep.octaves * swell);
            band_1.tune(centre, sweep.q, rate);
            band_2.tune(centre, sweep.q, rate);
            gains = pan_gains(sweep.pan_from + (sweep.pan_to - sweep.pan_from) * p);
        }
        let x = rng.noise();
        let air = band_2.band(band_1.band(x));
        let s = (air + sweep.body * body.low(x)) * swell * swell;
        left.push(s * gains.0);
        right.push(s * gains.1);
    }
    out.add_pair(0, &left, &right, 1.0, 1.0);
}

fn whoosh(out: &mut Stereo, rng: &mut Rng) {
    let sweep = AirSweep {
        seconds: 1.1,
        peak_at: 0.42,
        low: 260.0,
        octaves: 3.4,
        q: 1.3,
        body: 0.9,
        pan_from: -0.8,
        pan_to: 0.8,
    };
    air_sweep(out, rng, &sweep);
}

fn swipe(out: &mut Stereo, rng: &mut Rng) {
    let sweep = AirSweep {
        seconds: 0.35,
        peak_at: 0.35,
        low: 1400.0,
        octaves: 2.3,
        q: 1.6,
        body: 0.3,
        pan_from: -0.7,
        pan_to: 0.7,
    };
    air_sweep(out, rng, &sweep);
}

fn swoosh_down(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    let n = out.frames(0.8);
    let attack = out.frames(0.04).max(1);
    let mut band = Svf::default();
    let (mut gains, mut zip, mut zip_step) = ((0.0, 0.0), 0.0, 0.0);
    let (mut left, mut right) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let p = i as f64 / n as f64;
        if i % CONTROL == 0 {
            band.tune(5500.0 * exp2(-4.2 * p), 1.5, rate);
            gains = pan_gains(0.5 - p);
            zip_step = 1400.0 * exp2(-3.0 * p) / rate;
        }
        let fall = 1.0 - p;
        let s = (band.band(rng.noise()) + 0.12 * sin_turns(zip)) * ramp(i, attack) * fall * fall;
        zip = wrap(zip + zip_step);
        left.push(s * gains.0);
        right.push(s * gains.1);
    }
    out.add_pair(0, &left, &right, 1.0, 1.0);
}

fn pop(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    let n = out.frames(0.15);
    let attack = out.frames(0.0006).max(1);
    let (rise, fade) = (decay_coef(0.012, rate), decay_coef(0.022, rate));
    let (mut phase, mut s, mut a) = (0.0, 1.0, 1.0);
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        sound.push(sin_turns(phase) * a * ramp(i, attack));
        // The pitch bubbles up from 320 Hz to about 970 Hz.
        phase = wrap(phase + 320.0 * exp2(1.6 * (1.0 - s)) / rate);
        s *= rise;
        a *= fade;
    }
    out.add(0, &sound, 0.0, 1.0);
    out.add(0, &tick(rate, rng, 0.0008, 1500.0), 0.0, 0.3);
}

fn click(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    let tone = bell(rate, 3200.0, &[(1.0, 0.6, 0.004), (0.39, 0.4, 0.007)], 0.1);
    out.add(0, &tone, 0.0, 1.0);
    out.add(0, &tick(rate, rng, 0.0007, 2000.0), 0.0, 0.8);
}

/// A small hemispherical desk bell: inharmonic partials, the higher ones dying faster.
const DESK_BELL: [(f64, f64, f64); 4] = [
    (1.0, 1.0, 0.45),
    (2.32, 0.5, 0.22),
    (4.18, 0.28, 0.12),
    (6.35, 0.12, 0.07),
];

fn ding(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    let pitch = hz(98.0);
    out.add(0, &bell(rate, pitch, &DESK_BELL, 2.0), -0.15, 1.0);
    // A slightly detuned copy of the modes beats against them, as in a real bell.
    out.add(0, &bell(rate, pitch * 1.0007, &DESK_BELL, 2.0), 0.3, 0.45);
    out.add(0, &tick(rate, rng, 0.002, 3000.0), 0.0, 0.35);
}

/// A tubular chime: the modes of a free bar.
const TUBE: [(f64, f64, f64); 4] = [
    (1.0, 1.0, 0.6),
    (2.76, 0.35, 0.3),
    (5.4, 0.15, 0.16),
    (8.93, 0.06, 0.08),
];

fn chime(out: &mut Stereo, _rng: &mut Rng) {
    let rate = out.rate;
    for (k, midi) in [84.0, 88.0, 91.0, 96.0].into_iter().enumerate() {
        let k = k as f64;
        let delay = 0.09 * k;
        let tone = bell(rate, hz(midi), &TUBE, 2.5 - delay);
        out.add(out.frames(delay), &tone, 0.3 * k - 0.45, 1.0 - 0.1 * k);
    }
}

fn riser(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    let n = out.frames(2.6);
    let fade = out.frames(0.03).max(1);
    let mut other = rng.fork();
    let (mut band_l, mut band_r) = (Svf::default(), Svf::default());
    let (mut tone_l, mut tone_r) = (Svf::default(), Svf::default());
    let (mut saw_l, mut saw_r, mut tremolo, mut step) = (0.0, 0.25, 0.0, 0.0);
    let (mut left, mut right) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let p = i as f64 / n as f64;
        if i % CONTROL == 0 {
            let centre = 180.0 * exp2(5.6 * p);
            let q = 0.8 + 2.4 * p;
            band_l.tune(centre, q, rate);
            band_r.tune(centre * 1.06, q, rate);
            let cutoff = 300.0 * exp2(4.6 * p);
            tone_l.tune(cutoff, 1.2, rate);
            tone_r.tune(cutoff, 1.2, rate);
            step = 110.0 * exp2(3.0 * p * p) / rate;
        }
        // Louder and faster tremolo towards the end, cut off after a short fade.
        let trem = 0.8 + 0.2 * sin_turns(tremolo);
        tremolo = wrap(tremolo + (4.0 + 22.0 * p) / rate);
        let env = (0.03 + 0.97 * p * p) * trem * ramp(n - 1 - i, fade);
        let (up, down) = (step * 1.007, step / 1.007);
        let l = 0.7 * band_l.band(rng.noise()) + 0.35 * tone_l.low(saw(saw_l, up));
        let r = 0.7 * band_r.band(other.noise()) + 0.35 * tone_r.low(saw(saw_r, down));
        saw_l = wrap(saw_l + up);
        saw_r = wrap(saw_r + down);
        left.push(l * env);
        right.push(r * env);
    }
    out.add_pair(0, &left, &right, 1.0, 1.0);
}

fn impact(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    let n = out.frames(2.2);
    let attack = out.frames(0.0015).max(1);
    let (sweep, fade) = (decay_coef(0.07, rate), decay_coef(0.5, rate));
    let (crack_fade, rumble_fade) = (decay_coef(0.03, rate), decay_coef(0.6, rate));
    let (mut crack_1, mut crack_2) = (OnePole::new(1200.0, rate), OnePole::new(1200.0, rate));
    let (mut rumble_1, mut rumble_2) = (OnePole::new(90.0, rate), OnePole::new(90.0, rate));
    let norm = 1.0 / saturate(1.8);
    let (mut phase, mut s, mut a, mut c, mut u) = (0.0, 1.0, 1.0, 1.0, 1.0);
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        // A sine falling from about 128 Hz to 33 Hz, driven into soft saturation.
        let boom = saturate(1.8 * sin_turns(phase)) * norm * a;
        let crack = crack_2.low(crack_1.low(rng.noise())) * c * 0.9;
        let rumble = rumble_2.low(rumble_1.low(rng.noise())) * u * 5.0;
        sound.push((boom + crack + rumble) * ramp(i, attack));
        phase = wrap(phase + (33.0 + 95.0 * s) / rate);
        s *= sweep;
        a *= fade;
        c *= crack_fade;
        u *= rumble_fade;
    }
    out.add(0, &sound, 0.0, 1.0);
}

/// One slice of a glitch.
#[derive(Clone, Copy)]
enum Fragment {
    /// A bit-crushed saw at `hz` falling an octave, with `levels` steps per half wave.
    Tone { hz: f64, levels: f64 },
    /// Noise held for `hold` seconds at a time, from its own seed.
    Crushed { hold: f64, seed: u64 },
    /// An electric buzz at `hz`.
    Buzz { hz: f64 },
    /// Silence.
    Gap,
}

impl Fragment {
    fn random(rng: &mut Rng) -> Fragment {
        let pick = rng.unit();
        if pick < 0.4 {
            Fragment::Tone {
                hz: 200.0 * exp2(rng.range(0.0, 4.0)),
                levels: [2.0, 3.0, 4.0, 6.0][rng.below(4)],
            }
        } else if pick < 0.75 {
            Fragment::Crushed {
                hold: rng.range(0.0002, 0.002),
                seed: rng.next(),
            }
        } else if pick < 0.87 {
            Fragment::Buzz {
                hz: rng.range(50.0, 120.0),
            }
        } else {
            Fragment::Gap
        }
    }

    fn render(self, rate: f64, n: usize) -> Vec<f64> {
        let edge = frame_count(0.0003, rate).max(1);
        let mut phase = 0.0;
        let mut held = 0.0;
        let (mut rng, hold) = match self {
            Fragment::Crushed { hold, seed } => (Rng::new(seed), frame_count(hold, rate).max(1)),
            _ => (Rng::new(0), 1),
        };
        let mut sound = Vec::with_capacity(n);
        for i in 0..n {
            let p = i as f64 / n as f64;
            let x = match self {
                Fragment::Tone { hz, levels } => {
                    let x = ((2.0 * phase - 1.0) * levels).round() / levels;
                    phase = wrap(phase + hz * (1.0 - 0.5 * p) / rate);
                    x * 0.7
                }
                Fragment::Crushed { .. } => {
                    if i % hold == 0 {
                        held = rng.noise();
                    }
                    held * 0.8
                }
                Fragment::Buzz { hz } => {
                    let x = if phase < 0.5 { 0.5 } else { -0.5 } + 0.4 * sin_turns(phase);
                    phase = wrap(phase + hz / rate);
                    x
                }
                Fragment::Gap => 0.0,
            };
            sound.push(x * ramp(i, edge) * ramp(n - 1 - i, edge));
        }
        sound
    }
}

fn glitch(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    let total = out.frames(0.7);
    let mut at = 0;
    let mut previous: Option<(Fragment, f64, f64)> = None;
    while at < total {
        let n = out.frames([0.018, 0.025, 0.035, 0.05][rng.below(4)]).max(1);
        // Now and then the last slice repeats: the stutter of a skipping buffer.
        let slice = match previous {
            Some(slice) if rng.unit() < 0.3 => slice,
            _ => (
                Fragment::random(rng),
                rng.range(-0.8, 0.8),
                rng.range(0.5, 1.0),
            ),
        };
        out.add(at, &slice.0.render(rate, n), slice.1, slice.2);
        at += n;
        previous = Some(slice);
    }
}

/// One typewriter key: the snap of the type bar, the ring of the platen and the thunk of
/// the key going down.
fn key_strike(rate: f64, rng: &mut Rng) -> Vec<f64> {
    let n = frame_count(0.09, rate);
    let attack = frame_count(0.0001, rate).max(1);
    let mut high = OnePole::new(3000.0, rate);
    let mut ring = Svf::default();
    ring.tune(2100.0 * rng.range(0.92, 1.08), 5.0, rate);
    let (snap_fade, ring_fade) = (decay_coef(0.0015, rate), decay_coef(0.009, rate));
    let (thunk_fade, drop) = (decay_coef(0.018, rate), decay_coef(0.006, rate));
    let (mut a, mut b, mut c, mut s, mut phase) = (1.0, 1.0, 1.0, 1.0, 0.0);
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        let x = rng.noise();
        let v = high.high(x) * a * 0.9 + ring.band(x) * b * 2.0 + sin_turns(phase) * c * 0.6;
        sound.push(v * ramp(i, attack));
        phase = wrap(phase + (140.0 + 100.0 * s) / rate);
        a *= snap_fade;
        b *= ring_fade;
        c *= thunk_fade;
        s *= drop;
    }
    fade_end(&mut sound, rate);
    sound
}

fn typewriter(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    let mut time = 0.03;
    for _ in 0..6 {
        let velocity = rng.range(0.75, 1.0);
        let pan = rng.range(-0.3, 0.3);
        let strike = key_strike(rate, rng);
        out.add(out.frames(time), &strike, pan, velocity);
        // The key coming back up, much quieter.
        let release = tick(rate, rng, 0.0012, 2500.0);
        let up = time + rng.range(0.05, 0.07);
        out.add(out.frames(up), &release, pan, 0.2 * velocity);
        time += rng.range(0.11, 0.19);
    }
}

/// One click of a shutter mechanism: a snap, a metallic ring and a small body knock.
fn shutter_click(rate: f64, rng: &mut Rng, ring_hz: f64, body_hz: f64) -> Vec<f64> {
    let n = frame_count(0.06, rate);
    let attack = frame_count(0.0001, rate).max(1);
    let mut high = OnePole::new(2500.0, rate);
    let mut ring = Svf::default();
    ring.tune(ring_hz, 3.0, rate);
    let (snap_fade, ring_fade, body_fade) = (
        decay_coef(0.0012, rate),
        decay_coef(0.005, rate),
        decay_coef(0.012, rate),
    );
    let (mut a, mut b, mut c, mut phase) = (1.0, 1.0, 1.0, 0.0);
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        let x = rng.noise();
        let v = high.high(x) * a + ring.band(x) * b * 1.5 + sin_turns(phase) * c * 0.5;
        sound.push(v * ramp(i, attack));
        phase = wrap(phase + body_hz / rate);
        a *= snap_fade;
        b *= ring_fade;
        c *= body_fade;
    }
    fade_end(&mut sound, rate);
    sound
}

fn camera_shutter(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    let open = shutter_click(rate, rng, 4200.0, 650.0);
    let close = shutter_click(rate, rng, 3300.0, 520.0);
    // The whirr of the mirror and curtain between the two clicks.
    let n = out.frames(0.07);
    let edge = out.frames(0.01).max(1);
    let mut band = Svf::default();
    band.tune(1600.0, 1.2, rate);
    let whirr: Vec<f64> = (0..n)
        .map(|i| band.band(rng.noise()) * ramp(i, edge) * ramp(n - 1 - i, edge))
        .collect();
    out.add(0, &open, -0.1, 1.0);
    out.add(out.frames(0.012), &whirr, 0.0, 0.12);
    out.add(out.frames(0.09), &close, 0.1, 0.85);
}

/// A soft bell-like tone: two-operator FM with the modulator an octave up.
const NOTIFY: Patch = Patch {
    wave: Wave::Fm {
        ratio: 2.0,
        index: 1.6,
        floor: 0.2,
        decay: 0.08,
    },
    attack: 0.004,
    decay: 0.18,
    sustain: 0.0,
    release: 0.05,
    ..PLAIN
};

fn notification(out: &mut Stereo, _rng: &mut Rng) {
    let rate = out.rate;
    let first = note(rate, hz(81.0), 0.3, &NOTIFY, 0.0);
    let second = note(rate, hz(88.0), 0.7, &NOTIFY, 0.0);
    out.add(0, &first, -0.15, 0.85);
    out.add(out.frames(0.13), &second, 0.15, 1.0);
}

/// A narrow pulse wave, the sound of early game consoles.
const COIN_FIRST: Patch = Patch {
    wave: Wave::Pulse(0.25),
    attack: 0.001,
    release: 0.006,
    cutoff: 7000.0,
    ..PLAIN
};

const COIN_SECOND: Patch = Patch {
    decay: 0.17,
    sustain: 0.0,
    release: 0.02,
    ..COIN_FIRST
};

fn coin(out: &mut Stereo, _rng: &mut Rng) {
    let rate = out.rate;
    // A5 then the fifth above it, E6.
    out.add(0, &note(rate, hz(81.0), 0.075, &COIN_FIRST, 0.0), 0.0, 0.8);
    let second = note(rate, hz(88.0), 0.6, &COIN_SECOND, 0.0);
    out.add(out.frames(0.075), &second, 0.0, 1.0);
}

fn drum_hit(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    let n = out.frames(1.0);
    let attack = out.frames(0.001).max(1);
    let (sweep, fade, rattle_fade) = (
        decay_coef(0.035, rate),
        decay_coef(0.28, rate),
        decay_coef(0.09, rate),
    );
    let mut rattle = Svf::default();
    rattle.tune(1800.0, 0.8, rate);
    let norm = 1.0 / saturate(1.4);
    let (mut phase, mut s, mut a, mut b) = (0.0, 1.0, 1.0, 1.0);
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        let tom = saturate(1.4 * sin_turns(phase)) * norm * a;
        let noise = rattle.band(rng.noise()) * b * 0.45;
        sound.push((tom + noise) * ramp(i, attack));
        phase = wrap(phase + (82.0 + 120.0 * s) / rate);
        s *= sweep;
        a *= fade;
        b *= rattle_fade;
    }
    out.add(0, &sound, 0.0, 1.0);
    out.add(0, &tick(rate, rng, 0.001, 2000.0), 0.0, 0.4);
}

/// C major pentatonic from C7 up to G8.
const SPARKLE_NOTES: [f64; 9] = [96.0, 98.0, 100.0, 103.0, 105.0, 108.0, 110.0, 112.0, 115.0];

fn sparkle(out: &mut Stereo, rng: &mut Rng) {
    let rate = out.rate;
    for _ in 0..28 {
        let u = rng.unit();
        let time = 1.1 * u * u;
        let midi = SPARKLE_NOTES[rng.below(SPARKLE_NOTES.len())];
        let tau = rng.range(0.05, 0.14);
        let ping = bell(
            rate,
            hz(midi),
            &[(1.0, 1.0, tau), (2.0, 0.2, 0.5 * tau)],
            0.6,
        );
        let gain = rng.range(0.4, 1.0) * (1.0 - 0.4 * time);
        out.add(out.frames(time), &ping, rng.range(-0.8, 0.8), 0.5 * gain);
    }
    // A faint wash of high noise under the bells.
    let n = out.frames(1.2);
    let (mut high_1, mut high_2) = (OnePole::new(7000.0, rate), OnePole::new(7000.0, rate));
    let (mut left, mut right) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut other = rng.fork();
    for i in 0..n {
        let swell = sin_turns(0.5 * i as f64 / n as f64);
        left.push(high_1.high(rng.noise()) * swell);
        right.push(high_2.high(other.noise()) * swell);
    }
    out.add_pair(0, &left, &right, 0.06, 0.06);
}

/// A burst of high-passed noise decaying with time constant `tau`: the transient of a
/// click, a strike or a snap.
fn tick(rate: f64, rng: &mut Rng, tau: f64, cutoff: f64) -> Vec<f64> {
    let n = frame_count(tau * SILENT_SPAN, rate).max(1);
    let attack = frame_count(0.0001, rate).max(1);
    let fade = decay_coef(tau, rate);
    let mut high = OnePole::new(cutoff, rate);
    let mut a = 1.0;
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        sound.push(high.high(rng.noise()) * a * ramp(i, attack));
        a *= fade;
    }
    fade_end(&mut sound, rate);
    sound
}

/// A struck bell or bar, `seconds` long: sine partials at `ratio` times `hz` with amplitude
/// `amp`, each decaying with its own time constant (seconds), as the higher modes of a
/// struck body lose their energy faster. Partials near or above Nyquist are left out.
fn bell(rate: f64, hz: f64, partials: &[(f64, f64, f64)], seconds: f64) -> Vec<f64> {
    let n = frame_count(seconds, rate);
    let attack = frame_count(0.001, rate).max(1);
    let mut sound = vec![0.0; n];
    for &(ratio, amp, tau) in partials {
        let f = hz * ratio;
        if f >= NYQUIST_SHARE * rate {
            continue;
        }
        let (step, fade) = (f / rate, decay_coef(tau, rate));
        let (mut phase, mut a) = (0.0, amp);
        let audible = frame_count(tau * SILENT_SPAN, rate).min(n);
        let end = frame_count(END_FADE, rate).clamp(1, audible.max(1));
        for (i, s) in sound.iter_mut().enumerate().take(audible) {
            *s += sin_turns(phase) * a * ramp(i, attack) * ramp(audible - 1 - i, end);
            phase = wrap(phase + step);
            a *= fade;
        }
    }
    sound
}

// Instruments
//
// A note is one voice of a patch: an oscillator (sine, band-limited saw or pulse, or
// two-operator FM) through a resonant low-pass whose cutoff can sweep down after the attack
// or follow an LFO, shaped by an attack-decay-sustain-release envelope and optionally driven
// into soft saturation.

/// An oscillator.
#[derive(Clone, Copy)]
enum Wave {
    Sine,
    /// A sawtooth, band-limited with PolyBLEP.
    Saw,
    /// A pulse with the given duty cycle, band-limited with PolyBLEP.
    Pulse(f64),
    /// A sine carrier modulated by a sine at `ratio` times its pitch, with a modulation index
    /// (radians) that decays from `index` towards `floor` with time constant `decay`.
    Fm {
        ratio: f64,
        index: f64,
        floor: f64,
        decay: f64,
    },
}

/// A synthesizer voice. Times are in seconds; decays and releases are time constants.
#[derive(Clone, Copy)]
struct Patch {
    wave: Wave,
    /// Linear rise from silence.
    attack: f64,
    /// Exponential fall from the peak towards `sustain`.
    decay: f64,
    sustain: f64,
    /// Exponential fall after the note ends.
    release: f64,
    /// Low-pass cutoff in Hz; infinite for no filter.
    cutoff: f64,
    resonance: f64,
    /// Octaves above `cutoff` the filter starts at, falling back with `sweep_decay`.
    sweep: f64,
    sweep_decay: f64,
    /// Frequency of the LFO, in Hz.
    lfo_hz: f64,
    /// Pitch modulation by the LFO, in cents.
    vibrato: f64,
    /// Cutoff modulation by the LFO, in octaves.
    wah: f64,
    /// Soft saturation: 0 is clean.
    drive: f64,
    /// Semitones above the pitch the note starts at, sliding down within about 40 ms.
    bend: f64,
}

const PLAIN: Patch = Patch {
    wave: Wave::Sine,
    attack: 0.005,
    decay: 1.0,
    sustain: 1.0,
    release: 0.05,
    cutoff: f64::INFINITY,
    resonance: 0.7,
    sweep: 0.0,
    sweep_decay: 0.1,
    lfo_hz: 0.0,
    vibrato: 0.0,
    wah: 0.0,
    drive: 0.0,
    bend: 0.0,
};

/// Time constant of [`Patch::bend`], in seconds.
const BEND_SECONDS: f64 = 0.04;

/// One note of `patch` at `hz` held for `gate` seconds, until its release has faded to -60
/// dB (or earlier, when it has no sustain and has decayed that far). `lfo` is the LFO's
/// phase in turns at the note's start. Notes near or above Nyquist are silent.
fn note(rate: f64, hz: f64, gate: f64, patch: &Patch, lfo: f64) -> Vec<f64> {
    if hz <= 0.0 || hz >= NYQUIST_SHARE * rate {
        return Vec::new();
    }
    let gate = frame_count(gate, rate).max(1);
    let attack = frame_count(patch.attack, rate).max(1);
    let total = gate + frame_count(patch.release * SILENT_SPAN, rate).max(1);
    let (decay, release) = (
        decay_coef(patch.decay, rate),
        decay_coef(patch.release, rate),
    );
    let (sweep, bend) = (
        decay_coef(patch.sweep_decay, rate),
        decay_coef(BEND_SECONDS, rate),
    );
    let bend_depth = exp2(patch.bend / 12.0) - 1.0;
    let lfo_step = patch.lfo_hz / rate * CONTROL as f64;
    let vibrato = patch.vibrato / 1200.0 * LN_2;
    let filtered = patch.cutoff.is_finite();
    let moving = patch.sweep != 0.0 || patch.wah != 0.0;
    let mut filter = Svf::default();
    if filtered {
        filter.tune(patch.cutoff * exp2(patch.sweep), patch.resonance, rate);
    }
    let drive_norm = if patch.drive > 0.0 {
        1.0 / saturate(1.0 + patch.drive)
    } else {
        1.0
    };
    let (ratio, index, floor, index_decay) = match patch.wave {
        Wave::Fm {
            ratio,
            index,
            floor,
            decay,
        } => (ratio, index, floor, decay_coef(decay, rate)),
        _ => (0.0, 0.0, 0.0, 1.0),
    };
    let base = hz / rate;
    let (mut phase, mut mod_phase, mut lfo_phase, mut lfo_value) = (0.0, 0.0, lfo, 0.0);
    // Envelope states: decay, release, filter sweep, pitch bend, FM index.
    let (mut d, mut r, mut s, mut b, mut m) = (1.0, 1.0, 1.0, 1.0, 1.0);
    let mut held = 0.0;
    let mut sound = Vec::with_capacity(total);
    for i in 0..total {
        if i % CONTROL == 0 {
            if lfo_step > 0.0 {
                lfo_value = sin_turns(lfo_phase);
                lfo_phase = wrap(lfo_phase + lfo_step);
            }
            if filtered && moving {
                let octaves = patch.sweep * s + patch.wah * lfo_value;
                filter.tune(patch.cutoff * exp2(octaves), patch.resonance, rate);
            }
        }
        let level = if i < gate {
            held = if i < attack {
                i as f64 / attack as f64
            } else {
                let level = patch.sustain + (1.0 - patch.sustain) * d;
                d *= decay;
                level
            };
            held
        } else {
            r *= release;
            held * r
        };
        if patch.sustain <= 0.0 && i > attack && level < SILENT {
            break;
        }
        // exp2(cents / 1200 × lfo) to first order: the error is below 0.1 cent.
        let step = base * (1.0 + vibrato * lfo_value) * (1.0 + bend_depth * b);
        let x = match patch.wave {
            Wave::Sine => sin_turns(phase),
            Wave::Saw => saw(phase, step),
            Wave::Pulse(duty) => saw(phase, step) - saw(wrap(phase + duty), step),
            Wave::Fm { .. } => {
                let depth = floor + (index - floor) * m;
                m *= index_decay;
                sin_turns(phase + depth / TAU * sin_turns(mod_phase))
            }
        };
        phase = wrap(phase + step);
        mod_phase = wrap(mod_phase + step * ratio);
        let x = if filtered { filter.low(x) } else { x };
        let x = if patch.drive > 0.0 {
            saturate(x * (1.0 + patch.drive)) * drive_norm
        } else {
            x
        };
        sound.push(x * level);
        s *= sweep;
        b *= bend;
    }
    fade_end(&mut sound, rate);
    sound
}

/// A kick drum: a sine swept down from `top` to `base` Hz, decaying with time constant
/// `decay`, over a soft noise click of level `click`.
fn kick(rate: f64, rng: &mut Rng, base: f64, top: f64, decay: f64, click: f64) -> Vec<f64> {
    let n = frame_count(decay * SILENT_SPAN, rate).max(1);
    let attack = frame_count(0.001, rate).max(1);
    let (sweep, fade, snap) = (
        decay_coef(0.03, rate),
        decay_coef(decay, rate),
        decay_coef(0.0015, rate),
    );
    let mut tone = OnePole::new(2500.0, rate);
    let (mut phase, mut s, mut a, mut c) = (0.0, 1.0, 1.0, click);
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        let x = sin_turns(phase) * a + tone.low(rng.noise()) * c;
        sound.push(x * ramp(i, attack));
        phase = wrap(phase + (base + (top - base) * s) / rate);
        s *= sweep;
        a *= fade;
        c *= snap;
    }
    fade_end(&mut sound, rate);
    sound
}

/// A snare: a short two-mode drum tone at `tone` Hz under noise that decays with time
/// constant `decay` and is low-passed at `bright` Hz.
fn snare(rate: f64, rng: &mut Rng, tone: f64, decay: f64, bright: f64) -> Vec<f64> {
    let n = frame_count(decay.max(0.045) * SILENT_SPAN, rate).max(1);
    let attack = frame_count(0.0008, rate).max(1);
    let (tone_fade, noise_fade) = (decay_coef(0.045, rate), decay_coef(decay, rate));
    let (mut high, mut low) = (OnePole::new(900.0, rate), OnePole::new(bright, rate));
    let (mut p1, mut p2, mut a, mut b) = (0.0, 0.0, 1.0, 1.0);
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        let body = (0.6 * sin_turns(p1) + 0.3 * sin_turns(p2)) * a;
        let noise = low.low(high.high(rng.noise())) * b * 1.6;
        sound.push((body + noise) * ramp(i, attack));
        p1 = wrap(p1 + tone / rate);
        p2 = wrap(p2 + 1.62 * tone / rate);
        a *= tone_fade;
        b *= noise_fade;
    }
    fade_end(&mut sound, rate);
    sound
}

/// A hi-hat: high-passed noise decaying with time constant `decay`, low-passed at `bright`.
fn hat(rate: f64, rng: &mut Rng, decay: f64, bright: f64) -> Vec<f64> {
    let n = frame_count(decay * SILENT_SPAN, rate).max(1);
    let attack = frame_count(0.0004, rate).max(1);
    let fade = decay_coef(decay, rate);
    let (mut high_1, mut high_2) = (OnePole::new(7000.0, rate), OnePole::new(7000.0, rate));
    let mut low = OnePole::new(bright, rate);
    let mut a = 1.0;
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        let x = low.low(high_2.high(high_1.high(rng.noise())));
        sound.push(x * a * ramp(i, attack) * 2.0);
        a *= fade;
    }
    fade_end(&mut sound, rate);
    sound
}

/// A hand clap: three quick bursts of band-passed noise and a short tail.
fn clap(rate: f64, rng: &mut Rng) -> Vec<f64> {
    let n = frame_count(0.4, rate);
    let starts = [0.0, 0.009, 0.018, 0.026].map(|t| frame_count(t, rate));
    let (burst, tail) = (decay_coef(0.003, rate), decay_coef(0.085, rate));
    let attack = frame_count(0.0003, rate).max(1);
    let mut band = Svf::default();
    band.tune(1150.0, 1.2, rate);
    let mut levels = [0.0; 4];
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        for (k, level) in levels.iter_mut().enumerate() {
            if i == starts[k] {
                *level = if k == 3 { 0.7 } else { 1.0 };
            }
            *level *= if k == 3 { tail } else { burst };
        }
        let env: f64 = levels.iter().sum();
        sound.push(band.band(rng.noise()) * env * ramp(i, attack) * 1.5);
    }
    fade_end(&mut sound, rate);
    sound
}

/// A shaker: band-passed high noise that swells for a few milliseconds and decays.
fn shaker(rate: f64, rng: &mut Rng, decay: f64) -> Vec<f64> {
    let n = frame_count(decay * SILENT_SPAN + 0.008, rate);
    let attack = frame_count(0.008, rate).max(1);
    let fade = decay_coef(decay, rate);
    let mut band = Svf::default();
    band.tune(6500.0, 1.0, rate);
    let mut a = 1.0;
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        sound.push(band.band(rng.noise()) * a * ramp(i, attack) * 2.0);
        if i >= attack {
            a *= fade;
        }
    }
    fade_end(&mut sound, rate);
    sound
}

/// A vinyl crackle: a tiny dull pop.
fn crackle(rate: f64, rng: &mut Rng) -> Vec<f64> {
    let n = frame_count(0.004, rate).max(1);
    let attack = frame_count(0.0001, rate).max(1);
    let fade = decay_coef(0.0006, rate);
    let mut low = OnePole::new(5000.0, rate);
    let mut a = 1.0;
    let mut sound = Vec::with_capacity(n);
    for i in 0..n {
        sound.push(low.low(rng.noise()) * a * ramp(i, attack));
        a *= fade;
    }
    fade_end(&mut sound, rate);
    sound
}

/// `count` renderings of a sound with fresh noise each, so repeated hits do not sound
/// machine-identical.
fn variants(
    count: usize,
    rng: &mut Rng,
    mut make: impl FnMut(&mut Rng) -> Vec<f64>,
) -> Vec<Vec<f64>> {
    (0..count).map(|_| make(rng)).collect()
}

// Music
//
// A song places notes and drum hits on a beat grid into four stereo buses: the main mix,
// one ducked on every beat (the pumping of side-chain compression), and sends to an echo and
// a reverb. `finish` runs the sends, ducks, folds everything that rings past the loop's end
// back onto its start, and masters the result to the loudness target.
//
// The folding makes loops seamless. Every bus is linear and time-invariant (the ducking is a
// gain that repeats every beat, so it commutes with folding), so the folded loop equals the
// steady state of the pattern repeating forever: its last sample and its first are
// neighbours in one continuous performance. Parts that never stop (drones, LFOs) complete a
// whole number of cycles per loop instead.

/// Where and how loudly a sound plays in a song.
#[derive(Clone, Copy)]
struct To {
    gain: f64,
    pan: f64,
    /// Through the ducked bus.
    pump: bool,
    /// Send to the reverb.
    space: f64,
    /// Send to the echo.
    echo: f64,
}

fn to(gain: f64) -> To {
    To {
        gain,
        pan: 0.0,
        pump: false,
        space: 0.0,
        echo: 0.0,
    }
}

impl To {
    fn pan(self, pan: f64) -> To {
        To { pan, ..self }
    }

    fn space(self, space: f64) -> To {
        To { space, ..self }
    }

    fn echo(self, echo: f64) -> To {
        To { echo, ..self }
    }

    fn pumped(self) -> To {
        To { pump: true, ..self }
    }
}

/// How the notes of a chord are doubled: `voices` copies spread over `cents` of detune and
/// `width` of pan (1 for full left to full right), each with its LFO `phase` turns later.
#[derive(Clone, Copy)]
struct Spread {
    voices: usize,
    cents: f64,
    width: f64,
    phase: f64,
}

/// The voices of a chord, each with its pan.
type Voices = Vec<(Vec<f64>, f64)>;

/// A loop being written.
struct Song {
    rate: f64,
    seconds_per_beat: f64,
    frames_per_beat: f64,
    /// Length of the loop.
    frames: usize,
    rng: Rng,
    main: Stereo,
    pump: Stereo,
    space: Stereo,
    echo: Stereo,
    /// How deep the ducked bus dips on each beat, 0 to 1.
    pump_depth: f64,
    /// Reverb time (RT60) in seconds.
    reverb_seconds: f64,
    /// Echo delay in beats and the level of each repeat.
    echo_beats: f64,
    echo_feedback: f64,
    /// How many times in a row the loop's sounds are placed: 1 when rendering; tests place
    /// them twice to compare the loop with a performance that keeps going.
    repeats: usize,
}

impl Song {
    fn new(rate: f64, bpm: f64, frames: usize, rng: Rng) -> Song {
        let seconds_per_beat = 60.0 / bpm;
        Song {
            rate,
            seconds_per_beat,
            frames_per_beat: seconds_per_beat * rate,
            frames,
            rng,
            main: Stereo::new(rate, frames),
            pump: Stereo::new(rate, 0),
            space: Stereo::new(rate, 0),
            echo: Stereo::new(rate, 0),
            pump_depth: 0.0,
            reverb_seconds: 1.5,
            echo_beats: 0.75,
            echo_feedback: 0.35,
            repeats: 1,
        }
    }

    /// The frame at `beat`.
    fn at(&self, beat: f64) -> usize {
        frame_count(beat * self.seconds_per_beat, self.rate)
    }

    /// An LFO frequency that completes `cycles` cycles per loop.
    fn cycles(&self, cycles: f64) -> f64 {
        cycles * self.rate / self.frames as f64
    }

    /// One note starting at `beat`, held `beats` beats; its LFO keeps the song's phase.
    fn note(&self, beat: f64, midi: f64, beats: f64, patch: &Patch) -> Vec<f64> {
        let lfo = self.at(beat) as f64 * patch.lfo_hz / self.rate;
        note(
            self.rate,
            hz(midi),
            beats * self.seconds_per_beat,
            patch,
            lfo,
        )
    }

    /// The voices of a chord starting at `beat`, each with its pan.
    fn voices(
        &self,
        beat: f64,
        notes: &[f64],
        beats: f64,
        patch: &Patch,
        spread: Spread,
    ) -> Voices {
        let mut voices = Vec::with_capacity(notes.len() * spread.voices);
        for (k, &midi) in notes.iter().enumerate() {
            for v in 0..spread.voices {
                let offset = if spread.voices > 1 {
                    2.0 * v as f64 / (spread.voices - 1) as f64 - 1.0
                } else {
                    0.0
                };
                let shift = spread.phase * (k * spread.voices + v) as f64;
                let lfo = self.at(beat) as f64 * patch.lfo_hz / self.rate + shift;
                let pitch = hz(midi + offset * spread.cents / 100.0);
                let sound = note(self.rate, pitch, beats * self.seconds_per_beat, patch, lfo);
                voices.push((sound, offset * spread.width));
            }
        }
        voices
    }

    fn play(&mut self, beat: f64, sound: &[f64], to: To) {
        for repeat in 0..self.repeats {
            let at = self.at(beat) + repeat * self.frames;
            let bus = if to.pump {
                &mut self.pump
            } else {
                &mut self.main
            };
            bus.add(at, sound, to.pan, to.gain);
            if to.space > 0.0 {
                self.space.add(at, sound, to.pan, to.gain * to.space);
            }
            if to.echo > 0.0 {
                self.echo.add(at, sound, to.pan, to.gain * to.echo);
            }
        }
    }

    fn play_voices(&mut self, beat: f64, voices: &[(Vec<f64>, f64)], to: To) {
        for (sound, pan) in voices {
            self.play(beat, sound, to.pan((to.pan + pan).clamp(-1.0, 1.0)));
        }
    }

    /// Renders and plays a chord.
    fn chord(
        &mut self,
        beat: f64,
        notes: &[f64],
        beats: f64,
        patch: &Patch,
        spread: Spread,
        to: To,
    ) {
        let voices = self.voices(beat, notes, beats, patch, spread);
        self.play_voices(beat, &voices, to);
    }

    /// A sine that completes a whole number of cycles over the loop, so it runs on across
    /// the seam, swelling `swells` whole times per loop.
    fn drone(&mut self, midi: f64, swells: f64, to: To) {
        let frames = self.frames as f64;
        let cycles = (hz(midi) * frames / self.rate).round().max(1.0);
        let sound: Vec<f64> = (0..self.frames)
            .map(|n| {
                let u = n as f64 / frames;
                sin_turns(cycles * u) * (0.7 + 0.3 * sin_turns(swells * u))
            })
            .collect();
        self.play(0.0, &sound, to);
    }

    /// The finished loop: mixed down, folded and mastered.
    fn finish(self) -> (Vec<f64>, Vec<f64>) {
        let (rate, frames) = (self.rate, self.frames);
        let mut mix = self.mix_down();
        mix.fold(frames);
        master(mix.left, mix.right, rate)
    }

    /// Every bus with the sends and the ducking applied, mixed into one, still running past
    /// the loop's end.
    fn mix_down(self) -> Stereo {
        let Song {
            rate,
            seconds_per_beat,
            frames_per_beat,
            frames,
            mut main,
            mut pump,
            space,
            echo,
            pump_depth,
            reverb_seconds,
            echo_beats,
            echo_feedback,
            ..
        } = self;
        if !echo.is_empty() {
            let delay = frame_count(echo_beats * seconds_per_beat, rate).max(1);
            main.mix(&ping_pong(&echo, delay, echo_feedback));
        }
        if !space.is_empty() {
            main.mix(&reverb(&space, reverb_seconds));
        }
        if !pump.is_empty() {
            for (n, (l, r)) in pump.left.iter_mut().zip(pump.right.iter_mut()).enumerate() {
                let phase = ((n % frames) as f64 / frames_per_beat).fract();
                let gain = pump_gain(phase, pump_depth);
                *l *= gain;
                *r *= gain;
            }
            main.mix(&pump);
        }
        main
    }
}

/// The gain of the ducked bus at `phase` (0 to 1) through a beat: a dip of `depth` within
/// the first 4% of the beat, recovering along a cubic so it is back to 1 as the next beat
/// starts. Continuous across beats.
fn pump_gain(phase: f64, depth: f64) -> f64 {
    const ATTACK: f64 = 0.04;
    let rest = 1.0 - phase.max(ATTACK);
    let dip = depth * rest * rest * rest;
    if phase < ATTACK {
        1.0 - dip * phase / ATTACK
    } else {
        1.0 - dip
    }
}

/// Bass note and chord tones of a pop progression in C: C, G, Am, F.
const POP_CHORDS: [(f64, [f64; 4]); 4] = [
    (36.0, [60.0, 64.0, 67.0, 72.0]),
    (43.0, [59.0, 62.0, 67.0, 71.0]),
    (45.0, [60.0, 64.0, 69.0, 72.0]),
    (41.0, [60.0, 65.0, 69.0, 72.0]),
];

/// Beat and length of the chord stabs in a bar.
const POP_STABS: [(f64, f64); 5] = [
    (0.0, 0.35),
    (0.75, 0.35),
    (1.5, 0.35),
    (2.5, 0.35),
    (3.0, 0.7),
];

/// The lead melody over the four chords: beat, length in beats, note.
const POP_HOOK: [(f64, f64, f64); 23] = [
    (0.0, 0.5, 76.0),
    (0.5, 0.5, 79.0),
    (1.0, 1.0, 84.0),
    (2.0, 0.5, 79.0),
    (2.5, 0.5, 76.0),
    (3.0, 1.0, 74.0),
    (4.0, 0.5, 74.0),
    (4.5, 0.5, 79.0),
    (5.0, 1.0, 83.0),
    (6.0, 0.5, 79.0),
    (6.5, 0.5, 74.0),
    (7.0, 1.0, 71.0),
    (8.0, 0.5, 72.0),
    (8.5, 0.5, 76.0),
    (9.0, 1.0, 81.0),
    (10.0, 0.5, 79.0),
    (10.5, 0.5, 76.0),
    (11.0, 1.0, 72.0),
    (12.0, 0.5, 69.0),
    (12.5, 0.5, 72.0),
    (13.0, 1.0, 77.0),
    (14.0, 1.0, 76.0),
    (15.0, 1.0, 72.0),
];

const STAB: Patch = Patch {
    wave: Wave::Saw,
    attack: 0.003,
    decay: 0.25,
    sustain: 0.35,
    release: 0.04,
    cutoff: 1100.0,
    resonance: 1.1,
    sweep: 1.8,
    sweep_decay: 0.12,
    ..PLAIN
};

const POP_BASS: Patch = Patch {
    wave: Wave::Saw,
    attack: 0.002,
    decay: 0.12,
    sustain: 0.55,
    release: 0.03,
    cutoff: 420.0,
    resonance: 1.4,
    sweep: 1.6,
    sweep_decay: 0.05,
    ..PLAIN
};

const SUB: Patch = Patch {
    attack: 0.003,
    release: 0.03,
    ..PLAIN
};

const LEAD: Patch = Patch {
    wave: Wave::Pulse(0.35),
    attack: 0.006,
    decay: 0.3,
    sustain: 0.65,
    release: 0.09,
    cutoff: 3200.0,
    sweep: 0.6,
    sweep_decay: 0.1,
    lfo_hz: 5.5,
    vibrato: 12.0,
    ..PLAIN
};

fn upbeat(song: &mut Song) {
    song.pump_depth = 0.5;
    song.reverb_seconds = 1.2;
    let rate = song.rate;
    let kick = kick(rate, &mut song.rng, 48.0, 160.0, 0.17, 0.22);
    let clap = clap(rate, &mut song.rng);
    let open = hat(rate, &mut song.rng, 0.12, 15_000.0);
    let hats = variants(4, &mut song.rng, |rng| hat(rate, rng, 0.03, 15_000.0));
    let pair = Spread {
        voices: 2,
        cents: 14.0,
        width: 0.45,
        phase: 0.0,
    };
    // The stabs have no LFO, so each chord's two lengths are rendered once and reused.
    let stabs: Vec<[Voices; 2]> = POP_CHORDS
        .iter()
        .map(|(_, chord)| [0.35, 0.7].map(|beats| song.voices(0.0, chord, beats, &STAB, pair)))
        .collect();
    for bar in 0..8 {
        let start = f64::from(bar) * BEATS_PER_BAR;
        let index = bar as usize % POP_CHORDS.len();
        let root = POP_CHORDS[index].0;
        let bass = song.note(start, root + 12.0, 0.4, &POP_BASS);
        let sub = song.note(start, root, 0.4, &SUB);
        for beat in 0..4 {
            let at = start + f64::from(beat);
            let k = (bar * 4 + beat) as usize;
            song.play(at, &kick, to(1.0));
            song.play(at + 0.25, &hats[k % 4], to(0.07).pan(-0.2));
            song.play(at + 0.5, &open, to(0.16).pan(0.25));
            song.play(at + 0.75, &hats[(k + 2) % 4], to(0.09).pan(-0.2));
            song.play(at + 0.5, &bass, to(0.3).pumped());
            song.play(at + 0.5, &sub, to(0.42).pumped());
        }
        for beat in [1.0, 3.0] {
            song.play(start + beat, &clap, to(0.5).space(0.2));
        }
        for &(beat, length) in &POP_STABS {
            let voices = &stabs[index][usize::from(length > 0.5)];
            song.play_voices(start + beat, voices, to(0.07).pumped().space(0.2));
        }
    }
    // The hook comes in for the second half.
    for &(beat, length, midi) in &POP_HOOK {
        let at = 16.0 + beat;
        let lead = song.note(at, midi, length, &LEAD);
        song.play(at, &lead, to(0.17).pan(0.1).echo(0.3).space(0.15));
    }
}

/// Bass note and voicing of a lo-fi ii-V-I-vi in C: Dm9, G13, Cmaj9, Am9.
const CHILL_CHORDS: [(f64, [f64; 4]); 4] = [
    (38.0, [53.0, 57.0, 60.0, 64.0]),
    (43.0, [53.0, 57.0, 59.0, 64.0]),
    (36.0, [52.0, 55.0, 59.0, 62.0]),
    (33.0, [55.0, 59.0, 60.0, 64.0]),
];

/// A warm electric piano: FM with the modulator at the pitch and a falling index.
const EPIANO: Patch = Patch {
    wave: Wave::Fm {
        ratio: 1.0,
        index: 1.6,
        floor: 0.3,
        decay: 0.4,
    },
    attack: 0.004,
    decay: 1.4,
    sustain: 0.2,
    release: 0.15,
    cutoff: 3000.0,
    resonance: 0.6,
    vibrato: 8.0,
    ..PLAIN
};

const CHILL_BASS: Patch = Patch {
    attack: 0.006,
    decay: 0.6,
    sustain: 0.5,
    release: 0.08,
    drive: 0.6,
    ..PLAIN
};

fn chill(song: &mut Song) {
    song.reverb_seconds = 1.6;
    let rate = song.rate;
    // The tape wobble ("wow") completes 16 cycles per loop.
    let keys = Patch {
        lfo_hz: song.cycles(16.0),
        ..EPIANO
    };
    let kick = kick(rate, &mut song.rng, 46.0, 120.0, 0.2, 0.06);
    let snare = snare(rate, &mut song.rng, 185.0, 0.1, 3800.0);
    let hats = variants(4, &mut song.rng, |rng| hat(rate, rng, 0.035, 6500.0));
    let crackles = variants(6, &mut song.rng, |rng| crackle(rate, rng));
    for bar in 0..8 {
        let start = f64::from(bar) * BEATS_PER_BAR;
        let (root, chord) = CHILL_CHORDS[bar as usize % CHILL_CHORDS.len()];
        for beat in [0.0, 1.75, 2.5] {
            song.play(start + beat, &kick, to(0.9));
        }
        for beat in [1.03, 3.03] {
            song.play(start + beat, &snare, to(0.45).space(0.25));
        }
        // Swung eighths: the off-beats come late.
        for k in 0..8 {
            let offbeat = k % 2 == 1;
            let beat = 0.5 * f64::from(k) + if offbeat { 0.09 } else { 0.0 };
            let gain = if offbeat { 0.08 } else { 0.11 } * song.rng.range(0.8, 1.0);
            song.play(start + beat, &hats[k as usize % 4], to(gain).pan(0.3));
        }
        // A lazy strum on the one, and again on the and of three.
        for (beat, length, gain) in [(0.0, 1.8, 0.16), (2.5, 1.4, 0.12)] {
            for (k, &midi) in chord.iter().enumerate() {
                let at = start + beat + 0.03 * k as f64;
                let velocity = gain * song.rng.range(0.85, 1.0);
                let pan = 0.2 * k as f64 - 0.3;
                let piano = song.note(at, midi, length, &keys);
                song.play(at, &piano, to(velocity).pan(pan).space(0.3));
            }
        }
        for (beat, length) in [(0.0, 1.5), (2.5, 1.2)] {
            let bass = song.note(start + beat, root, length, &CHILL_BASS);
            song.play(start + beat, &bass, to(0.55));
        }
    }
    let beats = 8.0 * BEATS_PER_BAR;
    let count = (song.frames as f64 / rate * 6.0).round() as usize;
    for k in 0..count {
        let beat = song.rng.unit() * beats;
        let gain = song.rng.range(0.01, 0.04);
        let pan = song.rng.range(-0.7, 0.7);
        song.play(beat, &crackles[k % crackles.len()], to(gain).pan(pan));
    }
}

/// Bass note and voicing of a minor progression in D, two bars each: Dm, Bb, F, C.
const CINEMATIC_CHORDS: [(f64, [f64; 4]); 4] = [
    (38.0, [50.0, 57.0, 62.0, 65.0]),
    (34.0, [50.0, 58.0, 62.0, 65.0]),
    (41.0, [48.0, 57.0, 60.0, 65.0]),
    (36.0, [48.0, 55.0, 60.0, 64.0]),
];

/// A string section: detuned saws, slow attack, gentle vibrato.
const STRINGS: Patch = Patch {
    wave: Wave::Saw,
    attack: 0.9,
    decay: 2.0,
    sustain: 0.85,
    release: 0.35,
    cutoff: 1500.0,
    resonance: 0.6,
    lfo_hz: 4.8,
    vibrato: 6.0,
    ..PLAIN
};

/// Short low saw notes for the driving eighth-note pulse.
const OSTINATO: Patch = Patch {
    wave: Wave::Saw,
    attack: 0.004,
    decay: 0.12,
    sustain: 0.0,
    release: 0.04,
    cutoff: 650.0,
    resonance: 1.3,
    sweep: 1.4,
    sweep_decay: 0.05,
    ..PLAIN
};

fn cinematic(song: &mut Song) {
    song.reverb_seconds = 2.6;
    let rate = song.rate;
    let boom = kick(rate, &mut song.rng, 40.0, 110.0, 0.35, 0.3);
    let section = Spread {
        voices: 2,
        cents: 12.0,
        width: 0.5,
        phase: 0.23,
    };
    for (k, &(root, chord)) in CINEMATIC_CHORDS.iter().enumerate() {
        let start = 8.0 * k as f64;
        song.chord(start, &chord, 8.0, &STRINGS, section, to(0.06).space(0.35));
        // A high line doubles the top of the chords in the second half.
        if k >= 2 {
            let top = [chord[3] + 12.0];
            song.chord(start, &top, 8.0, &STRINGS, section, to(0.05).space(0.45));
        }
        for step in 0..16 {
            let at = start + 0.5 * f64::from(step);
            let gain = if step % 2 == 0 { 0.32 } else { 0.22 };
            let pulse = song.note(at, root + 12.0, 0.4, &OSTINATO);
            song.play(at, &pulse, to(gain).space(0.1));
        }
    }
    for bar in 0..8 {
        let start = f64::from(bar) * BEATS_PER_BAR;
        song.play(start, &boom, to(0.9).space(0.3));
        if bar % 2 == 1 {
            song.play(start + 2.5, &boom, to(0.55).space(0.3));
            song.play(start + 3.0, &boom, to(0.75).space(0.3));
        }
    }
}

/// Bass note and voicing of a I-IV-vi-V in G: G, C, Em, D.
const CORPORATE_CHORDS: [(f64, [f64; 4]); 4] = [
    (43.0, [67.0, 71.0, 74.0, 79.0]),
    (36.0, [67.0, 72.0, 76.0, 79.0]),
    (40.0, [67.0, 71.0, 76.0, 79.0]),
    (38.0, [66.0, 69.0, 74.0, 78.0]),
];

/// The order the chord tones are plucked in, one per eighth note.
const ARPEGGIO: [usize; 8] = [0, 1, 2, 3, 1, 2, 3, 2];

const PLUCK: Patch = Patch {
    wave: Wave::Saw,
    attack: 0.002,
    decay: 0.2,
    sustain: 0.0,
    release: 0.05,
    cutoff: 1000.0,
    resonance: 1.2,
    sweep: 2.4,
    sweep_decay: 0.08,
    ..PLAIN
};

/// Soft keys under the plucks: FM with a light index.
const KEYS: Patch = Patch {
    wave: Wave::Fm {
        ratio: 1.0,
        index: 0.8,
        floor: 0.15,
        decay: 0.5,
    },
    attack: 0.01,
    decay: 2.5,
    sustain: 0.4,
    release: 0.25,
    cutoff: 2500.0,
    resonance: 0.6,
    ..PLAIN
};

const CORPORATE_BASS: Patch = Patch {
    wave: Wave::Saw,
    attack: 0.003,
    decay: 0.15,
    sustain: 0.6,
    release: 0.03,
    cutoff: 380.0,
    resonance: 1.0,
    sweep: 1.0,
    sweep_decay: 0.06,
    ..PLAIN
};

fn corporate(song: &mut Song) {
    song.pump_depth = 0.25;
    song.reverb_seconds = 1.4;
    song.echo_beats = 0.75;
    song.echo_feedback = 0.35;
    let rate = song.rate;
    let kick = kick(rate, &mut song.rng, 50.0, 140.0, 0.13, 0.2);
    let clap = clap(rate, &mut song.rng);
    let hats = variants(4, &mut song.rng, |rng| hat(rate, rng, 0.025, 12_000.0));
    let shakers = variants(4, &mut song.rng, |rng| shaker(rate, rng, 0.03));
    let soft = Spread {
        voices: 2,
        cents: 8.0,
        width: 0.4,
        phase: 0.0,
    };
    // Nothing here has an LFO, so each chord's parts are rendered once and reused.
    let parts: Vec<_> = CORPORATE_CHORDS
        .iter()
        .map(|&(root, chord)| {
            let low: Vec<f64> = chord.iter().map(|midi| midi - 12.0).collect();
            let plucks: Vec<Vec<f64>> = ARPEGGIO
                .iter()
                .map(|&tone| song.note(0.0, chord[tone], 0.25, &PLUCK))
                .collect();
            let keys = song.voices(0.0, &low, 4.0, &KEYS, soft);
            (plucks, keys, song.note(0.0, root, 0.4, &CORPORATE_BASS))
        })
        .collect();
    for bar in 0..8 {
        let start = f64::from(bar) * BEATS_PER_BAR;
        let (plucks, keys, bass) = &parts[bar as usize % parts.len()];
        for beat in [0.0, 2.0, 2.5] {
            song.play(start + beat, &kick, to(0.85));
        }
        for beat in [1.0, 3.0] {
            song.play(start + beat, &clap, to(0.4).space(0.2));
        }
        for k in 0..16usize {
            let beat = start + 0.25 * k as f64;
            if k % 4 == 2 {
                song.play(beat, &hats[k / 4], to(0.12).pan(0.3));
            }
            let gain = if k % 2 == 1 { 0.05 } else { 0.035 };
            song.play(beat, &shakers[k % 4], to(gain).pan(-0.35));
        }
        for (step, pluck) in plucks.iter().enumerate() {
            let pan = if step % 2 == 0 { -0.3 } else { 0.3 };
            let at = start + 0.5 * step as f64;
            song.play(at, pluck, to(0.16).pan(pan).echo(0.35).space(0.1));
        }
        song.play_voices(start, keys, to(0.05).pumped().space(0.2));
        for step in 0..8 {
            song.play(start + 0.5 * f64::from(step), bass, to(0.32).pumped());
        }
    }
}

/// Bass note and chord of a minor progression in C, one bar each: Cm, Cm, Ab, G.
const TRAP_CHORDS: [(f64, [f64; 3]); 4] = [
    (36.0, [60.0, 63.0, 67.0]),
    (36.0, [60.0, 63.0, 67.0]),
    (32.0, [60.0, 63.0, 68.0]),
    (31.0, [59.0, 62.0, 67.0]),
];

/// The bell melody over the four bars: beat, note.
const TRAP_MELODY: [(f64, f64); 20] = [
    (0.0, 79.0),
    (0.75, 75.0),
    (1.5, 72.0),
    (2.5, 74.0),
    (3.0, 75.0),
    (4.0, 79.0),
    (4.75, 75.0),
    (5.5, 72.0),
    (6.5, 70.0),
    (7.0, 67.0),
    (8.0, 80.0),
    (8.75, 75.0),
    (9.5, 72.0),
    (10.5, 75.0),
    (11.0, 77.0),
    (12.0, 79.0),
    (12.75, 74.0),
    (13.5, 71.0),
    (14.5, 74.0),
    (15.0, 67.0),
];

/// An 808 bass: a long saturated sine that starts sharp and drops onto its pitch.
const BASS_808: Patch = Patch {
    attack: 0.002,
    decay: 1.1,
    sustain: 0.0,
    release: 0.04,
    drive: 1.6,
    bend: 4.0,
    ..PLAIN
};

/// A dark bell: FM with an inharmonic modulator ratio.
const TRAP_BELL: Patch = Patch {
    wave: Wave::Fm {
        ratio: 3.5,
        index: 2.2,
        floor: 0.4,
        decay: 0.25,
    },
    attack: 0.002,
    decay: 0.6,
    sustain: 0.0,
    release: 0.2,
    cutoff: 6000.0,
    ..PLAIN
};

const DARK_PAD: Patch = Patch {
    wave: Wave::Saw,
    attack: 0.3,
    decay: 2.0,
    sustain: 0.8,
    release: 0.4,
    cutoff: 800.0,
    ..PLAIN
};

fn trap(song: &mut Song) {
    song.reverb_seconds = 1.8;
    let rate = song.rate;
    let kick = kick(rate, &mut song.rng, 45.0, 160.0, 0.12, 0.3);
    let snare = snare(rate, &mut song.rng, 220.0, 0.12, 9000.0);
    let clap = clap(rate, &mut song.rng);
    let open = hat(rate, &mut song.rng, 0.15, 14_000.0);
    let hats = variants(4, &mut song.rng, |rng| hat(rate, rng, 0.022, 14_000.0));
    let wide = Spread {
        voices: 2,
        cents: 10.0,
        width: 0.6,
        phase: 0.0,
    };
    // The pad and the bells have no LFO, so each is rendered once and reused.
    let pads: Vec<_> = TRAP_CHORDS
        .iter()
        .map(|(_, chord)| song.voices(0.0, chord, 4.0, &DARK_PAD, wide))
        .collect();
    let bells: Vec<Vec<f64>> = TRAP_MELODY
        .iter()
        .map(|&(_, midi)| song.note(0.0, midi, 0.75, &TRAP_BELL))
        .collect();
    for bar in 0..8 {
        let start = f64::from(bar) * BEATS_PER_BAR;
        let index = bar as usize % TRAP_CHORDS.len();
        let root = TRAP_CHORDS[index].0;
        for (beat, length, octave) in [(0.0, 1.5, 0.0), (1.75, 0.75, 12.0), (2.75, 1.25, 0.0)] {
            let octave = if bar % 2 == 1 { octave } else { 0.0 };
            let bass = song.note(start + beat, root + octave, length, &BASS_808);
            song.play(start + beat, &bass, to(0.6));
        }
        for beat in [0.0, 2.75] {
            song.play(start + beat, &kick, to(0.8));
        }
        // Half time: the snare and clap land on the third beat.
        song.play(start + 2.0, &snare, to(0.45).space(0.15));
        song.play(start + 2.0, &clap, to(0.4).space(0.2));
        // Eighth-note hats, with triplet rolls into every second bar and a 32nd fill.
        let mut hits: Vec<(f64, f64)> = Vec::new();
        for k in 0..8 {
            let beat = 0.5 * f64::from(k);
            if bar % 2 == 1 && beat >= 3.0 {
                continue;
            }
            hits.push((beat, if k % 2 == 0 { 0.13 } else { 0.08 }));
        }
        if bar % 2 == 1 {
            for j in 0..6 {
                hits.push((3.0 + f64::from(j) / 6.0, 0.06 + 0.012 * f64::from(j)));
            }
        }
        if bar % 4 == 3 {
            hits.retain(|&(beat, _)| !(1.5..2.0).contains(&beat));
            for j in 0..4 {
                hits.push((1.5 + 0.125 * f64::from(j), 0.07));
            }
        }
        for (k, &(beat, gain)) in hits.iter().enumerate() {
            song.play(start + beat, &hats[k % 4], to(gain).pan(0.25));
        }
        if bar % 4 == 1 {
            song.play(start + 1.5, &open, to(0.1).pan(-0.2));
        }
        song.play_voices(start, &pads[index], to(0.035).space(0.3));
    }
    for phrase in 0..2 {
        for (&(beat, _), bell) in TRAP_MELODY.iter().zip(&bells) {
            let at = 16.0 * f64::from(phrase) + beat;
            song.play(at, bell, to(0.16).pan(-0.15).space(0.35));
        }
    }
}

/// Bass note and voicing of two long chords in C: Fmaj7, then Cmaj9 without its root.
const AMBIENT_CHORDS: [(f64, [f64; 4]); 2] = [
    (41.0, [53.0, 60.0, 64.0, 69.0]),
    (36.0, [52.0, 59.0, 62.0, 67.0]),
];

/// C major pentatonic from C5 to E6, for the distant bells.
const AMBIENT_NOTES: [f64; 8] = [72.0, 74.0, 76.0, 79.0, 81.0, 84.0, 86.0, 88.0];

/// A slow, dark pad whose filter opens and closes with the LFO.
const AMBIENT_PAD: Patch = Patch {
    wave: Wave::Saw,
    attack: 2.5,
    decay: 4.0,
    sustain: 0.9,
    release: 0.8,
    cutoff: 800.0,
    resonance: 0.9,
    wah: 1.2,
    ..PLAIN
};

const AMBIENT_BELL: Patch = Patch {
    wave: Wave::Fm {
        ratio: 3.5,
        index: 1.2,
        floor: 0.2,
        decay: 0.6,
    },
    attack: 0.003,
    decay: 1.5,
    sustain: 0.0,
    release: 0.5,
    ..PLAIN
};

fn ambient(song: &mut Song) {
    song.reverb_seconds = 4.0;
    // The filter breathes once per loop, each voice a little out of step with the others.
    let pad = Patch {
        lfo_hz: song.cycles(1.0),
        ..AMBIENT_PAD
    };
    let drift = Spread {
        voices: 2,
        cents: 9.0,
        width: 0.6,
        phase: 0.11,
    };
    for (k, (_, chord)) in AMBIENT_CHORDS.iter().enumerate() {
        let start = 16.0 * k as f64;
        song.chord(start, chord, 16.0, &pad, drift, to(0.07).space(0.45));
    }
    song.drone(36.0, 2.0, to(0.22));
    song.drone(43.0, 3.0, to(0.06).space(0.3));
    let beats = 8.0 * BEATS_PER_BAR;
    for _ in 0..14 {
        let beat = song.rng.unit() * beats;
        let midi = AMBIENT_NOTES[song.rng.below(AMBIENT_NOTES.len())];
        let gain = song.rng.range(0.06, 0.12);
        let pan = song.rng.range(-0.7, 0.7);
        let bell = song.note(beat, midi, 1.0, &AMBIENT_BELL);
        song.play(beat, &bell, to(gain).pan(pan).space(0.6));
    }
}

// Mixing and mastering

/// Two channels of `f64` samples that grow when sounds are added past their end.
struct Stereo {
    rate: f64,
    left: Vec<f64>,
    right: Vec<f64>,
}

impl Stereo {
    fn new(rate: f64, frames: usize) -> Stereo {
        Stereo {
            rate,
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        }
    }

    fn len(&self) -> usize {
        self.left.len()
    }

    fn is_empty(&self) -> bool {
        self.left.is_empty()
    }

    fn frames(&self, seconds: f64) -> usize {
        frame_count(seconds, self.rate)
    }

    /// Adds a mono sound at frame `at`, panned (-1 left to 1 right) with constant power.
    fn add(&mut self, at: usize, sound: &[f64], pan: f64, gain: f64) {
        let (left, right) = pan_gains(pan);
        self.add_pair(at, sound, sound, left * gain, right * gain);
    }

    /// Adds two channels at frame `at`, each with its own gain.
    fn add_pair(
        &mut self,
        at: usize,
        left: &[f64],
        right: &[f64],
        left_gain: f64,
        right_gain: f64,
    ) {
        let end = at + left.len().max(right.len());
        if self.left.len() < end {
            self.left.resize(end, 0.0);
            self.right.resize(end, 0.0);
        }
        for (out, x) in self.left[at..].iter_mut().zip(left) {
            *out += x * left_gain;
        }
        for (out, x) in self.right[at..].iter_mut().zip(right) {
            *out += x * right_gain;
        }
    }

    fn mix(&mut self, other: &Stereo) {
        self.add_pair(0, &other.left, &other.right, 1.0, 1.0);
    }

    /// Adds everything from frame `frames` on back onto the start, wrapping as often as
    /// needed, and cuts the buffer to `frames`.
    fn fold(&mut self, frames: usize) {
        if self.left.len() < frames {
            self.left.resize(frames, 0.0);
            self.right.resize(frames, 0.0);
        }
        for n in frames..self.left.len() {
            let (l, r) = (self.left[n], self.right[n]);
            self.left[n % frames] += l;
            self.right[n % frames] += r;
        }
        self.left.truncate(frames);
        self.right.truncate(frames);
    }
}

/// Left and right gains for `pan` (-1 left to 1 right), constant power.
fn pan_gains(pan: f64) -> (f64, f64) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) / 8.0;
    (sin_turns(angle + 0.25), sin_turns(angle))
}

/// A ping-pong echo of the send: the mono sum, repeated every `delay` frames, alternating
/// between left and right, each repeat `feedback` times the last.
fn ping_pong(send: &Stereo, delay: usize, feedback: f64) -> Stereo {
    let feedback = feedback.clamp(0.0, 0.9);
    let mut repeats = 1;
    let mut level = 1.0;
    while level > TAIL_FLOOR && repeats < 64 {
        level *= feedback;
        repeats += 1;
    }
    let len = send.len() + delay * repeats;
    let mut out = Stereo::new(send.rate, len);
    for n in delay..len {
        let m = n - delay;
        let dry = if m < send.len() {
            0.5 * (send.left[m] + send.right[m])
        } else {
            0.0
        };
        let left = dry + feedback * out.right[m];
        let right = feedback * out.left[m];
        out.left[n] = left;
        out.right[n] = right;
    }
    out
}

/// Comb delays of the reverb, in milliseconds, per channel: spread so that their echoes do
/// not line up, and different in each channel for width.
const COMB_MS: [[f64; 6]; 2] = [
    [31.1, 35.3, 38.9, 42.7, 46.1, 50.3],
    [32.3, 36.1, 39.7, 43.9, 47.3, 51.7],
];
/// All-pass delays of the reverb, in milliseconds, per channel.
const ALLPASS_MS: [[f64; 2]; 2] = [[5.1, 1.7], [5.5, 1.9]];
/// How much the reverb's combs darken each echo, 0 to 1.
const REVERB_DAMP: f64 = 0.3;
/// Output level of the reverb, so its long-run power is about that of its input.
const REVERB_GAIN: f64 = 0.15;

/// A feedback comb filter with a low-pass in its loop.
struct Comb {
    line: Vec<f64>,
    at: usize,
    feedback: f64,
    low: f64,
}

impl Comb {
    /// A comb of `ms` milliseconds whose echoes fall 60 dB in `seconds`.
    fn new(ms: f64, seconds: f64, rate: f64) -> Comb {
        let len = frame_count(ms / 1000.0, rate).max(1);
        let feedback = exp(-3.0 * LN_10 * len as f64 / rate / seconds.max(0.01));
        Comb {
            line: vec![0.0; len],
            at: 0,
            feedback,
            low: 0.0,
        }
    }

    fn run(&mut self, x: f64) -> f64 {
        let y = self.line[self.at];
        self.low = y + REVERB_DAMP * (self.low - y);
        self.line[self.at] = x + self.feedback * self.low;
        self.at = if self.at + 1 == self.line.len() {
            0
        } else {
            self.at + 1
        };
        y
    }
}

/// A Schroeder all-pass diffuser.
struct Allpass {
    line: Vec<f64>,
    at: usize,
}

impl Allpass {
    fn new(ms: f64, rate: f64) -> Allpass {
        Allpass {
            line: vec![0.0; frame_count(ms / 1000.0, rate).max(1)],
            at: 0,
        }
    }

    fn run(&mut self, x: f64) -> f64 {
        let delayed = self.line[self.at];
        self.line[self.at] = x + 0.5 * delayed;
        self.at = if self.at + 1 == self.line.len() {
            0
        } else {
            self.at + 1
        };
        delayed - x
    }
}

/// The reverberation of the send (its mono sum), with reverb time `seconds`, long enough to
/// hold the whole tail: parallel damped combs into series all-passes, per channel.
fn reverb(send: &Stereo, seconds: f64) -> Stereo {
    let rate = send.rate;
    // The tail falls 60 dB per reverb time, to TAIL_FLOOR in 100 / 60 of them.
    let len = send.len() + frame_count(seconds * 100.0 / 60.0, rate);
    let mut out = Stereo::new(rate, len);
    for (c, channel) in [&mut out.left, &mut out.right].into_iter().enumerate() {
        let mut combs: Vec<Comb> = COMB_MS[c]
            .iter()
            .map(|&ms| Comb::new(ms, seconds, rate))
            .collect();
        let mut passes: Vec<Allpass> = ALLPASS_MS[c]
            .iter()
            .map(|&ms| Allpass::new(ms, rate))
            .collect();
        for (n, y) in channel.iter_mut().enumerate() {
            let x = if n < send.len() {
                0.5 * (send.left[n] + send.right[n])
            } else {
                0.0
            };
            let mut wet = 0.0;
            for comb in &mut combs {
                wet += comb.run(x);
            }
            for pass in &mut passes {
                wet = pass.run(wet);
            }
            *y = wet * REVERB_GAIN;
        }
    }
    out
}

/// Scales the loop to [`MUSIC_LUFS`] and soft-limits its peaks. Limiting lowers the loudness
/// a little, so the gain is measured again after a first pass and corrected once.
fn master(left: Vec<f64>, right: Vec<f64>, rate: f64) -> (Vec<f64>, Vec<f64>) {
    let target = power_from_db(MUSIC_LUFS + 0.691);
    let measured = loudness_power(&left, &right, rate);
    if !(measured > 0.0 && measured.is_finite()) {
        return (left, right);
    }
    let limited = |gain: f64| -> (Vec<f64>, Vec<f64>) {
        (
            left.iter().map(|&x| soft_limit(x * gain)).collect(),
            right.iter().map(|&x| soft_limit(x * gain)).collect(),
        )
    };
    let mut gain = (target / measured).sqrt();
    let (l, r) = limited(gain);
    let after = loudness_power(&l, &r, rate);
    if after > 0.0 {
        gain *= (target / after).sqrt();
    }
    limited(gain)
}

/// `x` unchanged up to [`LIMIT_KNEE`], then bent smoothly (with a continuous slope) towards
/// [`LIMIT_CEILING`], which it never reaches.
fn soft_limit(x: f64) -> f64 {
    let a = x.abs();
    if a <= LIMIT_KNEE {
        return x;
    }
    let range = LIMIT_CEILING - LIMIT_KNEE;
    let u = (a - LIMIT_KNEE) / range;
    (LIMIT_KNEE + range * u / (1.0 + u)).copysign(x)
}

/// A second-order IIR filter (direct form I).
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn new(b: [f64; 3], a: [f64; 2]) -> Biquad {
        Biquad {
            b,
            a,
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// The two stages of the ITU-R BS.1770 K-weighting filter at `rate`: a high shelf of about
/// +4 dB above 1.7 kHz (the head) and a high-pass at about 38 Hz, from their analogue
/// prototypes by the bilinear transform. At 48 kHz they equal the coefficients the standard
/// lists.
fn k_weighting(rate: f64) -> [Biquad; 2] {
    const SHELF_HZ: f64 = 1681.974450955533;
    const SHELF_DB: f64 = 3.999843853973347;
    const SHELF_Q: f64 = 0.7071752369554196;
    const HIGHPASS_HZ: f64 = 38.13547087602444;
    const HIGHPASS_Q: f64 = 0.5003270373238773;
    let k = tan_pi((SHELF_HZ / rate).min(0.49));
    let vh = exp(SHELF_DB / 20.0 * LN_10);
    let vb = exp(0.4996667741545416 * SHELF_DB / 20.0 * LN_10);
    let a0 = 1.0 + k / SHELF_Q + k * k;
    let shelf = Biquad::new(
        [
            (vh + vb * k / SHELF_Q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / SHELF_Q + k * k) / a0,
        ],
        [2.0 * (k * k - 1.0) / a0, (1.0 - k / SHELF_Q + k * k) / a0],
    );
    let k = tan_pi((HIGHPASS_HZ / rate).min(0.49));
    let a0 = 1.0 + k / HIGHPASS_Q + k * k;
    let highpass = Biquad::new(
        [1.0, -2.0, 1.0],
        [
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / HIGHPASS_Q + k * k) / a0,
        ],
    );
    [shelf, highpass]
}

/// The gated mean power of the K-weighted channels (summed), as in ITU-R BS.1770: 400 ms
/// blocks every 100 ms, those under -70 LUFS left out, then those more than 10 LU under the
/// mean of the rest. The integrated loudness is `-0.691 + 10 log10` of it. Shorter input
/// counts as one block.
fn loudness_power(left: &[f64], right: &[f64], rate: f64) -> f64 {
    let hop = frame_count(0.1, rate).max(1);
    let full = left.len() / hop;
    let mut segments = vec![0.0; full];
    let mut total = 0.0;
    for channel in [left, right] {
        let [mut shelf, mut highpass] = k_weighting(rate);
        for (n, &x) in channel.iter().enumerate() {
            let y = highpass.run(shelf.run(x));
            let e = y * y;
            total += e;
            if let Some(segment) = segments.get_mut(n / hop) {
                *segment += e;
            }
        }
    }
    if full < 4 {
        return if left.is_empty() {
            0.0
        } else {
            total / left.len() as f64
        };
    }
    let blocks: Vec<f64> = segments
        .windows(4)
        .map(|w| (w[0] + w[1] + w[2] + w[3]) / (4 * hop) as f64)
        .collect();
    let absolute = power_from_db(-70.0 + 0.691);
    let gated_mean = |threshold: f64| {
        let (sum, count) = blocks
            .iter()
            .filter(|&&z| z > threshold)
            .fold((0.0, 0usize), |(sum, count), &z| (sum + z, count + 1));
        if count == 0 { 0.0 } else { sum / count as f64 }
    };
    let relative = 0.1 * gated_mean(absolute);
    gated_mean(relative.max(absolute))
}

// Filters and oscillators

/// A one-pole low-pass; its complement is a gentle high-pass.
struct OnePole {
    a: f64,
    z: f64,
}

impl OnePole {
    fn new(cutoff: f64, rate: f64) -> OnePole {
        let cutoff = cutoff.min(NYQUIST_SHARE * rate);
        OnePole {
            a: 1.0 - exp(-TAU * cutoff / rate),
            z: 0.0,
        }
    }

    fn low(&mut self, x: f64) -> f64 {
        self.z += self.a * (x - self.z);
        self.z
    }

    fn high(&mut self, x: f64) -> f64 {
        x - self.low(x)
    }
}

/// A state-variable filter, trapezoidal-integrated (stable under fast modulation).
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f64,
    ic2: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    k: f64,
}

impl Svf {
    fn tune(&mut self, cutoff: f64, q: f64, rate: f64) {
        let cutoff = cutoff.max(5.0).min(NYQUIST_SHARE * rate);
        let g = tan_pi(cutoff / rate);
        self.k = 1.0 / q.max(0.1);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// Low-pass and band-pass outputs.
    fn run(&mut self, x: f64) -> (f64, f64) {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1)
    }

    fn low(&mut self, x: f64) -> f64 {
        self.run(x).0
    }

    /// Band-pass with unit gain at the centre.
    fn band(&mut self, x: f64) -> f64 {
        self.run(x).1 * self.k
    }
}

/// A sawtooth at phase `t` (turns) stepping `dt` per sample, from -1 to 1, with its jump
/// smoothed by a polynomial band-limited step (PolyBLEP).
fn saw(t: f64, dt: f64) -> f64 {
    let blep = if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    };
    2.0 * t - 1.0 - blep
}

// Deterministic maths
//
// Only additions, multiplications, divisions, square roots and rounding to integers, which
// IEEE 754 defines exactly, so the results do not depend on the platform's maths library.

/// `sin(2πx)` for `x` in turns: reduced to a quarter turn around zero and summed as a Taylor
/// series to the 15th power (error under 1e-11).
fn sin_turns(x: f64) -> f64 {
    let mut r = x - (x + 0.5).floor();
    if r > 0.25 {
        r = 0.5 - r;
    } else if r < -0.25 {
        r = -0.5 - r;
    }
    let t = TAU * r;
    let t2 = t * t;
    // Horner's scheme: t (1 - t²/(2·3) (1 - t²/(4·5) (1 - ... (1 - t²/(14·15))))). The
    // reciprocals are rounded once, at compile time.
    let sum = 1.0 - t2 * (1.0 / 210.0);
    let sum = 1.0 - t2 * (1.0 / 156.0) * sum;
    let sum = 1.0 - t2 * (1.0 / 110.0) * sum;
    let sum = 1.0 - t2 * (1.0 / 72.0) * sum;
    let sum = 1.0 - t2 * (1.0 / 42.0) * sum;
    let sum = 1.0 - t2 * (1.0 / 20.0) * sum;
    t * (1.0 - t2 * (1.0 / 6.0) * sum)
}

/// `e^x`: `x = k ln 2 + r` with `|r| <= ln 2 / 2`, a Taylor series of `e^r` to the 13th
/// power, scaled by `2^k` (relative error under 1e-13 for `|x|` up to 400, growing with
/// `|x|` through the rounding of `k ln 2`).
fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x < -708.0 {
        return 0.0;
    }
    if x > 709.0 {
        return f64::INFINITY;
    }
    let k = (x / LN_2).round();
    let r = x - k * LN_2;
    let mut sum = 1.0;
    for n in (1..=13).rev() {
        sum = 1.0 + sum * r / f64::from(n);
    }
    // k is within -1022 to 1023 here, so 2^k is a normal number built from its exponent.
    let bits = ((k as i64 + 1023) as u64) << 52;
    sum * f64::from_bits(bits)
}

fn exp2(x: f64) -> f64 {
    exp(x * LN_2)
}

/// `tan(πx)`, for `x` between 0 and 1/2.
fn tan_pi(x: f64) -> f64 {
    sin_turns(0.5 * x) / sin_turns(0.5 * x + 0.25)
}

/// The power ratio of `db` decibels.
fn power_from_db(db: f64) -> f64 {
    exp(db / 10.0 * LN_10)
}

/// The frequency of MIDI note `midi` (69 is A4, 440 Hz).
fn hz(midi: f64) -> f64 {
    440.0 * exp2((midi - 69.0) / 12.0)
}

/// The per-sample factor of an exponential decay with time constant `seconds`.
fn decay_coef(seconds: f64, rate: f64) -> f64 {
    exp(-1.0 / (seconds * rate))
}

/// A soft saturation curve: the (3, 2) Padé approximant of `tanh`, which reaches ±1 with a
/// flat slope at ±3 and stays there.
fn saturate(x: f64) -> f64 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// A phase in turns brought back into 0 to 1.
fn wrap(phase: f64) -> f64 {
    if phase >= 1.0 {
        phase - phase.floor()
    } else {
        phase
    }
}

/// Seconds over which a note or hit that has decayed to [`SILENT`] fades to nothing, so it
/// ends without a step.
const END_FADE: f64 = 0.002;

/// Fades the last [`END_FADE`] of `sound` (or its second half, if shorter) linearly to 0.
fn fade_end(sound: &mut [f64], rate: f64) {
    let fade = frame_count(END_FADE, rate).min(sound.len() / 2).max(1);
    for (k, x) in sound.iter_mut().rev().take(fade).enumerate() {
        *x *= k as f64 / fade as f64;
    }
}

/// A linear ramp from 0 at frame 0 to 1 at frame `frames`, then 1.
fn ramp(i: usize, frames: usize) -> f64 {
    (i as f64 / frames as f64).min(1.0)
}

/// Frames in `seconds` at `rate`, rounded.
fn frame_count(seconds: f64, rate: f64) -> usize {
    (seconds * rate).round() as usize
}

/// A seeded generator: xorshift64* (Vigna, 2016), seeded through SplitMix64.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        Rng(if z == 0 { 0x2545_f491_4f6c_dd1d } else { z })
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in 0 to 1 (exclusive), from 53 random bits.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in -1 to 1.
    fn noise(&mut self) -> f64 {
        2.0 * self.unit() - 1.0
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    /// Uniform in 0 to `n - 1`.
    fn below(&mut self, n: usize) -> usize {
        ((self.unit() * n as f64) as usize).min(n.saturating_sub(1))
    }

    /// An independent generator seeded from this one.
    fn fork(&mut self) -> Rng {
        Rng::new(self.next())
    }
}

/// The seed of a sound: the FNV-1a hash of its name.
fn seed(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustfft::FftPlanner;
    use rustfft::num_complex::Complex;
    use std::time::{Duration, Instant};

    const RATE: u32 = 48_000;

    fn rendered(name: &str, rate: u32) -> Vec<f32> {
        render(name, rate).unwrap_or_else(|| panic!("{name} is a built-in sound"))
    }

    fn channel(samples: &[f32], c: usize) -> Vec<f64> {
        samples
            .iter()
            .skip(c)
            .step_by(2)
            .map(|&s| f64::from(s))
            .collect()
    }

    fn mono(samples: &[f32]) -> Vec<f64> {
        samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[l, r]| 0.5 * (f64::from(l) + f64::from(r)))
            .collect()
    }

    fn db(amplitude: f64) -> f64 {
        20.0 * amplitude.log10()
    }

    fn peak(samples: &[f32]) -> f64 {
        samples
            .iter()
            .fold(0.0f64, |peak, &s| peak.max(f64::from(s).abs()))
    }

    fn rms(x: &[f64]) -> f64 {
        (x.iter().map(|s| s * s).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }

    /// The power spectrum of `x` (zero-padded to a power of two) as (frequency, power).
    fn spectrum(x: &[f64], rate: u32) -> Vec<(f64, f64)> {
        let n = x.len().next_power_of_two();
        let mut buffer: Vec<Complex<f64>> = x.iter().map(|&s| Complex::new(s, 0.0)).collect();
        buffer.resize(n, Complex::new(0.0, 0.0));
        FftPlanner::new().plan_fft_forward(n).process(&mut buffer);
        (0..=n / 2)
            .map(|k| (k as f64 * f64::from(rate) / n as f64, buffer[k].norm_sqr()))
            .collect()
    }

    /// The share of the energy of `x` between `low` and `high` Hz.
    fn band_share(x: &[f64], rate: u32, low: f64, high: f64) -> f64 {
        let bins = spectrum(x, rate);
        let total: f64 = bins.iter().map(|&(_, p)| p).sum();
        let band: f64 = bins
            .iter()
            .filter(|&&(f, _)| f >= low && f < high)
            .map(|&(_, p)| p)
            .sum();
        band / total
    }

    /// The spectral centroid of `x`, in Hz.
    fn centroid(x: &[f64], rate: u32) -> f64 {
        let bins = spectrum(x, rate);
        let total: f64 = bins.iter().map(|&(_, p)| p).sum();
        bins.iter().map(|&(f, p)| f * p).sum::<f64>() / total
    }

    /// The strongest frequency of `x`, Hann-windowed and zero-padded to 2^16 points.
    fn dominant(x: &[f64], rate: u32) -> f64 {
        let n = x.len() as f64;
        let windowed: Vec<f64> = x
            .iter()
            .enumerate()
            .map(|(i, &s)| s * (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n).cos()))
            .chain(std::iter::repeat(0.0))
            .take(1 << 16)
            .collect();
        spectrum(&windowed, rate)
            .into_iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap()
            .0
    }

    /// Onsets: where the 2 ms RMS envelope rises through 40% of its maximum (from silence
    /// before the start), at least 30 ms after the previous one; in seconds.
    fn onsets(x: &[f64], rate: u32) -> Vec<f64> {
        let window = (0.002 * f64::from(rate)) as usize;
        let envelope: Vec<f64> = std::iter::once(0.0)
            .chain(x.chunks(window).map(rms))
            .collect();
        let threshold = 0.4 * envelope.iter().fold(0.0f64, |m, &e| m.max(e));
        let mut found = Vec::new();
        let mut last: Option<usize> = None;
        for (k, pair) in envelope.windows(2).enumerate() {
            if pair[0] < threshold && pair[1] >= threshold && last.is_none_or(|l| k - l >= 15) {
                found.push(k as f64 * window as f64 / f64::from(rate));
                last = Some(k);
            }
        }
        found
    }

    /// Integrated loudness by ITU-R BS.1770-4 at 48 kHz, written independently of the
    /// mastering code: the standard's published K-weighting coefficients, 400 ms blocks
    /// every 100 ms, the absolute gate at -70 LUFS and the relative gate 10 LU down.
    fn integrated_loudness(samples: &[f32]) -> f64 {
        const SHELF_B: [f64; 3] = [1.53512485958697, -2.69169618940638, 1.19839281085285];
        const SHELF_A: [f64; 2] = [-1.69065929318241, 0.73248077421585];
        const HIGH_B: [f64; 3] = [1.0, -2.0, 1.0];
        const HIGH_A: [f64; 2] = [-1.99004745483398, 0.99007225036621];
        fn filter(x: &[f64], b: [f64; 3], a: [f64; 2]) -> Vec<f64> {
            let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
            x.iter()
                .map(|&x0| {
                    let y0 = b[0] * x0 + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
                    (x2, x1, y2, y1) = (x1, x0, y1, y0);
                    y0
                })
                .collect()
        }
        let weighted: Vec<Vec<f64>> = (0..2)
            .map(|c| {
                filter(
                    &filter(&channel(samples, c), SHELF_B, SHELF_A),
                    HIGH_B,
                    HIGH_A,
                )
            })
            .collect();
        let (block, step) = (19_200, 4_800);
        let frames = weighted[0].len();
        let blocks: Vec<f64> = (0..=(frames - block) / step)
            .map(|j| {
                weighted
                    .iter()
                    .map(|w| {
                        w[j * step..j * step + block]
                            .iter()
                            .map(|y| y * y)
                            .sum::<f64>()
                    })
                    .sum::<f64>()
                    / block as f64
            })
            .collect();
        let loudness = |z: f64| -0.691 + 10.0 * z.log10();
        let mean = |threshold: f64| {
            let kept: Vec<f64> = blocks
                .iter()
                .copied()
                .filter(|&z| loudness(z) > threshold)
                .collect();
            kept.iter().sum::<f64>() / kept.len() as f64
        };
        let relative = loudness(mean(-70.0)) - 10.0;
        loudness(mean(relative))
    }

    /// Renders `spec` at `rate` and checks the length, that every sample is finite, the
    /// levels, and that a second rendering gives the same samples; returns the samples.
    fn check_sound(spec: &SoundSpec, rate: u32) -> Vec<f32> {
        let samples = rendered(spec.name, rate);
        let frames = (spec.seconds * f64::from(rate)).round() as usize;
        assert_eq!(samples.len(), frames * 2, "{} length", spec.name);
        assert!(
            samples.iter().all(|s| s.is_finite()),
            "{} is finite",
            spec.name
        );
        let peak = peak(&samples);
        assert!(peak < 1.0, "{} peaks at {peak}", spec.name);
        match spec.kind {
            SoundKind::Effect => {
                let error = db(peak) + 3.0;
                assert!(
                    error.abs() < 0.01,
                    "{} peaks at {:.2} dBFS",
                    spec.name,
                    db(peak)
                );
            }
            SoundKind::Music => {
                assert!(
                    db(peak) < -0.9,
                    "{} peaks at {:.2} dBFS",
                    spec.name,
                    db(peak)
                );
            }
        }
        let all: Vec<f64> = samples.iter().map(|&s| f64::from(s)).collect();
        let level = db(rms(&all));
        assert!(
            level > -40.0,
            "{} is too quiet: {level:.1} dBFS RMS",
            spec.name
        );
        println!(
            "{rate} Hz {:<15} {:>6.2} s  peak {:>6.2} dBFS  RMS {:>6.1} dBFS",
            spec.name,
            spec.seconds,
            db(peak),
            level
        );
        assert!(
            rendered(spec.name, rate) == samples,
            "{} renders the same again",
            spec.name
        );
        samples
    }

    #[test]
    fn every_effect_renders_at_48_and_44_1_khz() {
        for spec in sounds().iter().filter(|s| s.kind == SoundKind::Effect) {
            for rate in [48_000, 44_100] {
                check_sound(spec, rate);
            }
        }
    }

    /// Checks the loop `name` at `rate` as [`check_sound`] does, and that it lasts whole
    /// bars, repeats without a click (the step from its last sample to its first is no
    /// larger than the 99.9th percentile of its sample-to-sample steps) and, at 48 kHz,
    /// measures -16 LUFS.
    fn check_loop(name: &str, rate: u32) {
        let spec = sounds().into_iter().find(|s| s.name == name).unwrap();
        assert_eq!(spec.kind, SoundKind::Music);
        let bars = spec.seconds * spec.bpm.unwrap() / 60.0 / BEATS_PER_BAR;
        assert!(
            (bars - bars.round()).abs() < 1e-9,
            "{name} lasts {bars} bars"
        );
        assert!(
            bars.round() == 8.0 || bars.round() == 16.0,
            "{name} lasts {bars} bars"
        );
        let samples = check_sound(&spec, rate);
        for c in 0..2 {
            let x = channel(&samples, c);
            let wrap = (x[0] - x[x.len() - 1]).abs();
            let mut steps: Vec<f64> = x.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
            steps.sort_by(f64::total_cmp);
            let p999 = steps[steps.len() * 999 / 1000];
            let median = steps[steps.len() / 2];
            println!(
                "{rate} Hz {name:<15} channel {c}: step across the seam {wrap:.5}, \
                 median step {median:.5}, 99.9th percentile {p999:.5}"
            );
            assert!(wrap <= p999, "{name} clicks at the seam: {wrap} > {p999}");
        }
        if rate == 48_000 {
            let loudness = integrated_loudness(&samples);
            println!("{name:<15} {loudness:.2} LUFS");
            assert!(
                (loudness - MUSIC_LUFS).abs() < 0.5,
                "{name} measures {loudness:.2} LUFS"
            );
        }
    }

    /// Checks that the loop `name` at `rate`, before mastering, equals the second of two
    /// performances of its pattern placed back to back: what plays across the seam is what
    /// a performance that kept going would play. Without the folding, the start would lack
    /// what rings on from the end; with any processing that is not time-invariant (or does
    /// not repeat with the loop), the two would differ throughout.
    fn check_repeats(name: &str, rate: u32) {
        let builtin = BUILTINS.iter().find(|b| b.name == name).unwrap();
        let Body::Music { bpm, make, .. } = builtin.body else {
            panic!("{name} is not music");
        };
        let rate = f64::from(rate);
        let frames = frame_count(builtin.seconds(), rate);
        let song = |repeats: usize| {
            let mut song = Song::new(rate, bpm, frames, Rng::new(seed(name)));
            song.repeats = repeats;
            make(&mut song);
            song.mix_down()
        };
        let mut looped = song(1);
        let unfolded = looped.left[..frames].to_vec();
        looped.fold(frames);
        let twice = song(2);
        let scale = looped.left.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        let largest = |a: &[f64], b: &[f64]| {
            a.iter()
                .zip(&b[frames..2 * frames])
                .fold(0.0f64, |m, (x, y)| m.max((x - y).abs()))
        };
        let worst = largest(&looped.left, &twice.left).max(largest(&looped.right, &twice.right));
        let cut = largest(&unfolded, &twice.left);
        println!(
            "{name:<15} loop against a continuing performance: largest difference {:.1e} \
             of the peak folded, {:.2} unfolded",
            worst / scale,
            cut / scale
        );
        assert!(worst <= TAIL_FLOOR * scale, "{name}: {worst}");
        assert!(cut > 1e-3 * scale, "{name}: the fold should matter");
    }

    /// Tests for each loop, at each rate, so that they render in parallel.
    macro_rules! loop_tests {
        ($($module:ident: $name:literal),* $(,)?) => {
            const TESTED_LOOPS: &[&str] = &[$($name),*];
            $(
                mod $module {
                    #[test]
                    fn renders_seamlessly_at_minus_16_lufs_at_48_khz() {
                        super::check_loop($name, 48_000);
                        super::check_repeats($name, 48_000);
                    }

                    #[test]
                    fn renders_seamlessly_at_44_1_khz() {
                        super::check_loop($name, 44_100);
                    }
                }
            )*
        };
    }

    loop_tests!(
        upbeat_loop: "upbeat-loop",
        chill_loop: "chill-loop",
        cinematic_loop: "cinematic-loop",
        corporate_loop: "corporate-loop",
        trap_loop: "trap-loop",
        ambient_loop: "ambient-loop",
    );

    #[test]
    fn every_loop_is_tested() {
        let music: Vec<&str> = sounds()
            .iter()
            .filter(|s| s.kind == SoundKind::Music)
            .map(|s| s.name)
            .collect();
        assert_eq!(music, TESTED_LOOPS);
    }

    #[test]
    fn the_impact_is_a_low_boom() {
        let share = band_share(&mono(&rendered("impact", RATE)), RATE, 0.0, 150.0);
        println!("impact: {:.1}% of the energy below 150 Hz", 100.0 * share);
        assert!(share > 0.8, "{share}");
    }

    #[test]
    fn the_ding_and_the_sparkle_are_bright() {
        for name in ["ding", "sparkle"] {
            let share = band_share(&mono(&rendered(name, RATE)), RATE, 2000.0, f64::INFINITY);
            println!("{name}: {:.1}% of the energy above 2 kHz", 100.0 * share);
            assert!(share > 0.9, "{name}: {share}");
        }
    }

    #[test]
    fn the_riser_grows_louder_and_brighter() {
        let x = mono(&rendered("riser", RATE));
        let body = &x[..x.len() - (0.05 * f64::from(RATE)) as usize];
        let quarters: Vec<&[f64]> = body.chunks(body.len().div_ceil(4)).collect();
        let levels: Vec<f64> = quarters.iter().map(|q| db(rms(q))).collect();
        let centroids: Vec<f64> = quarters.iter().map(|q| centroid(q, RATE)).collect();
        println!("riser quarters: RMS {levels:.1?} dBFS, centroid {centroids:.0?} Hz");
        for k in 1..4 {
            assert!(levels[k] > levels[k - 1] + 1.0, "{levels:?}");
            assert!(centroids[k] > centroids[k - 1] * 1.2, "{centroids:?}");
        }
    }

    #[test]
    fn the_swoosh_falls_and_the_whoosh_passes_left_to_right() {
        let x = mono(&rendered("swoosh-down", RATE));
        let third = x.len() / 3;
        let (early, late) = (centroid(&x[..third], RATE), centroid(&x[2 * third..], RATE));
        println!("swoosh-down centroid: {early:.0} Hz, then {late:.0} Hz");
        assert!(early > 2.0 * late, "{early} then {late}");

        let samples = rendered("whoosh", RATE);
        let (left, right) = (channel(&samples, 0), channel(&samples, 1));
        let third = left.len() / 3;
        let balance =
            |range: std::ops::Range<usize>| db(rms(&right[range.clone()]) / rms(&left[range]));
        let (first, last) = (balance(0..third), balance(2 * third..left.len()));
        println!(
            "whoosh right-to-left balance: {first:.1} dB in the first third, {last:.1} dB in the last"
        );
        assert!(first < -3.0 && last > 3.0, "{first} then {last}");
    }

    #[test]
    fn the_coin_steps_up_a_fifth() {
        let x = mono(&rendered("coin", RATE));
        let at = |seconds: f64| (seconds * f64::from(RATE)) as usize;
        let first = dominant(&x[at(0.005)..at(0.07)], RATE);
        let second = dominant(&x[at(0.15)..at(0.4)], RATE);
        println!("coin: {first:.1} Hz, then {second:.1} Hz");
        assert!((first - 880.0).abs() < 5.0, "{first}");
        assert!(
            (second / first - 1.4983).abs() < 0.01,
            "{first} then {second}"
        );
    }

    #[test]
    fn the_typewriter_and_the_shutter_click_the_right_number_of_times() {
        let keys = onsets(&mono(&rendered("typewriter", RATE)), RATE);
        let shutter = onsets(&mono(&rendered("camera-shutter", RATE)), RATE);
        println!("typewriter strikes at {keys:.3?} s, shutter clicks at {shutter:.3?} s");
        assert_eq!(keys.len(), 6, "{keys:?}");
        assert_eq!(shutter.len(), 2, "{shutter:?}");
    }

    #[test]
    fn unusual_sample_rates_give_finite_unclipped_audio() {
        for rate in [1, 300, 8000] {
            for spec in sounds() {
                let samples = rendered(spec.name, rate);
                let frames = (spec.seconds * f64::from(rate)).round() as usize;
                assert_eq!(samples.len(), frames * 2, "{} at {rate} Hz", spec.name);
                assert!(
                    samples.iter().all(|s| s.is_finite() && s.abs() < 1.0),
                    "{} at {rate} Hz",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn the_maths_matches_the_standard_library() {
        for k in -4000..4000 {
            let x = f64::from(k) * 0.000_731;
            let turns = sin_turns(x) - (TAU * x).sin();
            assert!(turns.abs() < 1e-11, "sin at {x}: {turns}");
            let e = exp(x * 100.0) / (x * 100.0).exp() - 1.0;
            assert!(e.abs() < 1e-13, "exp at {}: {e}", x * 100.0);
        }
        for k in 1..490 {
            let x = f64::from(k) / 1000.0;
            let t = tan_pi(x) / (std::f64::consts::PI * x).tan() - 1.0;
            assert!(t.abs() < 1e-10, "tan at {x}: {t}");
        }
        assert_eq!(exp(-1000.0), 0.0);
        assert!((hz(69.0) - 440.0).abs() < 1e-9 && (hz(81.0) - 880.0).abs() < 1e-9);
    }

    #[test]
    fn the_k_weighting_matches_the_standard_at_48_khz() {
        let [shelf, highpass] = k_weighting(48_000.0);
        let expected = [
            1.53512485958697,
            -2.69169618940638,
            1.19839281085285,
            -1.69065929318241,
            0.73248077421585,
        ];
        let got = [shelf.b[0], shelf.b[1], shelf.b[2], shelf.a[0], shelf.a[1]];
        for (g, e) in got.iter().zip(expected) {
            assert!((g - e).abs() < 1e-9, "{got:?}");
        }
        assert!((highpass.a[0] + 1.99004745483398).abs() < 1e-9);
        assert!((highpass.a[1] - 0.99007225036621).abs() < 1e-9);
    }

    #[test]
    fn the_limiter_never_reaches_its_ceiling() {
        assert_eq!(soft_limit(0.5), 0.5);
        assert_eq!(soft_limit(-0.3), -0.3);
        let mut last = 0.0;
        for k in 0..2000 {
            let x = f64::from(k) * 0.01;
            let y = soft_limit(x);
            assert!(y >= last && y < LIMIT_CEILING, "{x} -> {y}");
            assert_eq!(soft_limit(-x), -y);
            last = y;
        }
        // The slope is continuous at the knee.
        let slope = (soft_limit(LIMIT_KNEE + 1e-6) - LIMIT_KNEE) / 1e-6;
        assert!((slope - 1.0).abs() < 1e-4, "{slope}");
    }

    #[test]
    fn the_pump_is_continuous_across_beats() {
        assert_eq!(pump_gain(0.0, 0.5), 1.0);
        assert!((pump_gain(0.999_999, 0.5) - 1.0).abs() < 1e-9);
        assert!(pump_gain(0.04, 0.5) < 0.6);
        let below = pump_gain(0.04 - 1e-9, 0.5);
        assert!((below - pump_gain(0.04, 0.5)).abs() < 1e-6);
    }

    #[test]
    #[ignore = "timing means something only in an optimized build: run with --release -- --ignored --nocapture"]
    fn every_sound_renders_within_two_seconds() {
        let start = Instant::now();
        for spec in sounds() {
            let one = Instant::now();
            let samples = render(spec.name, RATE).unwrap();
            println!(
                "{:<15} {:>7.1} ms",
                spec.name,
                one.elapsed().as_secs_f64() * 1000.0
            );
            assert!(!samples.is_empty());
        }
        let total = start.elapsed();
        println!(
            "all sounds at 48 kHz: {:.0} ms",
            total.as_secs_f64() * 1000.0
        );
        if !cfg!(debug_assertions) {
            assert!(total < Duration::from_secs(2), "{total:?}");
        }
    }

    #[test]
    #[ignore = "writes every sound as a WAV file into FOSSCUT_SOUNDS_DIR, for listening"]
    fn write_every_sound_for_listening() {
        let dir = std::path::PathBuf::from(std::env::var_os("FOSSCUT_SOUNDS_DIR").unwrap());
        for spec in sounds() {
            let path = dir.join(format!("{}.wav", spec.name));
            crate::wav::write_wav(&path, &rendered(spec.name, RATE), 2, RATE).unwrap();
        }
    }
}
