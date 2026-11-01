//! Particle emitters (ADR-0038): every preset is valid, particle clips are added and changed
//! through commands with typed errors, their settings animate as `particles.*` properties,
//! and splits continue the same emission.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{ApiError, Session, codes};
use fosscut_model::{ClipContent, MemoryStore, Project};
use serde_json::{Value, json};
use support::*;

fn fails(session: &mut Session<MemoryStore>, command: &str, args: Value) -> ApiError {
    session.execute(request(command, args)).expect_err(command)
}

fn content(session: &Session<MemoryStore>, clip: &Value) -> fosscut_model::ParticleContent {
    let id = clip.as_str().unwrap();
    session
        .project()
        .timelines
        .values()
        .flat_map(|t| t.tracks.iter().flat_map(|tr| &tr.clips))
        .find_map(|c| match &c.content {
            ClipContent::Particles { particles: p } if c.id.as_str() == id => {
                Some(p.as_ref().clone())
            }
            _ => None,
        })
        .unwrap()
}

fn image_args() -> Value {
    json!({
        "name": "flare.png",
        "kind": "image",
        "path": { "absolute": "/fixtures/flare.png" },
        "identity": { "size": 4096, "sample_blake3": "cd".repeat(32) },
        "info": { "video": { "width": 256, "height": 256, "codec": "png" } }
    })
}

#[test]
fn every_preset_is_valid_and_named_once() {
    let project = Project::new(
        fosscut_model::Id::new(fosscut_model::IdKind::Project, 1),
        "Check",
        fosscut_model::TimelineFormat {
            width: 1920,
            height: 1080,
            rate: fosscut_time::Rate::FPS_30,
            sample_rate: 48_000,
        },
    );
    let presets = fosscut_core::particle_presets();
    let mut names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), presets.len(), "unique names");
    for wanted in [
        "sparks",
        "embers",
        "dust",
        "snow",
        "confetti",
        "bokeh",
        "light-streaks",
        "burst",
    ] {
        assert!(names.contains(&wanted), "{wanted}");
    }
    for preset in &presets {
        let issues = fosscut_model::particle_issues(&project, &preset.content);
        assert!(issues.is_empty(), "{}: {issues:?}", preset.name);
        assert!(!preset.description.is_empty() && !preset.tags.is_empty());
        assert_eq!(preset.content.offset, fosscut_time::Flicks::ZERO);
    }
    assert_eq!(
        fosscut_core::particle_preset("nothing").unwrap_err().code,
        codes::LIBRARY_UNKNOWN_ITEM
    );
}

#[test]
fn particles_go_on_a_track_above_the_footage_and_read_back() {
    let (mut session, _, _) = session_with_clip();
    let added = run(
        &mut session,
        "particles.add",
        json!({ "preset": "embers", "start": "1s", "duration": "3s" }),
    );
    let placed = &added.results[0];
    assert_eq!(placed["start"]["seconds"], "1");
    assert_eq!(placed["duration"]["seconds"], "3");
    let embers = content(&session, &placed["clip"]);
    assert_eq!(
        embers,
        fosscut_core::particle_preset("embers").unwrap().content
    );
    let doc = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    let tracks = doc["tracks"].as_array().unwrap();
    let top = tracks.iter().rfind(|t| t["kind"] == "video").unwrap();
    assert_eq!(top["id"], placed["track"], "above the footage");
    let clip = &top["clips"][0];
    assert_eq!(clip["kind"], "particles");
    assert_eq!(clip["sprite"], "dot");
    assert_eq!(clip["content"]["rate"], 24.0);

    // A free particle track is reused; an overlapping clip gets a track of its own.
    let later = run(
        &mut session,
        "particles.add",
        json!({ "preset": "snow", "start": "5s" }),
    );
    assert_eq!(later.results[0]["track"], placed["track"]);
    let over = run(&mut session, "particles.add", json!({ "start": "2s" }));
    assert_ne!(over.results[0]["track"], placed["track"]);
    let defaults = content(&session, &over.results[0]["clip"]);
    assert_eq!(defaults, fosscut_model::ParticleContent::default());

    // The capabilities list the presets and the emitter's reference.
    let caps = session
        .query("capabilities", json!({ "detail": "full" }))
        .unwrap();
    let presets = caps["library"]["particles"].as_array().unwrap();
    assert!(presets.iter().any(|p| p["name"] == "confetti"));
    assert!(caps["particles"]["content_schema"].is_object());
    assert_eq!(caps["particles"]["sprites"][1], "spark");
    let properties = caps["properties"].as_array().unwrap();
    let rate = properties
        .iter()
        .find(|p| p["path"] == "particles.rate")
        .unwrap();
    assert_eq!(rate["applies"], "particles");
    assert_eq!(rate["animatable"], true);
}

#[test]
fn adding_takes_changes_bursts_prewarm_and_a_seed() {
    let mut session = new_session();
    let added = run(
        &mut session,
        "particles.add",
        json!({
            "preset": "sparks",
            "set": { "rate": 90, "colors": ["#00AAFF"], "sprite": { "kind": "star", "points": 5 } },
            "bursts": [{ "at": "0.5s", "count": 200 }, { "at": { "frames": 3 }, "count": 10 }],
            "prewarm": "2s",
            "seed": 7,
            "start": "1s"
        }),
    );
    let made = content(&session, &added.results[0]["clip"]);
    assert_eq!(made.rate, 90.0);
    assert_eq!(made.colors[0].as_str(), "#00AAFF");
    assert_eq!(made.sprite.kind, fosscut_model::SpriteKind::Star);
    assert_eq!(made.sprite.points, 5);
    assert_eq!(
        made.sprite.stretch, 0.04,
        "the preset's other sprite fields stay"
    );
    assert_eq!(made.seed, 7);
    assert_eq!(made.prewarm.as_seconds_f64(), 2.0);
    let at: Vec<f64> = made.bursts.iter().map(|b| b.at.as_seconds_f64()).collect();
    assert_eq!(at, vec![0.1, 0.5], "sorted, from the clip's start");
    assert_eq!(made.bursts[1].count, 200);
}

#[test]
fn setting_changes_the_emitter_by_a_merge_patch() {
    let mut session = new_session();
    let added = run(&mut session, "particles.add", json!({ "preset": "dust" }));
    let clip = added.results[0]["clip"].clone();
    let changed = run(
        &mut session,
        "particles.set",
        json!({ "clip": clip, "set": { "gravity": 0.5, "sprite": { "kind": "ring" }, "opacity": null } }),
    );
    let after = content(&session, &clip);
    assert_eq!(after.gravity, 0.5);
    assert_eq!(after.sprite.kind, fosscut_model::SpriteKind::Ring);
    assert_eq!(after.sprite.softness, 0.7, "kept");
    assert_eq!(after.opacity, 1.0, "null resets to the default");
    assert_eq!(changed.results[0]["content"]["gravity"], 0.5);
    // Back to a preset, then new bursts, prewarm and seed.
    run(
        &mut session,
        "particles.set",
        json!({ "clip": clip, "preset": "burst", "bursts": [], "prewarm": "1s", "seed": 3 }),
    );
    let burst = content(&session, &clip);
    assert_eq!(burst.rate, 0.0);
    assert!(burst.bursts.is_empty());
    assert_eq!(burst.seed, 3);
    assert_eq!(burst.prewarm.as_seconds_f64(), 1.0);
}

#[test]
fn bad_emitters_are_refused_with_typed_errors() {
    let (mut session, asset, video) = session_with_clip();
    let added = run(&mut session, "particles.add", json!({}));
    let clip = added.results[0]["clip"].clone();
    let invalid = [
        ("particles.add", json!({ "set": { "rate": 5000 } })),
        ("particles.add", json!({ "set": { "bogus": 1 } })),
        ("particles.add", json!({ "set": { "colors": [] } })),
        ("particles.add", json!({ "set": [1, 2] })),
        (
            "particles.add",
            json!({ "bursts": [{ "at": "0s", "count": 0 }] }),
        ),
        ("particles.add", json!({ "prewarm": "40s" })),
        (
            "particles.add",
            json!({ "set": { "sprite": { "kind": "image" } } }),
        ),
        (
            "particles.add",
            json!({ "set": { "sprite": { "kind": "image", "asset": asset } } }),
        ),
        (
            "particles.set",
            json!({ "clip": video, "set": { "rate": 1 } }),
        ),
        (
            "particles.set",
            json!({ "clip": clip, "set": { "life": 0 } }),
        ),
    ];
    for (command, args) in invalid {
        let error = fails(&mut session, command, args.clone());
        assert_eq!(error.code, codes::PARTICLES_INVALID, "{command} {args}");
    }
    let missing = fails(
        &mut session,
        "particles.add",
        json!({ "set": { "sprite": { "kind": "image", "asset": "asset_00ff" } } }),
    );
    assert_eq!(missing.code, codes::ASSET_NOT_FOUND);
    let unknown = fails(&mut session, "particles.add", json!({ "preset": "fog" }));
    assert_eq!(unknown.code, codes::LIBRARY_UNKNOWN_ITEM);
    assert!(unknown.hint.unwrap().contains("sparks"));
}

#[test]
fn image_sprites_use_an_image_that_stays_in_the_bin() {
    let mut session = new_session();
    let image = run(&mut session, "media.add", image_args());
    let asset = image.results[0]["asset"].clone();
    let added = run(
        &mut session,
        "particles.add",
        json!({ "preset": "bokeh", "set": { "sprite": { "kind": "image", "asset": asset } } }),
    );
    let clip = added.results[0]["clip"].clone();
    let removed = fails(&mut session, "media.remove", json!({ "asset": asset }));
    assert_eq!(removed.code, codes::ASSET_IN_USE);
    // Replacing the image moves the sprite to the new one.
    let mut other = image_args();
    other["identity"]["size"] = json!(8192);
    other["name"] = json!("glint.png");
    let other = run(&mut session, "media.add", other).results[0]["asset"].clone();
    let replaced = run(
        &mut session,
        "media.replace",
        json!({ "asset": asset, "with": other }),
    );
    assert_eq!(replaced.results[0]["clips"], json!([clip]));
    assert_eq!(
        content(&session, &clip).sprite.asset.unwrap().as_str(),
        other.as_str().unwrap()
    );
}

#[test]
fn particle_settings_animate_as_properties() {
    let (mut session, _, video) = session_with_clip();
    let added = run(&mut session, "particles.add", json!({ "preset": "sparks" }));
    let clip = added.results[0]["clip"].clone();
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "particles.rate", "offset": "2s" }),
    );
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "particles.rate", "offset": "0s", "value": 0 }),
    );
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "particles.emitter.position", "offset": "1s", "value": [0.3, -0.2] }),
    );
    let doc = session.project().clone();
    let stored = doc
        .timelines
        .values()
        .flat_map(|t| t.tracks.iter().flat_map(|tr| &tr.clips))
        .find(|c| c.id.as_str() == clip.as_str().unwrap())
        .unwrap();
    let rate = &stored.keyframes["particles.rate"];
    assert_eq!(rate.len(), 2);
    assert_eq!(
        rate[1].value,
        fosscut_model::KeyValue::Number(160.0),
        "without a value, the current one"
    );
    assert!(fosscut_model::validate(&doc).is_empty());
    // clip.set sets plain values; an animated one is refused, here and in particles.set.
    run(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "particles.gravity": 3.5, "particles.direction": -60 } }),
    );
    assert_eq!(content(&session, &clip).gravity, 3.5);
    let animated = fails(
        &mut session,
        "particles.set",
        json!({ "clip": clip, "set": { "rate": 10 } }),
    );
    assert_eq!(animated.code, codes::CLIP_PROPERTY_ANIMATED);
    let animated = fails(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "particles.rate": 10 } }),
    );
    assert_eq!(animated.code, codes::CLIP_PROPERTY_ANIMATED);
    // Other settings still change.
    run(
        &mut session,
        "particles.set",
        json!({ "clip": clip, "set": { "drag": 1.5 } }),
    );
    // Out of range, or on a clip without particles.
    let range = fails(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "particles.opacity", "offset": "1s", "value": 2 }),
    );
    assert_eq!(range.code, codes::INVALID_ARGUMENTS);
    let video = fails(
        &mut session,
        "keyframe.set",
        json!({ "clip": video, "property": "particles.rate", "offset": "1s", "value": 2 }),
    );
    assert_eq!(video.code, codes::INVALID_ARGUMENTS);
    assert!(video.message.contains("particle"), "{}", video.message);
}

#[test]
fn split_parts_continue_the_same_emission() {
    let mut session = new_session();
    let added = run(
        &mut session,
        "particles.add",
        json!({ "preset": "snow", "duration": "4s", "bursts": [{ "at": "3s", "count": 50 }] }),
    );
    let clip = added.results[0]["clip"].clone();
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "particles.rate", "offset": "3s", "value": 10 }),
    );
    let split = run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "1.5s" }),
    );
    let right = split.results[0]["right"]["clip"].clone();
    let (left, right_content) = (content(&session, &clip), content(&session, &right));
    assert_eq!(left.offset.as_seconds_f64(), 0.0);
    assert_eq!(right_content.offset.as_seconds_f64(), 1.5);
    assert_eq!(
        right_content.bursts, left.bursts,
        "bursts keep emission time"
    );
    // Bursts given to the right part count from its own start.
    run(
        &mut session,
        "particles.set",
        json!({ "clip": right, "bursts": [{ "at": "0.5s", "count": 20 }] }),
    );
    assert_eq!(content(&session, &right).bursts[0].at.as_seconds_f64(), 2.0);
    // Trimming the start moves the offset too.
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": right, "start": "2s" }),
    );
    assert_eq!(content(&session, &right).offset.as_seconds_f64(), 2.0);
    assert!(fosscut_model::validate(session.project()).is_empty());
}
