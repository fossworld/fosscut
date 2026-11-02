//! Vector paths (ADR-0036): lines from SVG path data with strokes and trims, trims and
//! stroke widths animated by keyframes and draw animations, clips following paths, and SVG
//! drawings exploded into shape clips.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::codes;
use fosscut_model::{Clip, ClipContent, Id, LineCap, MaskShape, ShapeContent, TimelineRole};
use serde_json::{Value, json};
use support::*;

fn clip_of(session: &fosscut_core::Session<fosscut_model::MemoryStore>, id: &Value) -> Clip {
    let id = Id::parse(id.as_str().unwrap()).unwrap();
    session
        .project()
        .timelines
        .values()
        .find_map(|t| t.find_clip(&id).map(|(_, c)| c.clone()))
        .unwrap()
}

fn shape_of(clip: &Clip) -> ShapeContent {
    match &clip.content {
        ClipContent::Shape(shape) => (**shape).clone(),
        other => panic!("not a shape: {other:?}"),
    }
}

fn fails(
    session: &mut fosscut_core::Session<fosscut_model::MemoryStore>,
    command: &str,
    args: Value,
) -> fosscut_core::ApiError {
    session.execute(request(command, args)).unwrap_err()
}

#[test]
fn path_data_draws_stroked_lines_where_it_says() {
    let mut session = new_session();
    let added = run(
        &mut session,
        "shape.add",
        json!({
            "d": "M 192 540 C 600 108 1300 972 1728 540",
            "stroke": { "color": "#FFCC00", "width": 0.02, "cap": "round" },
            "trim_end": 0.0
        }),
    );
    let clip = clip_of(&session, &added.results[0]["clip"]);
    let shape = shape_of(&clip);
    assert_eq!(shape.shape, MaskShape::Path);
    assert!(!shape.closed && shape.fill.is_none(), "a line without fill");
    assert_eq!(shape.points.len(), 2);
    assert!(shape.points[0].out_handle.is_some() && shape.points[1].in_handle.is_some());
    let stroke = shape.stroke.as_ref().unwrap();
    assert_eq!((stroke.width, stroke.cap), (0.02, LineCap::Round));
    assert_eq!(shape.trim_end, 0.0);
    // Its box spans the curve: 1536 of 1920 pixels wide, centred across the canvas.
    assert!((shape.size[0] - 0.8).abs() < 1e-9, "{:?}", shape.size);
    assert!(clip.transform.position[0].abs() < 1e-9);
    assert!(clip.transform.position[1].abs() < 0.05);

    // A closed path keeps its fill; null removes it and a merge patch adds a stroke.
    let ring = run(
        &mut session,
        "shape.add",
        json!({ "shape": "ellipse", "size": [0.2, 0.2], "fill": null, "stroke": { "width": 0.01 } }),
    );
    assert!(
        shape_of(&clip_of(&session, &ring.results[0]["clip"]))
            .fill
            .is_none()
    );
    let badge = run(&mut session, "shape.add", json!({ "shape": "star" }));
    let id = badge.results[0]["clip"].clone();
    assert!(shape_of(&clip_of(&session, &id)).fill.is_some());
    run(
        &mut session,
        "shape.set",
        json!({ "clip": id, "set": { "fill": null, "stroke": { "join": "round", "dashes": [0.02, 0.01] } } }),
    );
    let changed = shape_of(&clip_of(&session, &id));
    assert!(changed.fill.is_none());
    assert_eq!(changed.stroke.unwrap().dashes, vec![0.02, 0.01]);

    // Broken data and broken strokes are refused before anything changes.
    let error = fails(&mut session, "shape.add", json!({ "d": "M 0 0 Q" }));
    assert_eq!(error.code, codes::SHAPE_INVALID);
    let error = fails(
        &mut session,
        "shape.add",
        json!({ "shape": "ellipse", "stroke": { "width": 2.0 } }),
    );
    assert_eq!(error.code, codes::SHAPE_INVALID);
    assert!(error.message.contains("width"), "{}", error.message);
    let error = fails(
        &mut session,
        "shape.add",
        json!({ "shape": "rectangle", "closed": false }),
    );
    assert_eq!(error.code, codes::SHAPE_INVALID);
}

#[test]
fn trims_and_stroke_widths_animate() {
    let mut session = new_session();
    let added = run(
        &mut session,
        "shape.add",
        json!({ "shape": "ellipse", "fill": null, "stroke": { "width": 0.01 } }),
    );
    let id = added.results[0]["clip"].clone();
    run(
        &mut session,
        "keyframe.set_all",
        json!({ "clip": id, "property": "shape.trim_end", "keys": [
            { "offset": "0s", "value": 0, "easing": "ease_out" }, { "offset": "1s", "value": 1 }
        ] }),
    );
    run(
        &mut session,
        "clip.set",
        json!({ "clip": id, "set": { "shape.trim_offset": 0.25, "shape.stroke_width": 0.03 } }),
    );
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": id, "property": "shape.stroke_width", "offset": "2s" }),
    );
    let clip = clip_of(&session, &id);
    let shape = shape_of(&clip);
    assert_eq!(shape.trim_offset, 0.25);
    assert_eq!(shape.stroke.as_ref().unwrap().width, 0.03);
    let half = clip
        .shape_at(fosscut_time::Flicks::from_seconds_f64(0.5).unwrap())
        .unwrap();
    assert!(
        half.trim_end > 0.5 && half.trim_end < 1.0,
        "eased: {}",
        half.trim_end
    );
    assert_eq!(
        clip.keyframes["shape.stroke_width"][0].value,
        fosscut_model::KeyValue::Number(0.03),
        "a keyframe without a value keeps the current width"
    );
    // A keyed trim is set with keyframes, not clip.set.
    let error = fails(
        &mut session,
        "clip.set",
        json!({ "clip": id, "set": { "shape.trim_end": 0.5 } }),
    );
    assert_eq!(error.code, codes::CLIP_PROPERTY_ANIMATED);
    // Draw animations play on shapes.
    run(
        &mut session,
        "animation.set",
        json!({ "clip": id, "in": { "preset": "draw_on", "duration": "1s" }, "out": { "preset": "draw_off" } }),
    );
    assert_eq!(
        clip_of(&session, &id).animations.entrance.unwrap().preset,
        "draw_on"
    );
    let error = fails(
        &mut session,
        "animation.set",
        json!({ "clip": id, "in": { "preset": "draw_off" } }),
    );
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);

    // Shape properties belong to shapes, and stroke widths to stroked shapes.
    let title = run(&mut session, "text.add", json!({ "text": "Title" }));
    let error = fails(
        &mut session,
        "clip.set",
        json!({ "clip": title.results[0]["clip"], "set": { "shape.trim_end": 0.5 } }),
    );
    assert!(
        error.message.contains("only shape clips"),
        "{}",
        error.message
    );
    let plain = run(&mut session, "shape.add", json!({ "shape": "heart" }));
    let error = fails(
        &mut session,
        "clip.set",
        json!({ "clip": plain.results[0]["clip"], "set": { "shape.stroke_width": 0.02 } }),
    );
    assert_eq!(error.code, codes::SHAPE_INVALID);
}

#[test]
fn clips_follow_paths_until_told_to_stop() {
    let mut session = new_session();
    let route = run(
        &mut session,
        "shape.add",
        json!({ "d": "M 480 540 L 1440 540", "stroke": { "width": 0.005 }, "duration": "4s" }),
    );
    let route = route.results[0]["clip"].clone();
    let dot = run(
        &mut session,
        "shape.add",
        json!({ "shape": "ellipse", "size": [0.02, 0.0356], "duration": "4s" }),
    );
    let dot = dot.results[0]["clip"].clone();
    let followed = run(
        &mut session,
        "clip.follow",
        json!({ "clip": dot, "path": route, "orient": true, "offset": [0, -0.02] }),
    );
    let result = &followed.results[0];
    assert_eq!(result["follow"]["clip"], route);
    // At progress 0 it sits on the route's start, a quarter of the way across.
    assert!(
        (result["position"][0].as_f64().unwrap() + 0.25).abs() < 1e-6,
        "{result}"
    );
    assert!(
        (result["position"][1].as_f64().unwrap() + 0.02).abs() < 1e-6,
        "{result}"
    );
    assert!(result["rotation"].as_f64().unwrap().abs() < 1e-6);
    run(
        &mut session,
        "keyframe.set_all",
        json!({ "clip": dot, "property": "follow.progress", "keys": [
            { "offset": "0s", "value": 0 }, { "offset": "4s", "value": 1 }
        ] }),
    );
    let clip = clip_of(&session, &dot);
    let middle = fosscut_model::follow_placement(
        session.project(),
        &clip,
        fosscut_time::Flicks::from_seconds(2).unwrap(),
    )
    .unwrap();
    assert!(middle.position[0].abs() < 1e-6, "{middle:?}");
    // Changing a follow keeps what is not given; progress is keyed, so it is refused.
    let error = fails(
        &mut session,
        "clip.follow",
        json!({ "clip": dot, "progress": 0.5 }),
    );
    assert_eq!(error.code, codes::CLIP_PROPERTY_ANIMATED);
    run(
        &mut session,
        "clip.follow",
        json!({ "clip": dot, "orient": false }),
    );
    let follow = clip_of(&session, &dot).follow.unwrap();
    assert!(!follow.orient && follow.offset == [0.0, -0.02]);

    // What cannot be followed.
    let title = run(&mut session, "text.add", json!({ "text": "Not a path" }));
    let title = title.results[0]["clip"].clone();
    let spare = run(
        &mut session,
        "shape.add",
        json!({ "shape": "star", "start": "5s" }),
    );
    let spare = spare.results[0]["clip"].clone();
    for (args, needle) in [
        (json!({ "clip": route, "path": title }), "only shape clips"),
        (json!({ "clip": dot, "path": dot }), "itself"),
        (
            json!({ "clip": title, "path": dot }),
            "follows a path itself",
        ),
        (
            json!({ "clip": route, "path": spare }),
            "cannot follow a path itself",
        ),
        (json!({ "clip": title }), "follows nothing yet"),
    ] {
        let error = fails(&mut session, "clip.follow", args.clone());
        assert_eq!(error.code, codes::FOLLOW_INVALID, "{args}");
        assert!(error.message.contains(needle), "{args}: {}", error.message);
    }
    let elsewhere = run(&mut session, "timeline.add", json!({ "name": "Other" }));
    let other = elsewhere.results[0]["timeline"].clone();
    let far = run(
        &mut session,
        "shape.add",
        json!({ "shape": "ellipse", "timeline": other }),
    );
    let error = fails(
        &mut session,
        "clip.follow",
        json!({ "clip": far.results[0]["clip"], "path": route }),
    );
    assert!(
        error.message.contains("not a clip on the timeline"),
        "{}",
        error.message
    );

    // Clearing stops it, and removing the path releases its followers.
    run(
        &mut session,
        "clip.follow",
        json!({ "clip": dot, "clear": true }),
    );
    let clip = clip_of(&session, &dot);
    assert!(clip.follow.is_none() && !clip.keyframes.contains_key("follow.progress"));
    run(
        &mut session,
        "clip.follow",
        json!({ "clip": dot, "path": route }),
    );
    // The route is the only clip of the primary video track, so removing it is guarded.
    let mut remove = request("clip.remove", json!({ "clip": route }));
    remove.allow_destructive = true;
    session.execute(remove).unwrap();
    assert!(clip_of(&session, &dot).follow.is_none());
}

#[test]
fn copies_follow_the_copies_of_their_paths() {
    let mut session = new_session();
    let route = run(
        &mut session,
        "shape.add",
        json!({ "d": "M 100 100 L 1800 900", "stroke": { "width": 0.01 } }),
    );
    let route = route.results[0]["clip"].clone();
    let dot = run(&mut session, "shape.add", json!({ "shape": "ellipse" }));
    let dot = dot.results[0]["clip"].clone();
    run(
        &mut session,
        "clip.follow",
        json!({ "clip": dot, "path": route, "progress": 0.3 }),
    );
    let duplicated = run(
        &mut session,
        "timeline.duplicate",
        json!({ "name": "Copy" }),
    );
    let copy = Id::parse(duplicated.results[0]["timeline"].as_str().unwrap()).unwrap();
    let doc = &session.project().timelines[&copy];
    let clips: Vec<&Clip> = doc.tracks.iter().flat_map(|t| &t.clips).collect();
    let follower = clips.iter().find(|c| c.follow.is_some()).unwrap();
    let path = &follower.follow.as_ref().unwrap().clip;
    assert!(
        doc.find_clip(path).is_some(),
        "the copy follows the copied path"
    );
    assert_ne!(path.as_str(), route.as_str().unwrap());

    // Pasting both keeps them together; pasting the follower alone elsewhere lets go.
    let copied = session
        .query("clips.copy", json!({ "clips": [route, dot] }))
        .unwrap();
    let pasted = run(
        &mut session,
        "clips.paste",
        json!({ "clips": copied["clips"], "at": "5s" }),
    );
    let new_route = pasted.results[0]["clips"][0]["clip"].clone();
    let new_dot = pasted.results[0]["clips"][1]["clip"].clone();
    assert_eq!(
        clip_of(&session, &new_dot).follow.unwrap().clip.as_str(),
        new_route.as_str().unwrap()
    );
    let alone = session
        .query("clips.copy", json!({ "clips": [dot] }))
        .unwrap();
    let pasted = run(
        &mut session,
        "clips.paste",
        json!({ "clips": alone["clips"], "at": "10s", "timeline": copy }),
    );
    assert!(
        clip_of(&session, &pasted.results[0]["clips"][0]["clip"])
            .follow
            .is_none()
    );
}

#[test]
fn drawings_explode_into_a_compound_of_shapes() {
    let mut session = new_session();
    let added = run(&mut session, "media.add", vector_args());
    let asset = added.results[0]["asset"].clone();
    // A drawing shows as an image too.
    let image = run(
        &mut session,
        "clip.add",
        json!({ "asset": asset, "duration": "1s" }),
    );
    assert!(matches!(
        clip_of(&session, &image.results[0]["clip"]).content,
        ClipContent::Image { .. }
    ));
    let before = session.project().clone();
    let exploded = run(
        &mut session,
        "vector.explode",
        json!({ "asset": asset, "start": "1s", "parts": [
            { "name": "arrow", "position": [-0.25, 0], "shape": {
                "shape": "path", "size": [0.5, 0.5], "closed": false, "fill": null,
                "points": [{ "at": [-1, 1] }, { "at": [1, -1] }],
                "stroke": { "color": "#FF6600", "width": 0.04 } } },
            { "opacity": 0.5, "position": [0.25, 0], "shape": {
                "shape": "ellipse", "size": [0.5, 1], "fill": { "color": "#0066FF" } } }
        ] }),
    );
    let result = &exploded.results[0];
    let nested = Id::parse(result["timeline"].as_str().unwrap()).unwrap();
    let project = session.project();
    let entry = project
        .manifest
        .timelines
        .iter()
        .find(|t| t.id == nested)
        .unwrap();
    assert_eq!(
        (entry.role, entry.name.as_str()),
        (TimelineRole::Compound, "logo.svg")
    );
    let doc = &project.timelines[&nested];
    // 200 x 100 fitted into 1920 x 1080.
    assert_eq!((doc.format.width, doc.format.height), (1920, 960));
    assert_eq!(doc.tracks.len(), 2);
    assert_eq!(doc.tracks[0].name.as_deref(), Some("arrow"));
    assert_eq!(doc.tracks[1].name.as_deref(), Some("path 2"));
    let ellipse = &doc.tracks[1].clips[0];
    assert_eq!(ellipse.opacity, 0.5);
    assert_eq!(ellipse.transform.position, [0.25, 0.0]);
    assert_eq!(result["parts"][0]["name"], "arrow");
    let compound = clip_of(&session, &result["compound"]["clip"]);
    assert_eq!(
        compound.content,
        ClipContent::Timeline {
            timeline: nested.clone(),
            source_in: fosscut_time::Flicks::ZERO
        }
    );
    assert_eq!(result["compound"]["start"]["frames"], 30);

    // Undo takes all of it away.
    run(&mut session, "history.undo", json!({}));
    assert_eq!(session.project().timelines, before.timelines);
    assert_eq!(
        session.project().manifest.timelines,
        before.manifest.timelines
    );

    // Only drawings, and only with their paths.
    let error = fails(&mut session, "vector.explode", json!({ "asset": asset }));
    assert_eq!(error.code, codes::VECTOR_INVALID);
    assert!(error.hint.is_some());
    let video = run(&mut session, "media.add", test_pattern_args());
    let error = fails(
        &mut session,
        "vector.explode",
        json!({ "asset": video.results[0]["asset"], "parts": [] }),
    );
    assert_eq!(error.code, codes::VECTOR_INVALID);
    let error = fails(
        &mut session,
        "vector.explode",
        json!({ "asset": asset, "parts": [{ "shape": { "shape": "path", "size": [0.1, 0.1] } }] }),
    );
    assert_eq!(error.code, codes::VECTOR_INVALID);
    assert!(error.message.contains("three points"), "{}", error.message);
}
