//! 3D objects (ADR-0040): every preset is valid, objects are added and changed through
//! commands with typed errors, from presets, model assets, text and shape outlines, their
//! settings animate as `model.*` properties, and their assets count as used.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{ApiError, Session, codes};
use fosscut_model::{ClipContent, MemoryStore, ModelContent, ModelObject, Project};
use serde_json::{Value, json};
use support::*;

fn fails(session: &mut Session<MemoryStore>, command: &str, args: Value) -> ApiError {
    session.execute(request(command, args)).expect_err(command)
}

fn content(session: &Session<MemoryStore>, clip: &Value) -> ModelContent {
    let id = clip.as_str().unwrap();
    session
        .project()
        .timelines
        .values()
        .flat_map(|t| t.tracks.iter().flat_map(|tr| &tr.clips))
        .find_map(|c| match &c.content {
            ClipContent::Model { model } if c.id.as_str() == id => Some(model.as_ref().clone()),
            _ => None,
        })
        .unwrap()
}

fn model_args() -> Value {
    json!({
        "name": "house.glb",
        "kind": "model",
        "path": { "relative": "models/abcd/house.glb" },
        "identity": { "size": 2048, "sample_blake3": "ef".repeat(32) },
        "info": { "model": { "meshes": 2, "triangles": 18, "materials": 2, "textures": 1, "size": [1.1, 1.6, 1.1] } }
    })
}

fn image_args() -> Value {
    json!({
        "name": "logo.png",
        "kind": "image",
        "path": { "absolute": "/fixtures/logo.png" },
        "identity": { "size": 4096, "sample_blake3": "cd".repeat(32) },
        "info": { "video": { "width": 256, "height": 256, "codec": "png" } }
    })
}

fn add(session: &mut Session<MemoryStore>, args: Value) -> Value {
    run(session, "model.add", args).results[0].clone()
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
    let presets = fosscut_core::model_presets();
    let mut names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), presets.len(), "unique names");
    for wanted in ["shipping-box", "city-blocks", "podium", "extruded-title"] {
        assert!(names.contains(&wanted), "{wanted}");
    }
    for preset in &presets {
        let issues = fosscut_model::model_issues(&project, &preset.content);
        assert!(issues.is_empty(), "{}: {issues:?}", preset.name);
        assert!(!preset.description.is_empty() && !preset.tags.is_empty());
    }
    assert_eq!(
        fosscut_core::model_preset("nothing").unwrap_err().code,
        codes::LIBRARY_UNKNOWN_ITEM
    );
}

#[test]
fn objects_go_on_a_track_above_the_footage_and_read_back() {
    let (mut session, _, _) = session_with_clip();
    let placed = add(
        &mut session,
        json!({ "preset": "shipping-box", "start": "1s", "duration": "3s" }),
    );
    assert_eq!(placed["start"]["seconds"], "1");
    assert_eq!(placed["duration"]["seconds"], "3");
    let preset = fosscut_core::model_preset("shipping-box").unwrap();
    assert_eq!(content(&session, &placed["clip"]), preset.content);
    let doc = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    let tracks = doc["tracks"].as_array().unwrap();
    let top = tracks.iter().rfind(|t| t["kind"] == "video").unwrap();
    assert_eq!(top["id"], placed["track"], "above the footage");
    let clip = &top["clips"][0];
    assert_eq!(clip["kind"], "model");
    assert_eq!(clip["object"], "box");
    assert_eq!(clip["content"]["floor"]["shadow"], 0.55);
    // The preset's placement comes with it.
    assert_eq!(clip["transform"]["rotation_y"], -28.0);

    // A free object track is reused; an overlapping object gets a track of its own.
    let later = add(&mut session, json!({ "preset": "podium", "start": "5s" }));
    assert_eq!(later["track"], placed["track"]);
    let over = add(&mut session, json!({ "start": "2s" }));
    assert_ne!(over["track"], placed["track"]);
    assert_eq!(content(&session, &over["clip"]), ModelContent::default());

    // The capabilities list the presets, the objects' reference and the properties.
    let caps = session
        .query("capabilities", json!({ "detail": "full" }))
        .unwrap();
    let presets = caps["library"]["models"].as_array().unwrap();
    assert!(presets.iter().any(|p| p["name"] == "city-blocks"));
    assert!(caps["models"]["content_schema"].is_object());
    assert_eq!(caps["models"]["kinds"][1], "box");
    assert_eq!(caps["models"]["faces"][4], "top");
    let properties = caps["properties"].as_array().unwrap();
    let azimuth = properties
        .iter()
        .find(|p| p["path"] == "model.light.azimuth")
        .unwrap();
    assert_eq!(azimuth["applies"], "model");
    assert_eq!(azimuth["animatable"], true);
    let commands = caps["commands"].as_array().unwrap();
    for name in ["model.add", "model.set"] {
        assert!(commands.iter().any(|c| c["name"] == name), "{name}");
    }
}

#[test]
fn objects_come_from_model_assets_text_and_shape_outlines() {
    let mut session = new_session();
    let model = run(&mut session, "media.add", model_args()).results[0]["asset"].clone();
    // A model asset is not an ordinary clip.
    let refused = fails(&mut session, "clip.add", json!({ "asset": model }));
    assert_eq!(refused.code, codes::INVALID_ARGUMENTS);
    assert!(refused.hint.unwrap_or_default().contains("model.add"));
    let house = add(&mut session, json!({ "asset": model }));
    match content(&session, &house["clip"]).object {
        ModelObject::Asset(object) => {
            assert_eq!(object.asset.as_str(), model.as_str().unwrap());
            assert_eq!(object.size, 0.4, "the default size");
        }
        other => panic!("{other:?}"),
    }

    // Text replaces a title's words and keeps its look.
    let title = add(
        &mut session,
        json!({ "preset": "extruded-title", "text": "SALE", "start": "6s" }),
    );
    match content(&session, &title["clip"]).object {
        ModelObject::Text(text) => {
            assert_eq!(text.text, "SALE");
            assert!(text.bold && text.bevel > 0.0);
        }
        other => panic!("{other:?}"),
    }

    // A shape clip's closed outline, as SVG path data as wide as the shape.
    let shape = run(
        &mut session,
        "shape.add",
        json!({ "d": "M 760 340 L 1160 340 L 960 740 Z" }),
    );
    let star = add(
        &mut session,
        json!({ "shape_clip": shape.results[0]["clip"], "start": "12s", "set": { "object": { "depth": 0.1 } } }),
    );
    match content(&session, &star["clip"]).object {
        ModelObject::Path(path) => {
            assert!(path.path.starts_with('M'), "{}", path.path);
            assert!((path.size - 400.0 / 1920.0).abs() < 1e-6, "{}", path.size);
            assert_eq!(path.depth, 0.1);
        }
        other => panic!("{other:?}"),
    }
    let open = run(
        &mut session,
        "shape.add",
        json!({ "d": "M 100 900 C 600 100 1300 1000 1800 300", "stroke": { "width": 0.01 } }),
    );
    let error = fails(
        &mut session,
        "model.add",
        json!({ "shape_clip": open.results[0]["clip"] }),
    );
    assert_eq!(error.code, codes::OBJECT_INVALID);
    assert!(error.message.contains("open"), "{}", error.message);
}

#[test]
fn bad_objects_are_refused_with_typed_errors() {
    let mut session = new_session();
    let image = run(&mut session, "media.add", image_args()).results[0]["asset"].clone();
    let cases = [
        (
            json!({ "set": { "object": { "kind": "box", "bevel": 1.0 } } }),
            codes::OBJECT_INVALID,
        ),
        (
            json!({ "set": { "light": { "elevation": 100 } } }),
            codes::OBJECT_INVALID,
        ),
        (
            json!({ "set": { "object": { "kind": "sphere" }, "faces": { "top": {} } } }),
            codes::OBJECT_INVALID,
        ),
        (
            json!({ "set": { "object": { "kind": "text", "text": "  " } } }),
            codes::OBJECT_INVALID,
        ),
        (json!({ "set": { "unknown": 1 } }), codes::OBJECT_INVALID),
        (json!({ "set": [1, 2] }), codes::OBJECT_INVALID),
        (json!({ "asset": "asset_0099" }), codes::ASSET_NOT_FOUND),
        (json!({ "asset": image }), codes::OBJECT_INVALID),
        (
            json!({ "set": { "material": { "texture": "asset_0042" } } }),
            codes::ASSET_NOT_FOUND,
        ),
        (json!({ "preset": "nothing" }), codes::LIBRARY_UNKNOWN_ITEM),
    ];
    let revision = session.project().manifest.revision;
    for (args, code) in cases {
        let error = fails(&mut session, "model.add", args.clone());
        assert_eq!(error.code, code, "{args}: {}", error.message);
    }
    assert_eq!(
        session.project().manifest.revision,
        revision,
        "nothing changed"
    );
    // A texture must be an image.
    let printed = add(
        &mut session,
        json!({ "set": { "faces": { "front": { "texture": image, "texture_size": 0.5 } } } }),
    );
    assert_eq!(
        content(&session, &printed["clip"]).texture_assets()[0].as_str(),
        image.as_str().unwrap()
    );
}

#[test]
fn setting_changes_the_object_by_a_merge_patch() {
    let (mut session, _, video) = session_with_clip();
    let added = add(&mut session, json!({ "preset": "shipping-box" }));
    let clip = added["clip"].clone();
    let changed = run(
        &mut session,
        "model.set",
        json!({ "clip": clip, "set": { "light": { "azimuth": 40 }, "floor": null, "material": { "roughness": 0.4 } } }),
    );
    let after = content(&session, &clip);
    assert_eq!(after.light.azimuth, 40.0);
    assert_eq!(after.light.elevation, 55.0, "kept");
    assert!(after.floor.is_none(), "null removes it");
    let material = after.material.clone().unwrap();
    assert_eq!(material.roughness, 0.4);
    assert_eq!(material.color.as_str(), "#C9792F", "kept");
    assert_eq!(changed.results[0]["content"]["light"]["azimuth"], 40.0);
    // Another kind of object starts afresh rather than mixing fields.
    run(
        &mut session,
        "model.set",
        json!({ "clip": clip, "set": { "object": { "kind": "sphere", "size": 0.2 } } }),
    );
    match content(&session, &clip).object {
        ModelObject::Sphere(sphere) => assert_eq!(sphere.size, 0.2),
        other => panic!("{other:?}"),
    }
    // Back to a preset.
    run(
        &mut session,
        "model.set",
        json!({ "clip": clip, "preset": "neon-ring" }),
    );
    assert_eq!(
        content(&session, &clip),
        fosscut_core::model_preset("neon-ring").unwrap().content
    );
    // Only 3D objects are changed this way, and only with something to change.
    let error = fails(
        &mut session,
        "model.set",
        json!({ "clip": video, "set": {} }),
    );
    assert_eq!(error.code, codes::OBJECT_INVALID);
}

#[test]
fn settings_animate_as_model_properties() {
    let mut session = new_session();
    let title = add(&mut session, json!({ "preset": "extruded-title" }));
    let clip = title["clip"].clone();
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "model.depth", "at": "0s", "value": 0.0 }),
    );
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "model.depth", "at": "1s", "value": 0.08 }),
    );
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "model.light.azimuth", "at": "2s", "value": 60 }),
    );
    run(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "model.emissive_strength": 2.5 } }),
    );
    assert_eq!(content(&session, &clip).emissive_strength, 2.5);
    // A keyframed setting is refused by model.set.
    let error = fails(
        &mut session,
        "model.set",
        json!({ "clip": clip, "set": { "object": { "depth": 0.2 } } }),
    );
    assert_eq!(error.code, codes::CLIP_PROPERTY_ANIMATED);
    // A sphere has no depth to animate, and a clip without a floor no shadow.
    let sphere = add(
        &mut session,
        json!({ "preset": "chrome-sphere", "start": "6s", "set": { "floor": null } }),
    );
    for (property, why) in [("model.depth", "depth"), ("model.shadow", "floor")] {
        let error = fails(
            &mut session,
            "keyframe.set",
            json!({ "clip": sphere["clip"], "property": property, "at": "0s", "value": 0.5 }),
        );
        assert_eq!(error.code, codes::INVALID_ARGUMENTS, "{property}");
        assert!(error.message.contains(why), "{}", error.message);
    }
    // Ranges come from the vocabulary.
    let error = fails(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "model.light.elevation", "at": "0s", "value": 120 }),
    );
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);
}

#[test]
fn model_and_texture_assets_count_as_used_and_are_replaced() {
    let mut session = new_session();
    let model = run(&mut session, "media.add", model_args()).results[0]["asset"].clone();
    let image = run(&mut session, "media.add", image_args()).results[0]["asset"].clone();
    let mut other_model = model_args();
    other_model["identity"]["size"] = json!(99);
    other_model["name"] = json!("barn.glb");
    let barn = run(&mut session, "media.add", other_model).results[0]["asset"].clone();
    let mut other_image = image_args();
    other_image["identity"]["size"] = json!(77);
    let badge = run(&mut session, "media.add", other_image).results[0]["asset"].clone();
    let house = add(
        &mut session,
        json!({ "asset": model, "set": { "material": { "texture": image } } }),
    );
    for asset in [&model, &image] {
        let error = fails(&mut session, "media.remove", json!({ "asset": asset }));
        assert_eq!(error.code, codes::ASSET_IN_USE, "{asset}");
    }
    let replaced = run(
        &mut session,
        "media.replace",
        json!({ "asset": model, "with": barn }),
    );
    assert_eq!(replaced.results[0]["clips"][0], house["clip"]);
    run(
        &mut session,
        "media.replace",
        json!({ "asset": image, "with": badge }),
    );
    let after = content(&session, &house["clip"]);
    assert_eq!(
        after.model_asset().unwrap().as_str(),
        barn.as_str().unwrap()
    );
    assert_eq!(after.texture_assets()[0].as_str(), badge.as_str().unwrap());
    // A model only takes the place of a model.
    let error = fails(
        &mut session,
        "media.replace",
        json!({ "asset": barn, "with": badge }),
    );
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);
}
