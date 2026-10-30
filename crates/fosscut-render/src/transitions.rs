//! The CPU side of transitions (ADR-0011, item 4): which shader case a transition type
//! uses and how its parameters pack into the pass's uniform. The shader is `fs_transition`
//! in `shaders/effects.wgsl`.

use fosscut_model::ParamValue;

use crate::effects::{SLOTS, hex_rgb};
use crate::plan::TransitionLayer;

/// Shader case numbers, in `transition_types()` order.
pub const KINDS: [&str; 10] = [
    "dissolve", "dip", "wipe", "slide", "push", "zoom", "blur", "iris", "clock", "spin",
];

/// The uniform of the transition pass. p[0]: case, progress, softness, direction (0 left,
/// 1 right, 2 up, 3 down); p[1]: the dip color, sRGB-encoded.
pub fn pack(transition: &TransitionLayer) -> [[f32; 4]; SLOTS] {
    let mut p = [[0.0f32; 4]; SLOTS];
    let kind = KINDS
        .iter()
        .position(|k| *k == transition.kind)
        .unwrap_or(0);
    let name = |key: &str| match transition.params.get(key) {
        Some(ParamValue::Name(n)) => Some(n.as_str()),
        _ => None,
    };
    let direction = match name("direction").unwrap_or("left") {
        "right" => 1.0,
        "up" => 2.0,
        "down" => 3.0,
        _ => 0.0,
    };
    let softness = match transition.params.get("softness") {
        Some(ParamValue::Number(v)) => *v as f32,
        _ => 0.1,
    };
    p[0] = [kind as f32, transition.progress as f32, softness, direction];
    let color = hex_rgb(name("color").unwrap_or("#000000"));
    p[1] = [color[0], color[1], color[2], 1.0];
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Layer, SourceRef};
    use fosscut_model::{Id, IdKind};
    use fosscut_time::Flicks;

    #[test]
    fn every_transition_type_has_a_case_and_packs_its_parameters() {
        for spec in fosscut_model::transition_types() {
            assert!(KINDS.contains(&spec.name), "{} renders", spec.name);
        }
        let layer = Layer::new(SourceRef {
            asset: Id::new(IdKind::Asset, 1),
            time: Flicks::ZERO,
        });
        let mut transition = TransitionLayer {
            outgoing: layer.clone(),
            incoming: layer,
            kind: "wipe".into(),
            params: [
                ("direction".to_string(), ParamValue::Name("up".into())),
                ("softness".to_string(), ParamValue::Number(0.5)),
            ]
            .into_iter()
            .collect(),
            progress: 0.25,
        };
        assert_eq!(pack(&transition)[0], [2.0, 0.25, 0.5, 2.0]);
        transition.kind = "dip".into();
        transition.params = [("color".to_string(), ParamValue::Name("#FFFFFF".into()))]
            .into_iter()
            .collect();
        let p = pack(&transition);
        assert_eq!((p[0][0], p[1]), (1.0, [1.0, 1.0, 1.0, 1.0]));
        transition.kind = "unknown".into();
        assert_eq!(pack(&transition)[0][0], 0.0, "unknown types dissolve");
    }
}
