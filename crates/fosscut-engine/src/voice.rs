//! Voiceovers from text (ADR-0033 item 5): speech synthesized with a Kokoro voice is written
//! into the bundle's `speech/` folder as a 24 kHz mono WAV, then imported and placed like a
//! library sound. [`dub`] speaks every page of a caption track, one clip per page at the
//! page's start.
//!
//! **Files.** A speech file is named by a hash of the text, the voice and its file, the
//! speed, the model and pronouncing dictionary files and the synthesis version, so the
//! same request finds its file again and nothing is synthesized twice; synthesis on the CPU
//! is deterministic, so the file is the same on every machine.
//!
//! **Calls, not changes.** Like [`crate::captions`], these functions write only the speech
//! files. They return the `media.add` and `clip.add` calls a front end applies in one
//! request with `expected_revision` set to the returned `revision`: the calls name the
//! asset ids `media.add` will give, which are only right at that revision. Every
//! `media.add` comes before the first `clip.add`, so the ids follow one another.

use std::path::{Path, PathBuf};

use fosscut_analysis::models;
use fosscut_analysis::voice::{self as tts, Synthesizer, VoiceError};
use fosscut_model::{ContentIdentity, Id, IdKind, Project};
use fosscut_time::{FLICKS_PER_SECOND, Flicks};
use serde::Serialize;
use serde_json::{Value, json};

pub use fosscut_analysis::voice::VoiceInfo;

use crate::EngineError;
use crate::captions::{Page, pages};

/// The voices of the installed (or installable) Kokoro model, for front ends to list.
pub fn voices(models: &Path) -> Vec<VoiceInfo> {
    tts::voices(models)
}

/// The bundle folder speech is written to, as relative paths start.
pub const FOLDER: &str = "speech/";
/// How much faster than asked [`dub`] speaks a page whose speech is longer than its room.
pub const MAX_FIT: f32 = 1.25;

impl From<VoiceError> for EngineError {
    fn from(error: VoiceError) -> EngineError {
        match error {
            VoiceError::Model(error) => EngineError::from(error),
            VoiceError::Onnx(error) => EngineError::from(error),
            VoiceError::EmptyText | VoiceError::InvalidSpeed { .. } => {
                EngineError::InvalidArguments {
                    message: error.to_string(),
                }
            }
            other => EngineError::Failed {
                code: other.code().to_string(),
                message: other.to_string(),
            },
        }
    }
}

/// A speech file in the bundle.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpeechFile {
    /// Its path in the bundle, such as `speech/3f9c0e1a2b4d5c6e7f801234.wav`.
    pub relative: String,
    /// True when the file was there already and nothing was synthesized.
    pub reused: bool,
    /// How long it lasts.
    pub duration: Flicks,
    /// The same in seconds.
    pub seconds: f64,
    /// The voice.
    pub voice: String,
    /// The speed it was spoken at.
    pub speed: f32,
    /// The `media.add` arguments that import it.
    pub media_add: Value,
}

/// A speed as speech files are keyed: checked, and rounded to hundredths.
pub fn speed(speed: Option<f32>) -> Result<f32, EngineError> {
    let speed = speed.unwrap_or(1.0);
    if !speed.is_finite() || !(tts::MIN_SPEED..=tts::MAX_SPEED).contains(&speed) {
        return Err(VoiceError::InvalidSpeed { speed }.into());
    }
    Ok((speed * 100.0).round() / 100.0)
}

/// The file name (without folder) for speaking `text` with `voice` at `speed`: a hash of
/// them, the checksums of the voice, model and pronouncing dictionary files, and the
/// synthesis version.
pub fn speech_name(text: &str, voice: &str, speed: f32) -> Result<String, EngineError> {
    tts::voice(voice)?;
    let model = models::manifest(tts::MODEL)?;
    let sha = |name: &str| {
        model
            .files
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.sha256)
            .unwrap_or_default()
    };
    let key = format!(
        "fosscut-speech\n{}\n{}\n{}\n{}\n{voice}\n{}\n{speed:.2}\n{text}",
        tts::VERSION,
        model.name,
        sha("model.onnx"),
        sha("cmudict/cmudict.dict"),
        sha(&format!("voices/{voice}.bin")),
    );
    let hash = blake3::hash(key.as_bytes()).to_hex();
    Ok(format!("{}.wav", &hash[..24]))
}

/// The asset name for speech of `text`: `Speech: ` and its first words, up to 32
/// characters, with `...` when there is more.
pub fn asset_name(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut start = String::new();
    for word in &words {
        let length = start.chars().count() + usize::from(!start.is_empty()) + word.chars().count();
        if length > 32 {
            break;
        }
        if !start.is_empty() {
            start.push(' ');
        }
        start.push_str(word);
    }
    if start.is_empty() {
        start = words
            .first()
            .map(|w| w.chars().take(32).collect())
            .unwrap_or_default();
    }
    if start.chars().count() < words.join(" ").chars().count() {
        format!("Speech: {start}...")
    } else {
        format!("Speech: {start}")
    }
}

/// Sample frames in a speech file this module wrote: a 16-bit mono WAV at the voice's rate
/// whose data fills the file; `None` for anything else, which is then written again.
fn frames(path: &Path) -> Option<u64> {
    let bytes = std::fs::read(path).ok()?;
    let header = bytes.get(..44)?;
    let u16_at = |i: usize| u16::from_le_bytes([header[i], header[i + 1]]);
    let u32_at =
        |i: usize| u32::from_le_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
    let fits = &header[..4] == b"RIFF"
        && &header[8..16] == b"WAVEfmt "
        && u16_at(20) == 1
        && u16_at(22) == 1
        && u32_at(24) == tts::RATE
        && u16_at(34) == 16
        && &header[36..40] == b"data"
        && u64::from(u32_at(40)) + 44 == bytes.len() as u64
        && u32_at(40) > 0;
    fits.then(|| u64::from(u32_at(40)) / 2)
}

/// The words FossCut spoke into `file`, when it is a speech file of a bundle's speech
/// folder with its text beside it.
pub fn spoken_text(file: &Path) -> Option<String> {
    let in_speech_folder = file
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| format!("{}/", name.to_string_lossy()) == FOLDER);
    if !in_speech_folder || file.extension().is_none_or(|e| e != "wav") {
        return None;
    }
    std::fs::read_to_string(file.with_extension("txt"))
        .ok()
        .filter(|text| !text.trim().is_empty())
}

/// Writes speech of `text` with `voice` at `speed` into `bundle`'s speech folder, unless the
/// file is there already, and returns it with the `media.add` arguments that import it.
/// The voice model in `models` is loaded into `synthesizer` the first time something has to
/// be synthesized.
pub fn speech_file(
    synthesizer: &mut Option<Synthesizer>,
    bundle: &Path,
    models: &Path,
    text: &str,
    voice: &str,
    speed: f32,
) -> Result<SpeechFile, EngineError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(VoiceError::EmptyText.into());
    }
    let relative = format!("{FOLDER}{}", speech_name(text, voice, speed)?);
    let path = bundle.join(&relative);
    let io = |what: &str, e: std::io::Error| EngineError::Failed {
        code: "fosscut.voice.write_failed".into(),
        message: format!("cannot {what} {}: {e}", path.display()),
    };
    let (count, reused) = match frames(&path) {
        Some(count) => (count, true),
        None => {
            let speech = match synthesizer {
                Some(synthesizer) => synthesizer.speak(text, voice, speed)?,
                None => synthesizer
                    .insert(Synthesizer::open(models)?)
                    .speak(text, voice, speed)?,
            };
            let bytes = fosscut_audio::wav::wav_bytes(&speech.samples, 1, tts::RATE);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io("create the folder for", e))?;
            }
            // Written beside it and renamed, so a reader never finds half a file.
            let partial = path.with_extension("wav.part");
            std::fs::write(&partial, &bytes).map_err(|e| io("write", e))?;
            std::fs::rename(&partial, &path).map_err(|e| io("write", e))?;
            (speech.samples.len() as u64, false)
        }
    };
    // The exact words beside the sound, so captions of it spell them as written rather
    // than as a recognizer hears them (see `spoken_text`).
    let words = path.with_extension("txt");
    if !words.is_file() {
        std::fs::write(&words, text).map_err(|e| io("write", e))?;
    }
    let identity = crate::import::identity(&path).map_err(|e| EngineError::Failed {
        code: e.code,
        message: e.message,
    })?;
    let duration = Flicks::new(
        (i128::from(count) * i128::from(FLICKS_PER_SECOND) / i128::from(tts::RATE)) as i64,
    );
    let media_add = json!({
        "name": asset_name(text),
        "kind": "audio",
        "path": { "relative": relative },
        "identity": identity,
        "info": {
            "duration": duration.get(),
            "container": "wav",
            "audio": { "sample_rate": tts::RATE, "channels": 1, "codec": "pcm_s16le" },
        },
    });
    Ok(SpeechFile {
        relative,
        reused,
        duration,
        seconds: count as f64 / f64::from(tts::RATE),
        voice: voice.to_string(),
        speed,
        media_add,
    })
}

/// The asset ids `media.add` calls will give, from the project's next id.
struct Assets {
    next: u64,
    known: Vec<(ContentIdentity, Id)>,
}

impl Assets {
    fn of(project: &Project) -> Assets {
        Assets {
            next: project.manifest.next_id,
            known: project
                .media
                .assets
                .iter()
                .filter_map(|a| a.identity.clone().map(|i| (i, a.id.clone())))
                .collect(),
        }
    }

    /// The asset importing `media_add` gives, and whether it is a new one (`media.add`
    /// returns the asset with the same content when there is one).
    fn add(&mut self, media_add: &Value) -> Result<(Id, bool), EngineError> {
        let identity: ContentIdentity = serde_json::from_value(media_add["identity"].clone())
            .map_err(|e| EngineError::Failed {
                code: "fosscut.voice.write_failed".into(),
                message: format!("the speech file has no content identity: {e}"),
            })?;
        if let Some((_, id)) = self.known.iter().find(|(known, _)| *known == identity) {
            return Ok((id.clone(), false));
        }
        let id = Id::new(IdKind::Asset, self.next);
        self.next += 1;
        self.known.push((identity, id.clone()));
        Ok((id, true))
    }
}

fn call(command: &str, args: Value) -> Value {
    json!({ "command": command, "args": args })
}

/// What [`speak`] says and where it goes.
#[derive(Clone, Debug, Default)]
pub struct SpeakRequest {
    /// The project bundle, where the speech file is written.
    pub bundle: PathBuf,
    /// The models folder.
    pub models: PathBuf,
    /// What to say.
    pub text: String,
    /// The voice; [`tts::DEFAULT_VOICE`] when `None`.
    pub voice: Option<String>,
    /// The speed, from 0.5 to 2; 1 when `None`.
    pub speed: Option<f32>,
    /// The text's language; English when `None`, and nothing else has a voice.
    pub language: Option<String>,
    /// Timeline; the primary timeline when `None`.
    pub timeline: Option<Id>,
    /// The audio track; without it, the first audio track free at `start` (one is added
    /// when none is), or after the first audio track's last clip when `start` is `None`.
    pub track: Option<Id>,
    /// Where the speech starts, as a time input.
    pub start: Option<Value>,
}

/// Speech ready to import and place.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Spoken {
    /// The project revision the calls are for.
    pub revision: u64,
    /// The asset the speech is, once imported.
    pub asset: Id,
    /// The speech file.
    pub file: SpeechFile,
    /// `media.add` (unless the project has the file already) and `clip.add`.
    pub calls: Vec<Value>,
}

/// Synthesizes `request.text` into the bundle (or finds it there) and returns the calls
/// that import it and place it.
pub fn speak(project: &Project, request: &SpeakRequest) -> Result<Spoken, EngineError> {
    tts::check_language(request.language.as_deref().unwrap_or("en"))?;
    let timeline = timeline_of(project, request.timeline.as_ref())?;
    let voice = request.voice.as_deref().unwrap_or(tts::DEFAULT_VOICE);
    let speed = speed(request.speed)?;
    let mut synthesizer = None;
    let file = speech_file(
        &mut synthesizer,
        &request.bundle,
        &request.models,
        &request.text,
        voice,
        speed,
    )?;
    let mut assets = Assets::of(project);
    let (asset, new) = assets.add(&file.media_add)?;
    let mut calls = Vec::new();
    if new {
        calls.push(call("media.add", file.media_add.clone()));
    }
    let mut place = json!({ "asset": asset, "timeline": timeline });
    if let Some(start) = &request.start {
        place["start"] = start.clone();
    }
    match &request.track {
        Some(track) => place["track"] = json!(track),
        None if request.start.is_some() => place["free_track"] = json!(true),
        None => {}
    }
    calls.push(call("clip.add", place));
    Ok(Spoken {
        revision: project.manifest.revision,
        asset,
        file,
        calls,
    })
}

fn timeline_of(project: &Project, timeline: Option<&Id>) -> Result<Id, EngineError> {
    let id = timeline
        .cloned()
        .unwrap_or_else(|| project.manifest.primary_timeline.clone());
    if project.timelines.contains_key(&id) {
        Ok(id)
    } else {
        Err(EngineError::TimelineNotFound {
            timeline: id.to_string(),
        })
    }
}

/// Which text of a caption page [`dub`] speaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DubText {
    /// The words shown.
    #[default]
    Words,
    /// The page's translation (bilingual captions); pages without one are skipped.
    Translation,
}

impl DubText {
    /// The choice named `words` or `translation`.
    pub fn named(name: &str) -> Result<DubText, EngineError> {
        match name {
            "words" => Ok(DubText::Words),
            "translation" => Ok(DubText::Translation),
            other => Err(EngineError::InvalidArguments {
                message: format!("dub text must be words or translation, not {other:?}"),
            }),
        }
    }
}

/// What [`dub`] speaks.
#[derive(Clone, Debug, Default)]
pub struct DubRequest {
    /// The project bundle, where speech files are written.
    pub bundle: PathBuf,
    /// The models folder.
    pub models: PathBuf,
    /// Timeline; the primary timeline when `None`.
    pub timeline: Option<Id>,
    /// The caption track; the timeline's first when `None`.
    pub track: Option<Id>,
    /// The voice; [`tts::DEFAULT_VOICE`] when `None`.
    pub voice: Option<String>,
    /// The speed, from 0.5 to 2; 1 when `None`.
    pub speed: Option<f32>,
    /// The language of the text spoken; English when `None`, and nothing else has a voice.
    pub language: Option<String>,
    /// The pages' words or their translations.
    pub text: DubText,
    /// The audio track for every clip; without it each clip goes on the first audio track
    /// free at its time, so speech that runs long never blocks the next page's.
    pub audio_track: Option<Id>,
}

/// One dubbed page.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DubbedPage {
    /// The caption clip.
    pub clip: Id,
    /// Where its speech starts: the page's start.
    pub start: Flicks,
    /// What was said.
    pub text: String,
    /// The time until the next page starts (the page's own length for the last page).
    pub room: Flicks,
    /// How long the speech lasts.
    pub speech: Flicks,
    /// The speed used: the one asked for, or up to [`MAX_FIT`] times faster to fit the room.
    pub speed: f32,
    /// The speech file.
    pub file: String,
    /// True when the file was there already.
    pub reused: bool,
    /// The asset the speech is, once imported.
    pub asset: Id,
    /// Set when the speech is still longer than its room. It is never cut: it runs on, on
    /// another audio track where it meets the next page's speech.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// A dubbed caption track, ready to import and place.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Dubbed {
    /// The project revision the calls are for.
    pub revision: u64,
    /// The timeline.
    pub timeline: Id,
    /// The caption track.
    pub track: Id,
    /// The voice.
    pub voice: String,
    /// Every page spoken.
    pub pages: Vec<DubbedPage>,
    /// Pages with nothing to say (no translation, or only punctuation).
    pub skipped: Vec<Id>,
    /// The `media.add` calls for new files, then one `clip.add` per page.
    pub calls: Vec<Value>,
}

/// Speaks every page of a caption track (its words, or its translations) and returns the
/// calls that import the speech and place one clip per page at the page's start. Speech
/// longer than the time until the next page is spoken faster, up to [`MAX_FIT`] times the
/// speed asked for; if it is still longer it is kept whole with a warning.
pub fn dub(project: &Project, request: &DubRequest) -> Result<Dubbed, EngineError> {
    tts::check_language(request.language.as_deref().unwrap_or("en"))?;
    let voice = request
        .voice
        .clone()
        .unwrap_or_else(|| tts::DEFAULT_VOICE.to_string());
    tts::voice(&voice)?;
    let asked = speed(request.speed)?;
    let (timeline, track, pages) =
        pages(project, request.timeline.as_ref(), request.track.as_ref())?;
    let mut synthesizer = None;
    let mut assets = Assets::of(project);
    let mut imports = Vec::new();
    let mut places = Vec::new();
    let mut dubbed = Vec::new();
    let mut skipped = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        let text = match request.text {
            DubText::Words => Some(page.text.replace('\n', " ")),
            DubText::Translation => page.translation.clone(),
        };
        let Some(text) = text.filter(|t| t.chars().any(char::is_alphanumeric)) else {
            skipped.push(page.clip.clone());
            continue;
        };
        let room = pages
            .get(index + 1)
            .map_or(page.end, |next: &Page| next.start)
            - page.start;
        let (file, used) = fit(&mut synthesizer, request, &text, &voice, asked, room)?;
        let warning = (file.duration > room).then(|| {
            format!(
                "the speech lasts {:.2} s but the page has {:.2} s before the next one, even at speed {used}; it runs on",
                file.seconds,
                seconds(room)
            )
        });
        let (asset, new) = assets.add(&file.media_add)?;
        if new {
            imports.push(call("media.add", file.media_add.clone()));
        }
        let mut place = json!({
            "asset": asset,
            "timeline": timeline,
            "start": { "flicks": page.start.get() },
        });
        match &request.audio_track {
            Some(audio) => place["track"] = json!(audio),
            None => place["free_track"] = json!(true),
        }
        places.push(call("clip.add", place));
        dubbed.push(DubbedPage {
            clip: page.clip.clone(),
            start: page.start,
            text,
            room,
            speech: file.duration,
            speed: used,
            file: file.relative,
            reused: file.reused,
            asset,
            warning,
        });
    }
    imports.extend(places);
    Ok(Dubbed {
        revision: project.manifest.revision,
        timeline,
        track,
        voice,
        pages: dubbed,
        skipped,
        calls: imports,
    })
}

fn seconds(time: Flicks) -> f64 {
    time.get() as f64 / FLICKS_PER_SECOND as f64
}

/// Speech of `text` at `asked`, or faster (up to [`MAX_FIT`] times, at most
/// [`tts::MAX_SPEED`]) when it is longer than `room`, with the speed used.
fn fit(
    synthesizer: &mut Option<Synthesizer>,
    request: &DubRequest,
    text: &str,
    voice: &str,
    asked: f32,
    room: Flicks,
) -> Result<(SpeechFile, f32), EngineError> {
    let fastest = (asked * MAX_FIT).min(tts::MAX_SPEED);
    let mut used = asked;
    let mut file = speech_file(
        synthesizer,
        &request.bundle,
        &request.models,
        text,
        voice,
        used,
    )?;
    // Speed does not shorten speech exactly in proportion, so a second try corrects the
    // first.
    for _ in 0..2 {
        if file.duration <= room || used >= fastest || room.get() <= 0 {
            break;
        }
        let needed = file.seconds / seconds(room);
        let faster = ((used * needed as f32 * 1.02 * 100.0).ceil() / 100.0).min(fastest);
        if faster <= used {
            break;
        }
        used = faster;
        file = speech_file(
            synthesizer,
            &request.bundle,
            &request.models,
            text,
            voice,
            used,
        )?;
    }
    Ok((file, used))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_files_are_named_by_what_is_said() {
        let a = speech_name("Hello there.", "af_heart", 1.0).unwrap();
        assert_eq!(a.len(), 28);
        assert!(a.ends_with(".wav"));
        assert_eq!(speech_name("Hello there.", "af_heart", 1.0).unwrap(), a);
        for other in [
            speech_name("Hello there!", "af_heart", 1.0),
            speech_name("Hello there.", "af_bella", 1.0),
            speech_name("Hello there.", "af_heart", 1.1),
        ] {
            assert_ne!(other.unwrap(), a);
        }
        let error = speech_name("Hi", "xx_nobody", 1.0).unwrap_err();
        assert_eq!(error.code(), "fosscut.voice.unknown_voice");
        assert_eq!(speed(None).unwrap(), 1.0);
        assert_eq!(speed(Some(1.234)).unwrap(), 1.23);
        assert_eq!(
            speed(Some(9.0)).unwrap_err().code(),
            "fosscut.request.invalid_arguments"
        );
        assert_eq!(asset_name("  Hello   there  "), "Speech: Hello there");
        assert_eq!(
            asset_name(&"x".repeat(40)),
            format!("Speech: {}...", "x".repeat(32))
        );
        assert_eq!(
            asset_name("The quick brown fox jumps over the lazy dog."),
            "Speech: The quick brown fox jumps over..."
        );
        assert_eq!(DubText::named("translation").unwrap(), DubText::Translation);
        assert!(DubText::named("both").is_err());
    }

    #[test]
    fn voice_errors_keep_their_codes() {
        for (error, code) in [
            (
                VoiceError::LanguageUnavailable {
                    language: "hu".into(),
                },
                "fosscut.voice.language_unavailable",
            ),
            (VoiceError::EmptyText, "fosscut.request.invalid_arguments"),
            (
                VoiceError::Failed("x".into()),
                "fosscut.voice.synthesis_failed",
            ),
            (
                VoiceError::Model(fosscut_analysis::models::ModelError::NotInstalled {
                    what: "kokoro-82m".into(),
                }),
                "fosscut.model.not_installed",
            ),
            (
                VoiceError::Onnx(fosscut_analysis::onnx::OnnxError::RuntimeMissing {
                    tried: Vec::new(),
                }),
                "fosscut.model.runtime_missing",
            ),
        ] {
            assert_eq!(EngineError::from(error).code(), code);
        }
    }

    #[test]
    fn only_wav_files_this_module_writes_are_reused() {
        let dir = std::env::temp_dir().join(format!("fosscut-speech-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.wav");
        std::fs::write(
            &good,
            fosscut_audio::wav::wav_bytes(&[0.1; 2400], 1, tts::RATE),
        )
        .unwrap();
        assert_eq!(frames(&good), Some(2400));
        let stereo = dir.join("stereo.wav");
        std::fs::write(
            &stereo,
            fosscut_audio::wav::wav_bytes(&[0.1; 2400], 2, tts::RATE),
        )
        .unwrap();
        assert_eq!(frames(&stereo), None);
        let cut = dir.join("cut.wav");
        let mut bytes = fosscut_audio::wav::wav_bytes(&[0.1; 2400], 1, tts::RATE);
        bytes.truncate(1000);
        std::fs::write(&cut, bytes).unwrap();
        assert_eq!(frames(&cut), None);
        assert_eq!(frames(&dir.join("missing.wav")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
