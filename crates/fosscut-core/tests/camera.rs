//! Cameras and clips placed in 3D (ADR-0037): `camera.add`, `camera.set`, the 3D transform
//! properties through `clip.set` and `keyframe.set`, camera keyframes and 3D animation
//! presets.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::codes;
use serde_json::{Value, json};
use support::*;

fn fails(
    session: &mut fosscut_core::Session<fosscut_model::MemoryStore>,
    command: &str,
    args: Value,
) -> String {
    session
        .execute(request(command, args))
        .expect_err(command)
        .code
}

#[test]
fn cameras_go_on_a_camera_track_and_change_by_merge_patch() {
    let (mut session, _, clip) = session_with_clip();
    let added = run(&mut session, "camera.add", json!({}));
    let report = &added.results[0];
    assert_eq!(report["camera"], json!({}), "the default camera");
    assert_eq!(report["start"]["frames"], 0);
    assert_eq!(report["end"]["frames"], 240, "to the timeline's end");
    let camera = report["clip"].as_str().unwrap().to_string();
    let read = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    let tracks = read["tracks"].as_array().unwrap();
    let last_video = tracks.iter().rfind(|t| t["kind"] == "video").unwrap();
    assert_eq!(last_video["name"], "Camera");
    assert_eq!(last_video["clips"][0]["kind"], "camera");
    assert_eq!(last_video["clips"][0]["camera"], json!({}));
    // A second camera after the first reuses the camera track.
    let second = run(
        &mut session,
        "camera.add",
        json!({ "start": "8s", "duration": "1s", "camera": { "field_of_view": 70 } }),
    );
    assert_eq!(second.results[0]["track"], report["track"]);
    assert_eq!(second.results[0]["camera"]["field_of_view"], 70.0);

    let changed = run(
        &mut session,
        "camera.set",
        json!({ "clip": camera, "set": { "orbit": [25, 10], "depth_of_field": true, "aperture": 0.3 } }),
    );
    assert_eq!(changed.results[0]["camera"]["orbit"], json!([25.0, 10.0]));
    let reset = run(
        &mut session,
        "camera.set",
        json!({ "clip": camera, "set": { "aperture": null } }),
    );
    assert!(
        reset.results[0]["camera"].get("aperture").is_none(),
        "back to the default"
    );
    assert_eq!(
        fails(
            &mut session,
            "camera.set",
            json!({ "clip": camera, "set": { "field_of_view": 0 } })
        ),
        codes::CAMERA_INVALID
    );
    assert_eq!(
        fails(
            &mut session,
            "camera.set",
            json!({ "clip": camera, "set": { "zoom": 2 } })
        ),
        codes::CAMERA_INVALID
    );
    assert_eq!(
        fails(
            &mut session,
            "camera.set",
            json!({ "clip": clip, "set": { "roll": 2 } })
        ),
        codes::CAMERA_NOT_CAMERA
    );
    assert_eq!(
        fails(
            &mut session,
            "camera.add",
            json!({ "camera": { "max_blur": 1 } })
        ),
        codes::CAMERA_INVALID
    );
    // A camera has no picture: effects and picture properties are refused.
    assert_eq!(
        fails(
            &mut session,
            "clip.set",
            json!({ "clip": camera, "set": { "opacity": 0.5 } })
        ),
        codes::INVALID_ARGUMENTS
    );
    assert_eq!(
        fails(
            &mut session,
            "effect.add",
            json!({ "clip": camera, "type": "color.adjust" })
        ),
        codes::MODEL_INVALID
    );
}

#[test]
fn camera_properties_animate_and_refuse_plain_changes_while_keyed() {
    let mut session = new_session();
    let added = run(&mut session, "camera.add", json!({ "duration": "2s" }));
    let camera = added.results[0]["clip"].as_str().unwrap().to_string();
    for (at, value) in [("0s", json!(0.0)), ("2s", json!(0.5))] {
        run(
            &mut session,
            "keyframe.set",
            json!({ "clip": camera, "property": "camera.position_z", "at": at, "value": value, "easing": "ease_in_out" }),
        );
    }
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": camera, "property": "camera.orbit", "at": "1s", "value": [30, 0] }),
    );
    // Without a value the keyframe keeps the current value.
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": camera, "property": "camera.field_of_view", "at": "1s" }),
    );
    let read = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    let clip = &read["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Camera")
        .unwrap()["clips"][0];
    assert_eq!(
        clip["keyframes"]["camera.position_z"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(clip["keyframes"]["camera.field_of_view"][0]["value"], 40.0);
    assert_eq!(
        fails(
            &mut session,
            "camera.set",
            json!({ "clip": camera, "set": { "position_z": 1 } })
        ),
        codes::CLIP_PROPERTY_ANIMATED
    );
    // Fields without keyframes still change, and clip.set reaches camera properties.
    run(
        &mut session,
        "camera.set",
        json!({ "clip": camera, "set": { "roll": 5 } }),
    );
    run(
        &mut session,
        "clip.set",
        json!({ "clip": camera, "set": { "camera.depth_of_field": true, "camera.point_of_interest": [0.1, 0] } }),
    );
    assert_eq!(
        fails(
            &mut session,
            "clip.set",
            json!({ "clip": camera, "set": { "camera.aperture": 3 } })
        ),
        codes::INVALID_ARGUMENTS
    );
    // Removing every keyframe keeps the value at the given time as the plain value.
    let removed = run(
        &mut session,
        "keyframe.remove",
        json!({ "clip": camera, "property": "camera.position_z", "all": true, "at": "2s" }),
    );
    assert!(removed.results[0].is_object());
    let read = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    let clip = &read["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Camera")
        .unwrap()["clips"][0];
    assert_eq!(clip["camera"]["position_z"], 0.5);
    assert_eq!(clip["camera"]["roll"], 5.0);
    assert_eq!(clip["camera"]["depth_of_field"], true);
    // A camera property on a video clip is refused.
    let (mut session, _, video) = session_with_clip();
    assert_eq!(
        fails(
            &mut session,
            "clip.set",
            json!({ "clip": video, "set": { "camera.roll": 1 } })
        ),
        codes::INVALID_ARGUMENTS
    );
}

#[test]
fn clips_take_3d_placement_by_path_and_by_keyframes() {
    let (mut session, _, clip) = session_with_clip();
    let set = run(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": {
            "transform.three_d": true,
            "transform.rotation_y": -30,
            "transform.rotation_x": 15,
            "transform.position_z": 0.4,
            "transform.anchor": [-0.5, 0]
        } }),
    );
    let transform = &set.results[0]["transform"];
    assert_eq!(transform["three_d"], true);
    assert_eq!(transform["rotation_y"], -30.0);
    assert_eq!(transform["anchor"], json!([-0.5, 0.0]));
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "transform.rotation_y", "at": "1s", "value": 0 }),
    );
    assert_eq!(
        fails(
            &mut session,
            "clip.set",
            json!({ "clip": clip, "set": { "transform.rotation_y": 5 } })
        ),
        codes::CLIP_PROPERTY_ANIMATED
    );
    assert_eq!(
        fails(
            &mut session,
            "clip.set",
            json!({ "clip": clip, "set": { "transform.three_d": 1 } })
        ),
        codes::INVALID_ARGUMENTS
    );
    assert_eq!(
        fails(
            &mut session,
            "keyframe.set",
            json!({ "clip": clip, "property": "transform.three_d", "at": "1s", "value": 1 })
        ),
        codes::INVALID_ARGUMENTS
    );
    // 3D animation presets sit beside the 2D ones.
    let animated = run(
        &mut session,
        "animation.set",
        json!({ "clip": clip, "in": { "preset": "flip_in" }, "out": { "preset": "fly_out_3d" }, "combo": { "preset": "turntable" } }),
    );
    assert_eq!(animated.results[0]["animations"]["in"]["preset"], "flip_in");
    assert_eq!(
        fails(
            &mut session,
            "animation.set",
            json!({ "clip": clip, "in": { "preset": "flip_out" } })
        ),
        codes::INVALID_ARGUMENTS
    );
    // capabilities lists the properties and the commands.
    let capabilities = session.query("capabilities", json!({})).unwrap();
    let text = capabilities.to_string();
    for name in [
        "camera.add",
        "camera.set",
        "transform.rotation_y",
        "camera.orbit",
        "flip_in",
    ] {
        assert!(text.contains(name), "{name} is listed");
    }
}
