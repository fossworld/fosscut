//! Voice filters: sound effects that change the character of a voice (pitch, robot,
//! telephone, megaphone, echo, reverb, chorus, a broken microphone, a whisper), and voice
//! presets, named combinations of them.
//!
//! The filters are ordinary sound effects of the registry (ADR-0026): category `audio`,
//! applied by the mixer in `fosscut-audio`, with constant parameters (`animatable: false`).
//! Their specs are appended to [`crate::effect_types`]. Presets are data, like the built-in
//! looks: applying one copies its effects into the clip, which owns them afterwards.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;

use crate::{EffectCategory, EffectSpec, ParamDefault, ParamKind, ParamSpec, ParamValue};

/// `voice.pitch`: higher or deeper without changing the length.
pub const PITCH: &str = "voice.pitch";
/// `voice.robot`: ring modulation and a tuned comb.
pub const ROBOT: &str = "voice.robot";
/// `voice.telephone`: a phone line's band with light distortion.
pub const TELEPHONE: &str = "voice.telephone";
/// `voice.megaphone`: a narrow, driven horn.
pub const MEGAPHONE: &str = "voice.megaphone";
/// `voice.echo`: delayed repeats.
pub const ECHO: &str = "voice.echo";
/// `voice.reverb`: a room around the voice.
pub const REVERB: &str = "voice.reverb";
/// `voice.chorus`: drifting, detuned copies.
pub const CHORUS: &str = "voice.chorus";
/// `voice.broken`: a failing microphone.
pub const BROKEN: &str = "voice.broken";
/// `voice.whisper`: breath noise shaped like the voice.
pub const WHISPER: &str = "voice.whisper";

/// A constant number of a sound effect (ADR-0026, item 4: no keyframes).
const fn setting(
    name: &'static str,
    default: f64,
    min: f64,
    max: f64,
    unit: &'static str,
    meaning: &'static str,
) -> ParamSpec {
    ParamSpec {
        name,
        kind: ParamKind::Number,
        default: ParamDefault::Number(default),
        min: Some(min),
        max: Some(max),
        unit,
        choices: &[],
        animatable: false,
        meaning,
    }
}

/// The dry/wet balance of an effect that adds a processed copy to the voice.
const fn mix(default: f64) -> ParamSpec {
    setting(
        "mix",
        default,
        0.0,
        1.0,
        "",
        "dry/wet balance at equal power: 0 only the original voice, 1 only the effect",
    )
}

/// The specs of the voice filters, appended to the registry by [`crate::effect_types`].
pub(crate) fn specs() -> Vec<EffectSpec> {
    let spec = |name, title, summary, params| EffectSpec {
        name,
        version: 1,
        title,
        category: EffectCategory::Audio,
        summary,
        params,
    };
    vec![
        spec(
            PITCH,
            "Voice pitch",
            "Makes a voice higher or deeper without changing its length. With formants kept it still sounds like the same person; without, the whole voice is scaled, from a giant to a cartoon.",
            vec![
                setting(
                    "semitones",
                    0.0,
                    -12.0,
                    12.0,
                    "semitones",
                    "how far the pitch moves: 12 is an octave up, -12 an octave down",
                ),
                ParamSpec {
                    name: "keep_formants",
                    kind: ParamKind::Bool,
                    default: ParamDefault::Bool(true),
                    min: None,
                    max: None,
                    unit: "",
                    choices: &[],
                    animatable: false,
                    meaning: "keep the resonances that make the voice sound like its speaker where they were; off moves them with the pitch",
                },
            ],
        ),
        spec(
            ROBOT,
            "Robot voice",
            "Makes a voice metallic and robotic: ring modulation by a steady tone, then a short comb tuned to the same frequency that adds a buzzing resonance.",
            vec![
                setting(
                    "frequency",
                    60.0,
                    20.0,
                    1000.0,
                    "Hz",
                    "frequency of the modulating tone: low values buzz, high values sound alien",
                ),
                setting(
                    "depth",
                    1.0,
                    0.0,
                    1.0,
                    "",
                    "how much of the voice is modulated: 0 leaves it unchanged",
                ),
            ],
        ),
        spec(
            TELEPHONE,
            "Telephone",
            "Sounds like a voice on a phone line: only the band from about 300 to 3400 Hz is kept, with light distortion on loud passages.",
            vec![
                setting(
                    "narrow",
                    0.3,
                    0.0,
                    1.0,
                    "",
                    "how small the speaker sounds: 0 keeps 300 to 3400 Hz, 1 keeps 600 to 2400 Hz",
                ),
                setting(
                    "crunch",
                    0.3,
                    0.0,
                    1.0,
                    "",
                    "how strongly loud passages are distorted: 0 not at all",
                ),
            ],
        ),
        spec(
            MEGAPHONE,
            "Megaphone",
            "Sounds like a voice through a loudhailer: a narrow band with a horn resonance around 1.8 kHz, driven into distortion.",
            vec![setting(
                "drive",
                0.6,
                0.0,
                1.0,
                "",
                "how hard the horn is driven: 0 clean, 1 heavily distorted",
            )],
        ),
        spec(
            ECHO,
            "Echo",
            "Repeats the sound after a delay, each repeat quieter than the one before; the repeats fade out within 1.5 seconds.",
            vec![
                setting(
                    "delay",
                    250.0,
                    20.0,
                    1000.0,
                    "ms",
                    "time between the sound and its first repeat, and between repeats",
                ),
                setting(
                    "feedback",
                    0.4,
                    0.0,
                    0.9,
                    "",
                    "level of each repeat relative to the one before: 0 a single repeat",
                ),
                mix(0.3),
            ],
        ),
        spec(
            REVERB,
            "Reverb",
            "Places the voice in a space, from a small room to a large hall, with a smooth tail that ends within 1.5 seconds.",
            vec![
                setting(
                    "room",
                    0.5,
                    0.0,
                    1.0,
                    "",
                    "size of the space: 0 a small room, 1 a large hall",
                ),
                setting(
                    "damping",
                    0.5,
                    0.0,
                    1.0,
                    "",
                    "how much faster high frequencies die away: 0 bright and hard, 1 dark and soft",
                ),
                mix(0.3),
            ],
        ),
        spec(
            CHORUS,
            "Chorus",
            "Thickens a voice into several copies that drift slightly in time and pitch, wider in stereo.",
            vec![
                setting("rate", 0.8, 0.05, 5.0, "Hz", "how fast the copies drift"),
                setting(
                    "depth",
                    0.5,
                    0.0,
                    1.0,
                    "",
                    "how far the copies drift: 1 is 6 milliseconds",
                ),
                mix(0.5),
            ],
        ),
        spec(
            BROKEN,
            "Broken mic",
            "Sounds like a failing microphone: crunchy low resolution, short cut-outs and crackle, placed the same way on every render.",
            vec![setting(
                "amount",
                0.6,
                0.0,
                1.0,
                "",
                "how broken: 0 unchanged, 1 low resolution with frequent cut-outs and crackle",
            )],
        ),
        spec(
            WHISPER,
            "Whisper",
            "Turns speech into a whisper: breath noise shaped band by band like the voice replaces its pitch.",
            vec![setting(
                "amount",
                1.0,
                0.0,
                1.0,
                "",
                "how much of the voice becomes whisper: 0 none, 1 only whisper",
            )],
        ),
    ]
}

/// One effect of a voice preset, with the parameters that differ from the defaults.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct VoicePresetEffect {
    /// The effect type, such as `voice.pitch`.
    #[serde(rename = "type")]
    pub effect_type: &'static str,
    /// Its parameters by name.
    pub params: BTreeMap<&'static str, ParamValue>,
}

/// A voice preset: a named set of sound effects with parameters chosen for a character.
///
/// It serializes like a built-in look (`name`, `title`, `description`, `effects` with `type`
/// and `params`), so the same `look.apply` path can add its effects to a clip.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct VoicePreset {
    /// Stable name, for example `deep`.
    pub name: &'static str,
    /// Display name.
    pub title: &'static str,
    /// What it sounds like and what it suits.
    pub description: &'static str,
    /// The effects it adds, in order.
    pub effects: Vec<VoicePresetEffect>,
}

/// A parameter value of a preset.
#[derive(Copy, Clone)]
enum Value {
    Number(f64),
    Flag(bool),
}

/// Builds one preset effect.
fn effect(effect_type: &'static str, params: &[(&'static str, Value)]) -> VoicePresetEffect {
    VoicePresetEffect {
        effect_type,
        params: params
            .iter()
            .map(|(name, value)| {
                let value = match *value {
                    Value::Number(v) => ParamValue::Number(v),
                    Value::Flag(v) => ParamValue::Flag(v),
                };
                (*name, value)
            })
            .collect(),
    }
}

/// Every voice preset, written for FossCut.
pub fn voice_presets() -> Vec<VoicePreset> {
    use Value::{Flag, Number as N};
    let preset = |name, title, description, effects| VoicePreset {
        name,
        title,
        description,
        effects,
    };
    vec![
        preset(
            "deep",
            "Deep",
            "A lower voice that still sounds like the speaker: four semitones down, formants kept.",
            vec![effect(
                PITCH,
                &[("semitones", N(-4.0)), ("keep_formants", Flag(true))],
            )],
        ),
        preset(
            "giant",
            "Giant",
            "A huge creature: nine semitones down with the whole voice scaled, in a large space.",
            vec![
                effect(
                    PITCH,
                    &[("semitones", N(-9.0)), ("keep_formants", Flag(false))],
                ),
                effect(
                    REVERB,
                    &[("room", N(0.7)), ("damping", N(0.6)), ("mix", N(0.2))],
                ),
            ],
        ),
        preset(
            "chipmunk",
            "Chipmunk",
            "A tiny, squeaky cartoon voice: eight semitones up with the whole voice scaled.",
            vec![effect(
                PITCH,
                &[("semitones", N(8.0)), ("keep_formants", Flag(false))],
            )],
        ),
        preset(
            "robot",
            "Robot",
            "A metallic machine voice with a low buzz.",
            vec![
                effect(ROBOT, &[("frequency", N(55.0)), ("depth", N(1.0))]),
                effect(
                    REVERB,
                    &[("room", N(0.15)), ("damping", N(0.2)), ("mix", N(0.12))],
                ),
            ],
        ),
        preset(
            "alien",
            "Alien",
            "A higher, warbling voice from somewhere else: pitched up, ring-modulated and doubled.",
            vec![
                effect(
                    PITCH,
                    &[("semitones", N(5.0)), ("keep_formants", Flag(false))],
                ),
                effect(ROBOT, &[("frequency", N(320.0)), ("depth", N(0.5))]),
                effect(
                    CHORUS,
                    &[("rate", N(2.5)), ("depth", N(0.7)), ("mix", N(0.5))],
                ),
            ],
        ),
        preset(
            "phone_call",
            "Phone call",
            "The other end of a phone call.",
            vec![effect(
                TELEPHONE,
                &[("narrow", N(0.2)), ("crunch", N(0.25))],
            )],
        ),
        preset(
            "radio",
            "Radio",
            "A small transistor radio with a gritty edge.",
            vec![effect(TELEPHONE, &[("narrow", N(0.5)), ("crunch", N(0.6))])],
        ),
        preset(
            "megaphone",
            "Megaphone",
            "A loudhailer outdoors, with a little space around it.",
            vec![
                effect(MEGAPHONE, &[("drive", N(0.7))]),
                effect(
                    REVERB,
                    &[("room", N(0.6)), ("damping", N(0.5)), ("mix", N(0.12))],
                ),
            ],
        ),
        preset(
            "broken_mic",
            "Broken mic",
            "A cheap microphone that cuts out and crackles.",
            vec![
                effect(BROKEN, &[("amount", N(0.75))]),
                effect(TELEPHONE, &[("narrow", N(0.1)), ("crunch", N(0.4))]),
            ],
        ),
        preset(
            "whisper",
            "Whisper",
            "Speech turned into a close whisper in a small room.",
            vec![
                effect(WHISPER, &[("amount", N(1.0))]),
                effect(
                    REVERB,
                    &[("room", N(0.2)), ("damping", N(0.6)), ("mix", N(0.1))],
                ),
            ],
        ),
        preset(
            "stadium",
            "Stadium",
            "An announcer over a stadium's speakers: a huge space with a slap of echo.",
            vec![
                effect(
                    ECHO,
                    &[("delay", N(170.0)), ("feedback", N(0.25)), ("mix", N(0.2))],
                ),
                effect(
                    REVERB,
                    &[("room", N(1.0)), ("damping", N(0.35)), ("mix", N(0.35))],
                ),
            ],
        ),
        preset(
            "cave",
            "Cave",
            "A voice in a stone cave: bright, long reflections and a distant echo.",
            vec![
                effect(
                    ECHO,
                    &[("delay", N(380.0)), ("feedback", N(0.3)), ("mix", N(0.18))],
                ),
                effect(
                    REVERB,
                    &[("room", N(0.85)), ("damping", N(0.1)), ("mix", N(0.45))],
                ),
            ],
        ),
        preset(
            "ghost",
            "Ghost",
            "A breathy, hovering presence in an empty hall.",
            vec![
                effect(WHISPER, &[("amount", N(0.7))]),
                effect(
                    CHORUS,
                    &[("rate", N(0.3)), ("depth", N(0.8)), ("mix", N(0.5))],
                ),
                effect(
                    REVERB,
                    &[("room", N(0.75)), ("damping", N(0.4)), ("mix", N(0.4))],
                ),
            ],
        ),
    ]
}

/// The voice preset called `name`.
pub fn voice_preset(name: &str) -> Option<VoicePreset> {
    voice_presets().into_iter().find(|p| p.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect_type;

    #[test]
    fn voice_filters_are_constant_sound_effects_in_the_registry() {
        let all = specs();
        assert_eq!(all.len(), 9);
        for spec in &all {
            assert!(spec.name.starts_with("voice."), "{}", spec.name);
            assert!(spec.is_audio(), "{}", spec.name);
            assert_eq!(effect_type(spec.name).as_ref(), Some(spec), "{}", spec.name);
            assert!(!spec.params.is_empty(), "{}", spec.name);
            for p in &spec.params {
                assert!(!p.animatable, "{}.{}", spec.name, p.name);
                assert!(!p.meaning.is_empty(), "{}.{}", spec.name, p.name);
                if let (ParamDefault::Number(d), Some(lo), Some(hi)) = (p.default, p.min, p.max) {
                    assert!(
                        lo < hi && (lo..=hi).contains(&d),
                        "{}.{}",
                        spec.name,
                        p.name
                    );
                }
            }
            for text in [spec.title, spec.summary] {
                assert!(!text.contains('\u{2014}'), "{}", spec.name);
            }
        }
        for name in [
            PITCH, ROBOT, TELEPHONE, MEGAPHONE, ECHO, REVERB, CHORUS, BROKEN, WHISPER,
        ] {
            assert!(all.iter().any(|s| s.name == name), "{name}");
        }
    }

    #[test]
    fn presets_use_known_effects_with_valid_parameters() {
        let presets = voice_presets();
        assert!(presets.len() >= 8);
        let mut names: Vec<&str> = presets.iter().map(|p| p.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), presets.len(), "preset names are unique");
        for preset in &presets {
            assert!(
                preset
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_'),
                "{}",
                preset.name
            );
            assert!(!preset.effects.is_empty(), "{}", preset.name);
            assert!(!preset.description.contains('\u{2014}'), "{}", preset.name);
            for e in &preset.effects {
                let spec = effect_type(e.effect_type)
                    .unwrap_or_else(|| panic!("{}: {}", preset.name, e.effect_type));
                assert!(spec.is_audio());
                for (name, value) in &e.params {
                    let param = spec
                        .param(name)
                        .unwrap_or_else(|| panic!("{}: {}.{name}", preset.name, e.effect_type));
                    let ok = match (param.kind, value) {
                        (ParamKind::Number, ParamValue::Number(v)) => {
                            param.min.is_none_or(|lo| *v >= lo)
                                && param.max.is_none_or(|hi| *v <= hi)
                        }
                        (ParamKind::Bool, ParamValue::Flag(_)) => true,
                        _ => false,
                    };
                    assert!(ok, "{}: {}.{name} = {value:?}", preset.name, e.effect_type);
                }
            }
        }
        assert_eq!(voice_preset("deep").map(|p| p.title), Some("Deep"));
        assert!(voice_preset("nope").is_none());
    }

    #[test]
    fn presets_serialize_like_looks() {
        let json = serde_json::to_value(voice_preset("stadium")).unwrap_or_default();
        assert_eq!(json["name"], "stadium");
        assert_eq!(json["effects"][0]["type"], ECHO);
        assert_eq!(json["effects"][0]["params"]["delay"], 170.0);
        assert_eq!(json["effects"][1]["params"]["room"], 1.0);
        let deep = serde_json::to_value(voice_preset("deep")).unwrap_or_default();
        assert_eq!(deep["effects"][0]["params"]["keep_formants"], true);
    }
}
