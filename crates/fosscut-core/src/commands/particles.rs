//! Particle emitters (ADR-0038): `particles.add` puts a particle clip (sparks, embers, dust,
//! snow, confetti, bokeh, light streaks, a burst and more, from FossCut's own presets) on a
//! video track above the footage, and `particles.set` changes one by a merge patch. Both
//! take times with units for bursts and prewarm, and keep keyframed settings to
//! `keyframe.set`.

use fosscut_model::{
    Burst, Clip, ClipContent, Id, IdKind, ParticleContent, Project, Track, TrackKind, Transform,
    particle_issues,
};
use fosscut_time::{Flicks, Rate, TimeInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ClipPlacement;
use super::caption::merge;
use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, insert_sorted, locate_clip,
    resolve, timeline_id, timeline_mut, track_index,
};
use crate::{ApiError, Command, Context, codes};

const PRESETS: &str = include_str!("../../presets/library/particles.json");

/// A built-in particle emitter, made by FossCut and free to use anywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticlePreset {
    /// Stable name, for example `sparks`.
    pub name: String,
    /// Display title.
    pub title: String,
    /// What it looks like and what it suits.
    pub description: String,
    /// Search words.
    pub tags: Vec<String>,
    /// The emitter.
    pub content: ParticleContent,
}

/// Every built-in particle preset.
pub fn particle_presets() -> Vec<ParticlePreset> {
    serde_json::from_str(PRESETS).unwrap_or_default()
}

/// The particle preset called `name`.
pub fn particle_preset(name: &str) -> Result<ParticlePreset, ApiError> {
    particle_presets()
        .into_iter()
        .find(|p| p.name == name)
        .ok_or_else(|| {
            let names: Vec<String> = particle_presets().into_iter().map(|p| p.name).collect();
            ApiError::new(
                codes::LIBRARY_UNKNOWN_ITEM,
                format!("there is no particle preset {name:?}"),
            )
            .with_hint(format!("Particle presets: {}.", names.join(", ")))
        })
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::PARTICLES_INVALID, message).with_hint(
        "capabilities lists the particle presets under library.particles and every emitter field under particles.",
    )
}

/// Refuses an emitter that breaks the rules of ADR-0038, with the first problem as message.
fn checked(project: &Project, command: &str, content: &ParticleContent) -> Result<(), ApiError> {
    match particle_issues(project, content).first() {
        Some(issue) if issue.code == "fosscut.model.missing_asset" => Err(ApiError::new(
            codes::ASSET_NOT_FOUND,
            format!("{command}: {}", issue.message),
        )),
        Some(issue) => Err(invalid(format!("{command}: {}", issue.message))),
        None => Ok(()),
    }
}

/// A burst with its time in any unit.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BurstInput {
    /// When, from the clip's start: `"0.5s"`, `{"frames": 15}` and the other time forms.
    pub at: TimeInput,
    /// How many particles, 1 to 5000.
    pub count: u32,
}

/// Applies the parts of a request both commands share to `content`: the merge patch,
/// then bursts (from the clip's start), prewarm and seed.
fn change(
    command: &str,
    content: &mut ParticleContent,
    set: Option<&Value>,
    bursts: Option<&[BurstInput]>,
    prewarm: Option<&TimeInput>,
    seed: Option<u32>,
    rate: Rate,
) -> Result<(), ApiError> {
    if let Some(patch) = set {
        if !patch.is_object() {
            return Err(invalid(format!(
                "{command}: set must be an object of emitter fields, not {patch}"
            )));
        }
        let mut value = serde_json::to_value(&*content).unwrap_or_else(|_| json!({}));
        merge(&mut value, patch);
        *content = serde_json::from_value(value).map_err(|e| invalid(format!("{command}: {e}")))?;
    }
    if let Some(bursts) = bursts {
        content.bursts = bursts
            .iter()
            .map(|b| {
                Ok(Burst {
                    // Bursts are kept in emission time, which runs from the clip's start
                    // plus its offset.
                    at: content.offset + resolve(&b.at, rate)?,
                    count: b.count,
                })
            })
            .collect::<Result<Vec<Burst>, ApiError>>()?;
        content.bursts.sort_by_key(|b| b.at);
    }
    if let Some(prewarm) = prewarm {
        let prewarm = resolve(prewarm, rate)?;
        if prewarm.is_negative() {
            return Err(invalid(format!("{command}: prewarm cannot be negative")));
        }
        content.prewarm = prewarm;
    }
    if let Some(seed) = seed {
        content.seed = seed;
    }
    Ok(())
}

/// The video track particles go on: the top video track when it holds only particle clips
/// and is free over `range`, otherwise a new one above every video track.
fn particle_track(
    project: &mut Project,
    timeline: &Id,
    range: (Flicks, Flicks),
) -> Result<Id, ApiError> {
    let doc = timeline_mut(project, timeline)?;
    let top = doc.tracks.iter().rposition(|t| t.kind == TrackKind::Video);
    if let Some(index) = top {
        let track = &doc.tracks[index];
        let particles_only = !track.clips.is_empty()
            && track
                .clips
                .iter()
                .all(|c| matches!(c.content, ClipContent::Particles { .. }));
        let free = track
            .clips
            .iter()
            .all(|c| c.end() <= range.0 || c.start >= range.1);
        if particles_only && free && !track.locked {
            return Ok(track.id.clone());
        }
    }
    let id = project.allocate_id(IdKind::Track);
    let doc = timeline_mut(project, timeline)?;
    doc.tracks.insert(
        top.map_or(0, |i| i + 1),
        Track {
            id: id.clone(),
            kind: TrackKind::Video,
            name: Some("Particles".into()),
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: Vec::new(),
        },
    );
    Ok(id)
}

/// Adds a particle emitter.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticlesAdd {
    /// A built-in preset to start from: `sparks`, `embers`, `dust`, `snow`, `confetti`,
    /// `bokeh`, `light-streaks`, `burst`, `spark-gather` or `sparkle` (`capabilities` lists
    /// them under library.particles). A gentle fountain of soft white dots when omitted.
    #[serde(default)]
    pub preset: Option<String>,
    /// Changes to the emitter as a JSON merge patch over its fields, for example
    /// `{"rate": 80, "colors": ["#FFD27A", "#FF6A00"], "sprite": {"kind": "star"}}`
    /// (`capabilities` lists the fields under particles).
    #[serde(default)]
    pub set: Option<Value>,
    /// Moments when many particles are born at once, from the clip's start, replacing the
    /// preset's: `[{"at": "0.5s", "count": 200}]`.
    #[serde(default)]
    pub bursts: Option<Vec<BurstInput>>,
    /// How long particles were already being born before the clip starts, so it opens
    /// full, up to 30 seconds: `"3s"`.
    #[serde(default)]
    pub prewarm: Option<TimeInput>,
    /// Chooses the random choices; another seed gives other particles that look alike.
    #[serde(default)]
    pub seed: Option<u32>,
    /// Target video track; when omitted, a track of particle clips above every video track
    /// (reused when free, otherwise made).
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Timeline position; 0 when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// Length; 5 seconds when omitted.
    #[serde(default)]
    pub duration: Option<TimeInput>,
    /// Placement of the whole picture; it covers the canvas when omitted.
    #[serde(default)]
    pub transform: Option<Transform>,
}

impl Command for ParticlesAdd {
    const NAME: &'static str = "particles.add";
    const SUMMARY: &'static str = "Add a particle emitter: sparks, embers, dust in light, snow, confetti, bokeh, light streaks, a burst, gathering sparks or sparkles from FossCut's presets, or your own (emitter shape and place, rate and bursts, life, speed, direction and spread, gravity, wind, drag, turbulence, spin, size and opacity over life, colours, sprite: dot, spark streak, star, ring, square or image). It covers the canvas on a track above the footage; animate particles.rate, particles.emitter.position and more with keyframe.set.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let mut content = match &self.preset {
            Some(name) => particle_preset(name)?.content,
            None => ParticleContent::default(),
        };
        change(
            Self::NAME,
            &mut content,
            self.set.as_ref(),
            self.bursts.as_deref(),
            self.prewarm.as_ref(),
            self.seed,
            rate,
        )?;
        // A new clip starts its emission at its start.
        content.offset = Flicks::ZERO;
        checked(ctx.project, Self::NAME, &content)?;
        let start = match &self.start {
            Some(start) => resolve(start, rate)?,
            None => Flicks::ZERO,
        };
        let duration = match &self.duration {
            Some(duration) => resolve(duration, rate)?,
            None => rate.snap(Flicks::from_seconds(5).unwrap_or(Flicks::ZERO)),
        };
        ensure_valid_range(start, duration)?;
        let track = match &self.track {
            Some(track) => track.clone(),
            None => particle_track(ctx.project, &timeline, (start, start + duration))?,
        };
        let id = ctx.project.allocate_id(IdKind::Clip);
        let doc = timeline_mut(ctx.project, &timeline)?;
        let index = track_index(doc, &track)?;
        let track_doc = &mut doc.tracks[index];
        let content = ClipContent::Particles {
            particles: Box::new(content),
        };
        ensure_unlocked(track_doc)?;
        ensure_kind(track_doc, &content)?;
        ensure_free(track_doc, start, start + duration, None, rate)?;
        let clip = Clip {
            transform: self.transform.unwrap_or_default(),
            ..Clip::new(id, start, duration, content)
        };
        let placement = ClipPlacement::of(&clip, &track, rate);
        insert_sorted(track_doc, clip);
        Ok(placement)
    }
}

/// Changes a particle clip's emitter.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticlesSet {
    /// The particle clip.
    pub clip: Id,
    /// A preset whose emitter replaces the current one before the other changes apply
    /// (the clip keeps its place in its emission).
    #[serde(default)]
    pub preset: Option<String>,
    /// Changes to the emitter as a JSON merge patch over its fields, for example
    /// `{"gravity": 0.5, "sprite": {"kind": "ring"}}`; `null` resets a field. Settings with
    /// keyframes change through keyframe.set instead.
    #[serde(default)]
    pub set: Option<Value>,
    /// Bursts from the clip's start, replacing its bursts: `[{"at": "1s", "count": 300}]`;
    /// `[]` removes them.
    #[serde(default)]
    pub bursts: Option<Vec<BurstInput>>,
    /// Emission before the clip's start, up to 30 seconds: `"2s"`.
    #[serde(default)]
    pub prewarm: Option<TimeInput>,
    /// A new seed.
    #[serde(default)]
    pub seed: Option<u32>,
}

/// A particle clip's emitter after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ParticlesChanged {
    /// The clip.
    pub clip: Id,
    /// Its emitter now; defaults are omitted, times are in flicks.
    pub content: ParticleContent,
}

impl Command for ParticlesSet {
    const NAME: &'static str = "particles.set";
    const SUMMARY: &'static str = "Change a particle clip's emitter: start again from a preset, change fields by a merge patch (rate, life, speed, direction, spread, gravity, wind, drag, turbulence, size, colours, sprite and more), and set bursts, prewarm and seed.";
    type Output = ParticlesChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ParticlesChanged, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let rate = ctx.project.timelines[&location.timeline].format.rate;
        let clip =
            &ctx.project.timelines[&location.timeline].tracks[location.track].clips[location.clip];
        let ClipContent::Particles { particles: current } = &clip.content else {
            return Err(invalid(format!(
                "particles.set: {} is a {} clip, not a particle clip",
                self.clip,
                clip.content.kind_name()
            )));
        };
        let before = current.as_ref().clone();
        let mut content = match &self.preset {
            Some(name) => ParticleContent {
                offset: before.offset,
                ..particle_preset(name)?.content
            },
            None => before.clone(),
        };
        change(
            Self::NAME,
            &mut content,
            self.set.as_ref(),
            self.bursts.as_deref(),
            self.prewarm.as_ref(),
            self.seed,
            rate,
        )?;
        content.offset = before.offset;
        // A setting with keyframes takes its value from them: refuse to change it here.
        for path in clip
            .keyframes
            .keys()
            .filter(|p| p.starts_with("particles."))
        {
            if content.value(path) != before.value(path) {
                return Err(ApiError::new(
                    codes::CLIP_PROPERTY_ANIMATED,
                    format!("particles.set: {path} of {} has keyframes", self.clip),
                )
                .with_hint("Set its value at a time with keyframe.set, or remove its keyframes with keyframe.remove (all: true) first."));
            }
        }
        checked(ctx.project, Self::NAME, &content)?;
        let doc = timeline_mut(ctx.project, &location.timeline)?;
        let track = &mut doc.tracks[location.track];
        ensure_unlocked(track)?;
        track.clips[location.clip].content = ClipContent::Particles {
            particles: Box::new(content.clone()),
        };
        Ok(ParticlesChanged {
            clip: self.clip,
            content,
        })
    }
}

/// What `capabilities` says about particles: the presets in brief or in full, and the
/// emitter's choices, animatable paths and limits.
pub(crate) fn particle_capabilities(brief: bool) -> (Value, Value) {
    let presets: Vec<Value> = particle_presets()
        .into_iter()
        .map(|p| {
            if brief {
                json!({ "name": p.name, "title": p.title, "tags": p.tags })
            } else {
                json!(p)
            }
        })
        .collect();
    let properties: Vec<&str> = fosscut_model::PARTICLE_PROPERTIES
        .iter()
        .map(|p| p.path)
        .collect();
    let mut reference = json!({
        "emitters": ["point", "line", "circle", "rectangle"],
        "aims": ["direction", "outward", "inward", "converge"],
        "sprites": fosscut_model::SpriteKind::NAMES,
        "blends": ["add", "normal"],
        "color_by": ["life", "random"],
        "animatable": properties,
        "units": "Distances, sizes and speeds are in canvas heights (per second); the emitter's position and size are fractions of the canvas like transform.position; directions are degrees clockwise from rightwards (90 down, -90 up). Animated settings are read when each particle is born; particles.rate decides when they are born.",
        "limits": {
            "rate": fosscut_model::MAX_PARTICLE_RATE,
            "life_seconds": fosscut_model::MAX_LIFE,
            "burst": fosscut_model::MAX_BURST,
            "bursts": fosscut_model::MAX_BURSTS,
            "drawn_at_once": fosscut_model::MAX_PARTICLES,
            "prewarm_seconds": fosscut_model::MAX_PREWARM_SECONDS,
        },
    });
    if !brief {
        reference["content_schema"] =
            serde_json::to_value(schemars::schema_for!(ParticleContent)).unwrap_or_default();
    }
    (json!(presets), reference)
}
