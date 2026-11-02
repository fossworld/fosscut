//! `capabilities` lists the Three.js scene templates with their parameters (ADR-0042):
//! names in the usual view, the typed schema in full detail, none in an outline, and the
//! new error code.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use serde_json::json;
use support::*;

#[test]
fn capabilities_list_scene_templates_with_their_params() {
    let session = new_session();
    let usual = session.query("capabilities", json!({})).unwrap();
    let compositions = usual["library"]["compositions"].as_array().unwrap();
    let route = compositions
        .iter()
        .find(|c| c["name"] == "route-map-3d")
        .unwrap();
    let names = route["params"].as_array().unwrap();
    for name in [
        "route",
        "seed",
        "route_color",
        "bloom",
        "motion_blur",
        "transparent",
    ] {
        assert!(names.contains(&json!(name)), "{name}");
    }
    let shop = compositions
        .iter()
        .find(|c| c["name"] == "shop-app")
        .unwrap();
    assert!(
        shop.get("params").is_none(),
        "compositions without params list none"
    );
    let outline = session
        .query("capabilities", json!({ "detail": "outline" }))
        .unwrap();
    assert!(
        outline["library"]["compositions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c.get("params").is_none())
    );

    let full = session
        .query("capabilities", json!({ "detail": "full" }))
        .unwrap();
    let phone = full["library"]["compositions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "phone-3d")
        .unwrap()
        .clone();
    let screen = phone["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "screen")
        .unwrap()
        .clone();
    assert_eq!(screen["type"], "slot");
    assert_eq!(screen["accepts"], json!(["composition", "image", "video"]));
    assert_eq!(screen["default"], "shop-app");
    let blur = phone["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "motion_blur")
        .unwrap()
        .clone();
    assert_eq!(blur["type"], "group");
    assert_eq!(blur["fields"][0]["name"], "samples");
    assert_eq!(blur["fields"][0]["integer"], true);
    let codes = full["errors"].as_array().unwrap();
    assert!(
        codes
            .iter()
            .any(|c| c["code"] == "fosscut.library.invalid_params")
    );
}
