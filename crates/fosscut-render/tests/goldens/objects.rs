//! Golden frames of 3D objects (ADR-0040), a module of `goldens.rs`: a lit shipping box with
//! a printed logo and a soft shadow on a catching floor over a photo of planks, a city of
//! blocks seen by a tilted camera, extruded and bevelled letters, a ring sorted between two
//! cards placed in 3D, a sphere blurred by the depth of field, and a glTF model with its
//! own materials and texture.

use fosscut_model::{Id, Project};
use fosscut_render::objects::{ModelData, fixture_glb};
use fosscut_render::plan::plan_frame;
use fosscut_time::Rate;

use super::objects_support::*;
use super::{SIZE, check_at};

fn golden(name: &str, (project, timeline): (Project, Id)) {
    let Some(mut renderer) = super::support::renderer() else {
        return;
    };
    let plan = plan_frame(&project, &timeline, Rate::FPS_30.frames_to_flicks(10)).unwrap();
    let model = ModelData::from_slice(&fixture_glb(Some(&checker_png()))).unwrap();
    let mut provider = Provider::new().with_house(model);
    check_at(name, &mut renderer, &plan, &mut provider, SIZE);
}

/// The orange box turned towards the light on the upper left, its front printed with a
/// dark ring and dot, its rounded edges catching the light, and its soft shadow falling to
/// the right on the planks.
#[test]
fn objects_shipping_box() {
    golden("objects-shipping-box", shipping_box());
}

/// Blocks of varied heights in rows, lit from the right and above, seen from above and
/// turned: the city recedes, nearer blocks larger, their tops lighter than their sides.
#[test]
fn objects_city() {
    golden("objects-city", city());
}

/// FOSS in bold letters with depth, light fronts, gold metallic sides and chamfered edges,
/// turned and tilted; the counters of O stay open.
#[test]
fn objects_extruded_title() {
    golden("objects-extruded-title", extruded_title());
}

/// A metal ring between a coral card in front and a teal card behind: the coral card
/// covers the ring's left side, the ring covers the teal card's left edge, which shows
/// through the ring's hole.
#[test]
fn objects_between_cards() {
    golden("objects-between-cards", between_cards());
}

/// The coral card in focus and a blue sphere far behind it, blurred with soft edges.
#[test]
fn objects_depth_of_field() {
    golden("objects-depth-of-field", defocused());
}

/// The glTF house: checkered walls from its texture, a red roof whose normals were
/// computed, turned, with its shadow on the planks.
#[test]
fn objects_gltf_house() {
    golden("objects-gltf-house", house());
}
