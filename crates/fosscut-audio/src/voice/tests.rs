//! Tests of the voice filters: what each one does, measured, and the properties they share
//! (blocks, silence, levels, presets).

use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

use fosscut_model::voice_filters::{self as names, voice_presets};
use fosscut_model::{ParamValue, effect_type};

use super::*;
use crate::effects::{self, AudioEffect};

/// The rate most tests run at: low enough for debug builds, high enough for the bands.
const RATE: u32 = 16_000;

fn frames(seconds: f64, rate: u32) -> usize {
    (seconds * f64::from(rate)).round() as usize
}

fn run(effect: &VoiceEffect, input: &[f32], channels: u16, rate: u32) -> Vec<f32> {
    effects::process(&[AudioEffect::Voice(effect.clone())], input, channels, rate)
}

fn sine(freq: f64, amp: f64, count: usize, rate: u32) -> Vec<f32> {
    (0..count)
        .map(|i| (amp * (TAU * freq * i as f64 / f64::from(rate)).sin()) as f32)
        .collect()
}

fn impulse(at: usize, count: usize) -> Vec<f32> {
    let mut x = vec![0.0f32; count];
    x[at] = 1.0;
    x
}

fn interleave(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter().zip(right).flat_map(|(l, r)| [*l, *r]).collect()
}

fn channel(x: &[f32], channels: usize, c: usize) -> Vec<f32> {
    x.iter().skip(c).step_by(channels).copied().collect()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

fn power(x: &[f32]) -> f64 {
    x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64
}

fn db(ratio: f64) -> f64 {
    10.0 * ratio.max(1e-30).log10()
}

fn max_difference(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

/// Power of the `freq` Hz component of `x` (Goertzel, Hann-windowed), as the power of a
/// sine of that amplitude would be: `A^2 / 2`.
fn tone_power(x: &[f32], freq: f64, rate: u32) -> f64 {
    let n = x.len();
    let w = TAU * freq / f64::from(rate);
    let coefficient = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for (i, v) in x.iter().enumerate() {
        let hann = 0.5 - 0.5 * (TAU * i as f64 / n as f64).cos();
        let s = f64::from(*v) * hann + coefficient * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    // The Hann window halves the amplitude.
    let amplitude =
        2.0 * 2.0 * (s1 * s1 + s2 * s2 - coefficient * s1 * s2).max(0.0).sqrt() / n as f64;
    amplitude * amplitude / 2.0
}

/// The frequency between `lo` and `hi` where `x` has the most power, to 0.02%: a scan in
/// steps of 0.2%, refined around the best step.
fn peak_frequency(x: &[f32], rate: u32, lo: f64, hi: f64) -> f64 {
    let scan = |lo: f64, hi: f64, step: f64| {
        let mut best = (0.0, lo);
        let mut f = lo;
        while f <= hi {
            let p = tone_power(x, f, rate);
            if p > best.0 {
                best = (p, f);
            }
            f *= step;
        }
        best.1
    };
    let coarse = scan(lo, hi, 1.002);
    scan(coarse / 1.003, coarse * 1.003, 1.0002)
}

/// A speech-like stereo signal: a voice with gliding pitch and three formants in syllables
/// with pauses, consonant noise at syllable starts, and a quiet noise floor, peaking at 0.5.
fn speech(seconds: f64, rate: u32) -> Vec<f32> {
    let count = frames(seconds, rate);
    let r = f64::from(rate);
    let mut noise = 0x2545_f491_4f6c_dd1du64;
    let mut white = move || {
        noise ^= noise << 13;
        noise ^= noise >> 7;
        noise ^= noise << 17;
        (noise >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    };
    let formant = |f: f64| {
        1.0 / (1.0 + ((f - 600.0) / 250.0).powi(2))
            + 0.6 / (1.0 + ((f - 1700.0) / 350.0).powi(2))
            + 0.3 / (1.0 + ((f - 2600.0) / 450.0).powi(2))
    };
    let mut phase = 0.0f64;
    let mut out = Vec::with_capacity(2 * count);
    for i in 0..count {
        let t = i as f64 / r;
        let f0 = 140.0 + 25.0 * (TAU * 0.7 * t).sin() + 4.0 * (TAU * 5.5 * t).sin();
        phase += TAU * f0 / r;
        let syllable = (TAU * 3.3 * t).sin().max(0.0).powf(0.6);
        let envelope = if t % 1.7 > 1.35 { 0.0 } else { syllable };
        let mut voice = 0.0;
        for h in 1..=24 {
            let f = h as f64 * f0;
            if f > 0.45 * r {
                break;
            }
            voice += formant(f) * (h as f64 * phase + h as f64).sin();
        }
        let onset = (t * 3.3).fract();
        let consonant = if onset < 0.06 && t % 1.7 <= 1.35 {
            0.4 * (1.0 - onset / 0.06)
        } else {
            0.0
        };
        let left = 0.12 * voice * envelope + consonant * white() + 0.001 * white();
        let right = 0.9 * left + 0.05 * consonant * white();
        out.push(left as f32);
        out.push(right as f32);
    }
    let scale = 0.5 / peak(&out);
    out.iter().map(|v| v * scale).collect()
}

/// What the mixer computes for frames `from..to` of a clip: the chain fed its warm-up
/// before (never before the clip's start) and its look-ahead after (never past its end),
/// with positions counted from the clip's start.
fn window(
    chain: &[AudioEffect],
    input: &[f32],
    channels: usize,
    rate: u32,
    from: usize,
    to: usize,
) -> Vec<f32> {
    let total = input.len() / channels;
    let first = from.saturating_sub(effects::warm_up_frames(chain, rate));
    let until = (to + effects::lookahead_frames(chain, rate)).min(total);
    let out = effects::process_at(
        chain,
        &input[first * channels..until * channels],
        channels as u16,
        rate,
        first as i64,
    );
    out[(from - first) * channels..(to - first) * channels].to_vec()
}

/// The largest difference between the whole clip processed at once and the frame `ranges`
/// rendered in blocks of 512, 1000 and 4800 frames the way the mixer renders them.
fn block_difference(
    chain: &[AudioEffect],
    input: &[f32],
    channels: usize,
    rate: u32,
    ranges: &[(usize, usize)],
) -> f32 {
    let whole = effects::process(chain, input, channels as u16, rate);
    let mut worst = 0.0f32;
    for block in [512, 1000, 4800] {
        for &(from, to) in ranges {
            let mut at = from;
            while at < to {
                let end = (at + block).min(to);
                let got = window(chain, input, channels, rate, at, end);
                worst = worst.max(max_difference(&got, &whole[at * channels..end * channels]));
                at = end;
            }
        }
    }
    worst
}

/// One of each filter, mostly at its registry defaults, and the pitch both ways.
fn all_filters() -> Vec<VoiceEffect> {
    vec![
        VoiceEffect::Pitch {
            semitones: 7.0,
            keep_formants: true,
        },
        VoiceEffect::Pitch {
            semitones: -5.0,
            keep_formants: false,
        },
        VoiceEffect::Pitch {
            semitones: -12.0,
            keep_formants: true,
        },
        VoiceEffect::Robot {
            frequency: 60.0,
            depth: 1.0,
        },
        VoiceEffect::Telephone {
            narrow: 0.3,
            crunch: 0.3,
        },
        VoiceEffect::Megaphone { drive: 0.6 },
        VoiceEffect::Echo {
            delay_ms: 250.0,
            feedback: 0.4,
            mix: 0.3,
        },
        VoiceEffect::Reverb {
            room: 0.5,
            damping: 0.5,
            mix: 0.3,
        },
        VoiceEffect::Chorus {
            rate: 0.8,
            depth: 0.5,
            mix: 0.5,
        },
        VoiceEffect::Broken { amount: 0.6 },
        VoiceEffect::Whisper { amount: 1.0 },
    ]
}

/// Asserts that each of `filters` sounds the same in blocks of 512, 1000 and 4800 frames as
/// at once, within 1e-6, at the start of a clip and past its warm-up.
fn assert_blocks(filters: impl Iterator<Item = VoiceEffect>) {
    let input = speech(2.6, RATE);
    let ranges = [
        (0, frames(0.2, RATE)),
        (frames(1.9, RATE) + 7, frames(2.2, RATE) + 7),
    ];
    for effect in filters {
        let chain = [AudioEffect::Voice(effect.clone())];
        assert!(effects::warm_up_frames(&chain, RATE) <= frames(1.9, RATE));
        let worst = block_difference(&chain, &input, 2, RATE, &ranges);
        println!("{effect:?}: blocks differ by {worst:e}");
        assert!(worst <= 1e-6, "{effect:?}: blocks differ by {worst}");
    }
}

#[test]
fn the_pitch_shifter_sounds_the_same_in_blocks() {
    assert_blocks(
        all_filters()
            .into_iter()
            .filter(|e| matches!(e, VoiceEffect::Pitch { .. })),
    );
}

#[test]
fn the_space_filters_sound_the_same_in_blocks() {
    assert_blocks(all_filters().into_iter().filter(|e| {
        matches!(
            e,
            VoiceEffect::Echo { .. } | VoiceEffect::Reverb { .. } | VoiceEffect::Chorus { .. }
        )
    }));
}

#[test]
fn the_timbre_filters_sound_the_same_in_blocks() {
    assert_blocks(all_filters().into_iter().filter(|e| {
        !matches!(
            e,
            VoiceEffect::Pitch { .. }
                | VoiceEffect::Echo { .. }
                | VoiceEffect::Reverb { .. }
                | VoiceEffect::Chorus { .. }
        )
    }));
}

#[test]
fn the_pitch_shifter_sounds_the_same_in_blocks_at_48_khz() {
    // At 48 kHz the splice search runs on a coarse copy first, a path of its own.
    let rate = 48_000;
    let input = channel(&speech(1.5, rate), 2, 0);
    let ranges = [(frames(0.95, rate) + 3, frames(1.07, rate) + 3)];
    for keep_formants in [false, true] {
        let chain = [AudioEffect::Voice(VoiceEffect::Pitch {
            semitones: 4.0,
            keep_formants,
        })];
        let worst = block_difference(&chain, &input, 1, rate, &ranges);
        println!("formants {keep_formants}: blocks differ by {worst:e}");
        assert!(worst <= 1e-6, "blocks differ by {worst}");
    }
}

#[test]
fn silence_stays_silent_and_tails_end_in_zeros() {
    let count = frames(3.0, RATE);
    let silence = vec![0.0f32; 2 * count];
    // A short tone, then silence long enough for every tail to end.
    let tone = sine(440.0, 0.5, frames(0.3, RATE), RATE);
    let mut burst = tone.clone();
    burst.resize(count, 0.0);
    let burst = interleave(&burst, &burst);
    let quiet_from = frames(0.3 + TAIL_SECONDS + 0.4, RATE);
    for effect in all_filters() {
        let out = run(&effect, &silence, 2, RATE);
        assert!(
            out.iter().all(|v| *v == 0.0),
            "{effect:?} made sound from silence"
        );
        let out = run(&effect, &burst, 2, RATE);
        assert!(
            out.iter()
                .all(|v| v.is_finite() && (*v == 0.0 || v.is_normal())),
            "{effect:?} left subnormal or non-finite samples"
        );
        assert!(
            out[2 * quiet_from..].iter().all(|v| *v == 0.0),
            "{effect:?} still sounds {:e} after its tail",
            peak(&out[2 * quiet_from..])
        );
    }
}

#[test]
fn no_filter_rises_far_above_the_input_peak() {
    let input = speech(2.5, RATE);
    let reference = peak(&input);
    for effect in all_filters() {
        let out = run(&effect, &input, 2, RATE);
        let rise = 20.0 * f64::from(peak(&out) / reference).log10();
        println!("{effect:?}: peak {rise:+.2} dB");
        // Wet effects add a delayed copy, which may meet a peak; the others shape the
        // waveform under its own peak (interpolation and envelope correction overshoot a
        // little).
        let margin = match effect {
            VoiceEffect::Echo { .. } | VoiceEffect::Reverb { .. } | VoiceEffect::Chorus { .. } => {
                1.5
            }
            _ => 1.0,
        };
        assert!(
            rise <= margin,
            "{effect:?} peaks {rise:+.2} dB above its input"
        );
    }
}

#[test]
fn zero_strength_and_unknown_types_change_nothing() {
    let input = speech(0.5, RATE);
    for effect in [
        VoiceEffect::Pitch {
            semitones: 0.0,
            keep_formants: true,
        },
        VoiceEffect::Pitch {
            semitones: f32::NAN,
            keep_formants: false,
        },
        VoiceEffect::Robot {
            frequency: 80.0,
            depth: 0.0,
        },
        VoiceEffect::Echo {
            delay_ms: 100.0,
            feedback: 0.5,
            mix: 0.0,
        },
        VoiceEffect::Reverb {
            room: 1.0,
            damping: 0.0,
            mix: f32::NAN,
        },
        VoiceEffect::Chorus {
            rate: 1.0,
            depth: 1.0,
            mix: 0.0,
        },
        VoiceEffect::Broken { amount: 0.0 },
        VoiceEffect::Whisper { amount: -1.0 },
    ] {
        assert_eq!(run(&effect, &input, 2, RATE), input, "{effect:?}");
        assert_eq!(effect.lookahead_frames(RATE), 0, "{effect:?}");
    }
    assert!(VoiceEffect::from_values("audio.denoise", &BTreeMap::new()).is_none());
}

#[test]
fn registry_parameters_reach_the_filters() {
    let defaults = |name: &str| -> BTreeMap<String, ParamValue> {
        effect_type(name)
            .unwrap_or_else(|| panic!("{name} is registered"))
            .params
            .iter()
            .map(|p| (p.name.to_string(), p.default.value()))
            .collect()
    };
    for name in [
        names::PITCH,
        names::ROBOT,
        names::TELEPHONE,
        names::MEGAPHONE,
        names::ECHO,
        names::REVERB,
        names::CHORUS,
        names::BROKEN,
        names::WHISPER,
    ] {
        let values = defaults(name);
        let effect = VoiceEffect::from_values(name, &values).unwrap_or_else(|| panic!("{name}"));
        // Every parameter is read: changing any of them changes the filter.
        for (param, value) in &values {
            let mut changed = values.clone();
            changed.insert(
                param.clone(),
                match value {
                    ParamValue::Number(v) => ParamValue::Number(v + 0.01),
                    ParamValue::Flag(v) => ParamValue::Flag(!v),
                    other => other.clone(),
                },
            );
            assert_ne!(
                VoiceEffect::from_values(name, &changed),
                Some(effect.clone()),
                "{name}.{param} is not read"
            );
        }
    }
    let pitch = defaults(names::PITCH);
    assert_eq!(
        VoiceEffect::from_values(names::PITCH, &pitch),
        Some(VoiceEffect::Pitch {
            semitones: 0.0,
            keep_formants: true
        })
    );
}

#[test]
fn pitch_moves_a_sine_by_the_ratio_and_keeps_its_length() {
    let rate = 48_000;
    let count = frames(2.0, rate);
    // A tone from 0.4 to 1.6 seconds.
    let (on, off) = (frames(0.4, rate), frames(1.6, rate));
    for (freq, semitones) in [
        (440.0, 7.0),
        (440.0, -12.0),
        (300.0, 12.0),
        (220.0, -5.0),
        (523.25, 3.0),
    ] {
        let mut input = vec![0.0f32; count];
        let tone = sine(freq, 0.5, off - on, rate);
        input[on..off].copy_from_slice(&tone);
        let expected = freq * 2f64.powf(semitones / 12.0);
        for keep_formants in [false, true] {
            let effect = VoiceEffect::Pitch {
                semitones: semitones as f32,
                keep_formants,
            };
            let out = run(&effect, &input, 1, rate);
            assert_eq!(out.len(), input.len());
            let steady = &out[frames(0.6, rate)..frames(1.4, rate)];
            let measured = peak_frequency(steady, rate, expected * 0.9, expected * 1.1);
            let error = measured / expected - 1.0;
            // Where the tone starts and stops: frames of 10 ms above half its level.
            let block = frames(0.01, rate);
            let level = power(steady);
            let loud: Vec<usize> = out
                .chunks(block)
                .enumerate()
                .filter(|(_, b)| power(b) > 0.25 * level)
                .map(|(i, _)| i * block)
                .collect();
            let (start, end) = (loud[0], loud[loud.len() - 1] + block);
            let level_change = db(level / power(&input[frames(0.6, rate)..frames(1.4, rate)]));
            println!(
                "{freq} Hz {semitones:+} st formants {keep_formants}: {measured:.2} Hz \
                 ({:+.3}%), {:.3} to {:.3} s, level {level_change:+.2} dB",
                100.0 * error,
                start as f64 / f64::from(rate),
                end as f64 / f64::from(rate)
            );
            assert!(error.abs() < 0.01, "{measured} Hz, expected {expected}");
            let tolerance = frames(0.03, rate);
            assert!(start.abs_diff(on) <= tolerance, "starts at {start}");
            assert!(end.abs_diff(off) <= tolerance, "ends at {end}");
            // With formants kept, a lone sine is its own formant: the correction keeps the
            // frame's energy, but the spectrum's balance is the input's.
            assert!(
                level_change.abs() < 1.5,
                "level changed by {level_change:.2} dB"
            );
        }
    }
}

/// The power-weighted mean frequency of `x` between 100 Hz and `top`.
fn centroid(x: &[f32], rate: u32, top: f64) -> f64 {
    let n = x.len().next_power_of_two();
    let mut buffer: Vec<rustfft::num_complex::Complex<f64>> = (0..n)
        .map(|i| {
            let v = x.get(i).map_or(0.0, |v| f64::from(*v));
            let hann = 0.5 - 0.5 * (TAU * i as f64 / x.len() as f64).cos();
            rustfft::num_complex::Complex::new(if i < x.len() { v * hann } else { 0.0 }, 0.0)
        })
        .collect();
    rustfft::FftPlanner::new()
        .plan_fft_forward(n)
        .process(&mut buffer);
    let (mut weighted, mut total) = (0.0, 0.0);
    for (k, z) in buffer.iter().enumerate().take(n / 2) {
        let f = k as f64 * f64::from(rate) / n as f64;
        if (100.0..top).contains(&f) {
            weighted += f * z.norm_sqr();
            total += z.norm_sqr();
        }
    }
    weighted / total
}

#[test]
fn kept_formants_stay_while_the_pitch_moves() {
    let rate = 48_000;
    // Harmonics of 150 Hz through one resonance at 1 kHz: a vowel-like spectrum whose
    // centroid sits near the resonance, whatever the pitch.
    let count = frames(1.6, rate);
    let input: Vec<f32> = (0..count)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            (1..=40)
                .map(|h| {
                    let f = 150.0 * h as f64;
                    let gain = 1.0 / (1.0 + ((f - 1000.0) / 220.0).powi(2));
                    0.08 * gain * (TAU * f * t + h as f64).sin()
                })
                .sum::<f64>() as f32
        })
        .collect();
    let steady = frames(0.3, rate)..frames(1.3, rate);
    let original = centroid(&input[steady.clone()], rate, 6000.0);
    for semitones in [7.0f32, -7.0] {
        let ratio = 2f64.powf(f64::from(semitones) / 12.0);
        let moved = run(
            &VoiceEffect::Pitch {
                semitones,
                keep_formants: false,
            },
            &input,
            1,
            rate,
        );
        let kept = run(
            &VoiceEffect::Pitch {
                semitones,
                keep_formants: true,
            },
            &input,
            1,
            rate,
        );
        let (moved, kept) = (
            centroid(&moved[steady.clone()], rate, 6000.0),
            centroid(&kept[steady.clone()], rate, 6000.0),
        );
        println!(
            "{semitones:+} st: centroid {original:.0} Hz, formants moved {moved:.0} Hz ({:.3} x), \
             kept {kept:.0} Hz ({:.3} x)",
            moved / original,
            kept / original
        );
        assert!(
            (moved / original / ratio - 1.0).abs() < 0.12,
            "moved {moved}"
        );
        assert!((kept / original - 1.0).abs() < 0.08, "kept {kept}");
    }
}

#[test]
fn the_robot_ring_modulates() {
    let rate = 48_000;
    let input = sine(1000.0, 0.5, frames(1.0, rate), rate);
    let reference = tone_power(&input, 1000.0, rate);
    let steady = frames(0.1, rate)..input.len();
    for (depth, carrier_db, sideband_db) in [(1.0f32, None, -6.02), (0.5, Some(-6.02), -12.04)] {
        let out = run(
            &VoiceEffect::Robot {
                frequency: 100.0,
                depth,
            },
            &input,
            1,
            rate,
        );
        let at = |f: f64| db(tone_power(&out[steady.clone()], f, rate) / reference);
        let (carrier, below, above) = (at(1000.0), at(900.0), at(1100.0));
        println!(
            "depth {depth}: 1000 Hz {carrier:.2} dB, 900 Hz {below:.2} dB, 1100 Hz {above:.2} dB"
        );
        for side in [below, above] {
            assert!((side - sideband_db).abs() < 0.3, "sideband {side:.2} dB");
        }
        match carrier_db {
            Some(expected) => assert!((carrier - expected).abs() < 0.3, "{carrier:.2} dB"),
            None => assert!(carrier < -40.0, "the tone is left at {carrier:.2} dB"),
        }
    }
}

#[test]
fn the_telephone_keeps_only_its_band() {
    let count = frames(1.0, RATE);
    let steady = frames(0.3, RATE)..count;
    let gain = |freq: f64, effect: &VoiceEffect| {
        let input = sine(freq, 0.3, count, RATE);
        let out = run(effect, &input, 1, RATE);
        db(tone_power(&out[steady.clone()], freq, RATE)
            / tone_power(&input[steady.clone()], freq, RATE))
    };
    let phone = VoiceEffect::Telephone {
        narrow: 0.3,
        crunch: 0.3,
    };
    let (low, mid, high) = (
        gain(100.0, &phone),
        gain(1000.0, &phone),
        gain(6000.0, &phone),
    );
    println!("telephone: 100 Hz {low:.1} dB, 1 kHz {mid:.2} dB, 6 kHz {high:.1} dB");
    assert!(
        low <= -20.0 && high <= -20.0,
        "100 Hz {low:.1} dB, 6 kHz {high:.1} dB"
    );
    assert!(mid.abs() < 1.0, "1 kHz {mid:.2} dB");
    // Narrow pulls the edges in.
    let narrow = VoiceEffect::Telephone {
        narrow: 1.0,
        crunch: 0.0,
    };
    let (edge_low, edge_high) = (gain(400.0, &narrow), gain(3000.0, &narrow));
    println!("narrow: 400 Hz {edge_low:.1} dB, 3 kHz {edge_high:.1} dB");
    assert!(edge_low <= -10.0 && edge_high <= -10.0);
}

#[test]
fn the_megaphone_honks_and_drive_adds_harmonics() {
    let count = frames(1.0, RATE);
    let steady = frames(0.3, RATE)..count;
    let input = sine(600.0, 0.3, count, RATE);
    let harmonic = |drive: f32| {
        let out = run(&VoiceEffect::Megaphone { drive }, &input, 1, RATE);
        let out = &out[steady.clone()];
        db(tone_power(out, 1800.0, RATE) / tone_power(out, 600.0, RATE))
    };
    let (clean, driven) = (harmonic(0.0), harmonic(1.0));
    println!("third harmonic: drive 0 {clean:.1} dB, drive 1 {driven:.1} dB");
    assert!(clean < -60.0 && driven > -25.0);
    let gain = |freq: f64| {
        let input = sine(freq, 0.3, count, RATE);
        let out = run(&VoiceEffect::Megaphone { drive: 0.0 }, &input, 1, RATE);
        db(tone_power(&out[steady.clone()], freq, RATE)
            / tone_power(&input[steady.clone()], freq, RATE))
    };
    let (low, horn, high) = (gain(150.0), gain(1800.0), gain(7000.0));
    println!("megaphone: 150 Hz {low:.1} dB, 1.8 kHz {horn:.2} dB, 7 kHz {high:.1} dB");
    assert!(
        low < -25.0 && high < -20.0 && horn > -1.0,
        "{low} {horn} {high}"
    );
}

#[test]
fn the_echo_repeats_at_the_delay_and_level() {
    let at = frames(0.1, RATE);
    let input = impulse(at, frames(2.0, RATE));
    let effect = VoiceEffect::Echo {
        delay_ms: 200.0,
        feedback: 0.5,
        mix: 0.4,
    };
    let out = run(&effect, &input, 1, RATE);
    let delay = frames(0.2, RATE);
    // Equal power at mix 0.4: the dry sound at cos(0.2 pi), the echoes at sin(0.2 pi).
    let (dry, wet) = ((0.2 * PI).cos() as f32, (0.2 * PI).sin() as f32);
    assert!((out[at] - dry).abs() < 1e-6, "dry {}", out[at]);
    for (k, level) in [(1, 1.0f32), (2, 0.5), (3, 0.25)] {
        let got = out[at + k * delay];
        println!("repeat {k} after {} ms: {got:.6} of the impulse", k * 200);
        assert!((got - wet * level).abs() < 1e-6, "repeat {k}: {got}");
    }
    // Only the repeats: nothing in between, and nothing after the tail.
    let repeats: usize = out.iter().filter(|v| v.abs() > 1e-9).count();
    assert_eq!(repeats, 1 + effect.warm_up_frames(RATE) / delay);
    let horizon = at + frames(TAIL_SECONDS, RATE);
    assert!(out[horizon + 1..].iter().all(|v| *v == 0.0));
    // A single repeat without feedback.
    let once = run(
        &VoiceEffect::Echo {
            delay_ms: 500.0,
            feedback: 0.0,
            mix: 1.0,
        },
        &input,
        1,
        RATE,
    );
    assert_eq!(once.iter().filter(|v| v.abs() > 1e-9).count(), 1);
    assert!((once[at + frames(0.5, RATE)] - 1.0).abs() < 1e-6);
}

/// Seconds for the reverb's energy decay curve (Schroeder's backward integration) to fall
/// from -5 to -25 dB, times three: the T20 estimate of its reverberation time.
fn t20(ir: &[f32], rate: u32) -> f64 {
    let mut edc: Vec<f64> = ir.iter().map(|v| f64::from(*v).powi(2)).collect();
    for i in (0..edc.len() - 1).rev() {
        edc[i] += edc[i + 1];
    }
    let total = edc[0];
    let when = |level: f64| {
        edc.iter()
            .position(|e| db(e / total) <= level)
            .unwrap_or(edc.len()) as f64
            / f64::from(rate)
    };
    3.0 * (when(-25.0) - when(-5.0))
}

#[test]
fn the_reverb_tail_decays_smoothly_and_grows_with_the_room() {
    let input = impulse(0, frames(2.0, RATE));
    let response = |room: f32, damping: f32| {
        run(
            &VoiceEffect::Reverb {
                room,
                damping,
                mix: 1.0,
            },
            &input,
            1,
            RATE,
        )
    };
    let mut times = Vec::new();
    for room in [0.0f32, 0.3, 0.6, 1.0] {
        let ir = response(room, 0.5);
        let time = t20(&ir, RATE);
        // The energy in 20 ms frames falls without jumps after the first reflections.
        let block = frames(0.02, RATE);
        let levels: Vec<f64> = ir.chunks(block).map(|b| db(power(b))).collect();
        // From after the first reflections of the largest room.
        let onset = frames(0.15, RATE) / block;
        let until = (frames(0.9 * TAIL_SECONDS, RATE) / block).min(levels.len() - 1);
        let worst_rise = levels[onset..until]
            .windows(2)
            .map(|w| w[1] - w[0])
            .fold(f64::NEG_INFINITY, f64::max);
        println!("room {room}: T20 {time:.2} s, largest rise between frames {worst_rise:.2} dB");
        assert!(
            worst_rise < 3.0,
            "room {room}: the tail jumps by {worst_rise:.2} dB"
        );
        // After the tail only the convolution's rounding is left, then nothing.
        let tail = frames(TAIL_SECONDS, RATE) + 1;
        assert!(ir[tail..].iter().all(|v| v.abs() < 1e-12));
        assert!(ir[tail + frames(0.2, RATE)..].iter().all(|v| *v == 0.0));
        times.push(time);
    }
    assert!(times.windows(2).all(|w| w[1] > w[0] * 1.2), "{times:?}");
    assert!(times[0] < 0.5 && times[3] > 1.2, "{times:?}");
    // Damping takes the highs away faster: the late tail's 3 to 6 kHz share drops.
    let late = frames(0.4, RATE)..frames(0.8, RATE);
    let share = |ir: &[f32]| {
        let tail = &ir[late.clone()];
        let high: f64 = (30..60)
            .map(|f| tone_power(tail, f64::from(f) * 100.0, RATE))
            .sum();
        let low: f64 = (3..10)
            .map(|f| tone_power(tail, f64::from(f) * 100.0, RATE))
            .sum();
        db(high / low)
    };
    let (bright, dark) = (share(&response(0.6, 0.0)), share(&response(0.6, 1.0)));
    println!("late highs against lows: damping 0 {bright:.1} dB, damping 1 {dark:.1} dB");
    assert!(dark < bright - 10.0);
    // The two channels hear different reflections.
    let stereo = interleave(&input, &input);
    let out = run(
        &VoiceEffect::Reverb {
            room: 0.5,
            damping: 0.5,
            mix: 1.0,
        },
        &stereo,
        2,
        RATE,
    );
    let (l, r) = (channel(&out, 2, 0), channel(&out, 2, 1));
    let dot: f64 = l
        .iter()
        .zip(&r)
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum();
    let correlation = dot / (power(&l) * power(&r)).sqrt() / l.len() as f64;
    println!("left/right correlation {correlation:.3}");
    assert!(correlation.abs() < 0.3);
}

#[test]
fn the_chorus_spreads_a_tone_around_its_pitch() {
    let rate = 48_000;
    let input = sine(1000.0, 0.5, frames(3.0, rate), rate);
    let steady = frames(0.5, rate)..input.len();
    let purity = |depth: f32| {
        let out = run(
            &VoiceEffect::Chorus {
                rate: 1.0,
                depth,
                mix: 1.0,
            },
            &input,
            1,
            rate,
        );
        let out = &out[steady.clone()];
        tone_power(out, 1000.0, rate) / power(out)
    };
    let (still, moving) = (purity(0.0), purity(1.0));
    println!("share of the power at 1 kHz: depth 0 {still:.3}, depth 1 {moving:.3}");
    assert!(still > 0.9, "fixed delays keep a pure tone: {still}");
    assert!(moving < 0.3, "moving delays spread it: {moving}");
}

#[test]
fn the_broken_mic_is_deterministic_crunchy_and_cuts_out() {
    let input = speech(2.5, RATE);
    let broken = VoiceEffect::Broken { amount: 1.0 };
    let out = run(&broken, &input, 2, RATE);
    assert_eq!(out, run(&broken, &input, 2, RATE));
    // Steady sound, so cut-outs show: a loud tone, in 50 ms stretches.
    let tone = sine(440.0, 0.5, frames(4.0, RATE), RATE);
    let out = run(&broken, &tone, 1, RATE);
    let stretch = frames(0.05, RATE);
    let cut = out
        .chunks(stretch)
        .skip(1)
        .filter(|b| db(power(b) / 0.125) < -15.0)
        .count();
    let stretches = out.len() / stretch - 1;
    println!("cut-outs: {cut} of {stretches} stretches");
    assert!(
        cut >= stretches / 10 && cut <= stretches / 2,
        "{cut} of {stretches}"
    );
    // Low resolution: at amount 1 the signal sits on 2^3 steps either side of the peak of
    // the last 30 ms (times the 0.65 left by the crackle's share), except where it crackles
    // or ramps.
    let window = frames(0.03, RATE);
    let (mut on_grid, mut counted) = (0, 0);
    for (s, block) in out.chunks(stretch).enumerate().skip(1) {
        if db(power(block) / 0.125) < -6.0 {
            continue;
        }
        for (j, v) in block.iter().enumerate() {
            let i = s * stretch + j;
            let recent = tone[i + 1 - window..=i]
                .iter()
                .fold(0.0f32, |m, v| m.max(v.abs()));
            let units = f64::from(*v) / (0.65 * f64::from(recent) / 8.0);
            counted += 1;
            if (units - units.round()).abs() < 0.02 {
                on_grid += 1;
            }
        }
    }
    let share = f64::from(on_grid) / f64::from(counted);
    println!(
        "{:.1}% of samples outside cut-outs on the 4-bit grid",
        100.0 * share
    );
    assert!(share > 0.85, "{share}");
    // Lighter is gentler: fewer cut-outs.
    let light = run(&VoiceEffect::Broken { amount: 0.3 }, &tone, 1, RATE);
    let light_cut = light
        .chunks(stretch)
        .skip(1)
        .filter(|b| db(power(b) / 0.125) < -15.0)
        .count();
    assert!(light_cut < cut);
}

#[test]
fn the_whisper_keeps_the_bands_and_loses_the_pitch() {
    let rate = RATE;
    let count = frames(1.5, rate);
    // Harmonics of 200 Hz, steady.
    let input: Vec<f32> = (0..count)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            (1..=15)
                .map(|h| 0.05 * (TAU * 200.0 * h as f64 * t).sin())
                .sum::<f64>() as f32
        })
        .collect();
    let out = run(&VoiceEffect::Whisper { amount: 1.0 }, &input, 1, rate);
    let steady = frames(0.3, rate)..count;
    let (x, y) = (&input[steady.clone()], &out[steady.clone()]);
    // The harmonic at 1 kHz against everything between 900 and 1100 Hz.
    let band = |s: &[f32]| {
        (900..=1100)
            .step_by(5)
            .map(|f| tone_power(s, f64::from(f), rate))
            .sum::<f64>()
    };
    let tonal = |s: &[f32]| tone_power(s, 1000.0, rate) / band(s);
    let level = db(power(y) / power(x));
    println!(
        "1 kHz share of its band: input {:.3}, whisper {:.3}; level {level:+.2} dB",
        tonal(x),
        tonal(y)
    );
    assert!(tonal(x) > 0.2 && tonal(y) < 0.05);
    assert!(level.abs() < 4.0, "level changed by {level:.2} dB");
}

#[test]
fn presets_are_known_filters_that_change_the_voice_the_same_in_blocks() {
    let input = speech(2.6, RATE);
    let reference = peak(&input);
    let ranges = [(frames(2.0, RATE), frames(2.3, RATE))];
    let chunks = |chain: &[AudioEffect]| {
        let whole = effects::process(chain, &input, 2, RATE);
        let mut worst = 0.0f32;
        for (from, to) in ranges {
            for at in (from..to).step_by(1000) {
                let end = (at + 1000).min(to);
                let got = window(chain, &input, 2, RATE, at, end);
                worst = worst.max(max_difference(&got, &whole[2 * at..2 * end]));
            }
        }
        worst
    };
    for preset in voice_presets() {
        let chain: Vec<AudioEffect> = preset
            .effects
            .iter()
            .map(|e| {
                let spec =
                    effect_type(e.effect_type).unwrap_or_else(|| panic!("{}", e.effect_type));
                let mut values: BTreeMap<String, ParamValue> = spec
                    .params
                    .iter()
                    .map(|p| (p.name.to_string(), p.default.value()))
                    .collect();
                for (name, value) in &e.params {
                    values.insert((*name).to_string(), value.clone());
                }
                let voice = VoiceEffect::from_values(e.effect_type, &values)
                    .unwrap_or_else(|| panic!("{}: {}", preset.name, e.effect_type));
                AudioEffect::Voice(voice)
            })
            .collect();
        let out = effects::process(&chain, &input, 2, RATE);
        assert!(out.iter().all(|v| v.is_finite()));
        let change = db(power(
            &out.iter()
                .zip(&input)
                .map(|(a, b)| a - b)
                .collect::<Vec<_>>(),
        ) / power(&input));
        let rise = 20.0 * f64::from(peak(&out) / reference).log10();
        let level = db(power(&out) / power(&input));
        let worst = chunks(&chain);
        println!(
            "{}: difference {change:+.1} dB, level {level:+.1} dB, peak {rise:+.2} dB, blocks {worst:e}",
            preset.name
        );
        assert!(change > -20.0, "{} barely changes the voice", preset.name);
        assert!(
            rise <= 1.5,
            "{} peaks {rise:+.2} dB above its input",
            preset.name
        );
        assert!(worst <= 1e-3, "{}: blocks differ by {worst}", preset.name);
    }
}
