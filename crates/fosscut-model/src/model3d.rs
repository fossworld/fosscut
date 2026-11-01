//! 3D object clips (ADR-0040): a glTF model or a built-in solid (box, sphere, cylinder,
//! plane, torus, extruded text, an extruded path, a grid of blocks) with a simple
//! physically based material, a sky light and a key light with a soft shadow, and an
//! optional floor that catches the shadow. The object is placed by the clip's 3D transform
//! (ADR-0037), seen through its timeline's camera, ordered among 3D layers by depth and
//! blurred by the depth of field like them.
//!
//! # Units
//!
//! Object space has x to the right, y down and z away from the viewer, like the scene. Sizes,
//! depths, bevels, gaps and floor sizes are fractions of the canvas width, the unit of depth
//! in ADR-0037, so an object is as deep as it is wide on every canvas. A glTF model (y up,
//! z towards the viewer) is turned to match and scaled so its longest side is `size`.
//! Angles are degrees.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::animation::{Applies, KeyValue, Property, PropertyKind};
use crate::timeline::Color;
use crate::{AssetKind, Id, Issue, Project, is_default};

/// The longest text an extruded text object takes, in characters.
pub const MAX_TEXT_CHARS: usize = 200;
/// The most rows or columns of a grid of blocks.
pub const MAX_GRID: u32 = 64;
/// The largest size of an object, a fraction of the canvas width.
pub const MAX_SIZE: f64 = 4.0;

macro_rules! defaults {
    ($($name:ident, $is:ident: $ty:ty = $value:expr;)*) => {
        $(
            fn $name() -> $ty {
                $value
            }

            fn $is(value: &$ty) -> bool {
                *value == $name()
            }
        )*
    };
}

fn color(hex: &str) -> Color {
    Color::parse(hex).unwrap_or_default()
}

defaults! {
    one, is_one: f64 = 1.0;
    yes, is_yes: bool = true;
    grey, is_grey: Color = color("#D0D0D0");
    roughness, is_roughness: f64 = 0.5;
    cube, is_cube: [f64; 3] = [0.25, 0.25, 0.25];
    solid_size, is_solid_size: f64 = 0.25;
    asset_size, is_asset_size: f64 = 0.4;
    column, is_column: [f64; 2] = [0.2, 0.3];
    plane_size, is_plane_size: [f64; 2] = [0.4, 0.4];
    thickness, is_thickness: f64 = 0.08;
    text_size, is_text_size: f64 = 0.12;
    depth, is_depth: f64 = 0.04;
    path_size, is_path_size: f64 = 0.3;
    grid, is_grid: u32 = 8;
    block, is_block: f64 = 0.05;
    gap, is_gap: f64 = 0.015;
    heights, is_heights: [f64; 2] = [0.02, 0.2];
    ambient, is_ambient: f64 = 0.35;
    ground_color, is_ground_color: Color = color("#6B645C");
    azimuth, is_azimuth: f64 = -35.0;
    elevation, is_elevation: f64 = 50.0;
    softness, is_softness: f64 = 0.35;
    floor_color, is_floor_color: Color = color("#9A9A9A");
    shadow, is_shadow: f64 = 0.6;
    floor_size, is_floor_size: f64 = 4.0;
}

/// A 3D object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelContent {
    /// What the object is: a model asset or a built-in solid (a box when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub object: ModelObject,
    /// Its material. A model asset keeps the materials of its file unless this is given;
    /// built-in solids are light grey and half rough without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<Material>,
    /// Materials of single faces of a box or of every block of a grid, or of the front and
    /// back of extruded text and paths (their sides and bevels keep `material`).
    #[serde(default, skip_serializing_if = "is_default")]
    pub faces: Faces,
    /// Multiplies the light every material gives off by itself, 0 to 100 (1 when omitted;
    /// animate `model.emissive_strength`).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub emissive_strength: f64,
    /// The light on the object: a sky light and a key light with a shadow.
    #[serde(default, skip_serializing_if = "is_default")]
    pub light: Light,
    /// A level floor under the object that catches its shadow, or shows as a surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<Floor>,
}

impl Default for ModelContent {
    /// A light grey cube under the default light.
    fn default() -> ModelContent {
        ModelContent {
            object: ModelObject::default(),
            material: None,
            faces: Faces::default(),
            emissive_strength: one(),
            light: Light::default(),
            floor: None,
        }
    }
}

/// What an object is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelObject {
    /// A glTF model asset (`.glb`, or `.gltf` with its files).
    Asset(AssetObject),
    /// A box, with rounded edges when bevelled; its faces take materials of their own.
    Box(BoxObject),
    /// A sphere.
    Sphere(SphereObject),
    /// An upright cylinder with flat caps.
    Cylinder(CylinderObject),
    /// A flat rectangle facing the viewer, like a picture; lay it down with `rotation_x`.
    Plane(PlaneObject),
    /// A ring facing the viewer.
    Torus(TorusObject),
    /// Text extruded into depth, with a bevel.
    Text(TextObject),
    /// An outline from SVG path data (or a shape clip's path) extruded into depth.
    Path(PathObject),
    /// A grid of blocks of varied heights, standing on the ground like a city.
    Blocks(BlocksObject),
}

impl Default for ModelObject {
    fn default() -> ModelObject {
        ModelObject::Box(BoxObject::default())
    }
}

impl ModelObject {
    /// The kind as used in JSON (`asset`, `box`, `sphere`, ...).
    pub fn kind_name(&self) -> &'static str {
        match self {
            ModelObject::Asset(_) => "asset",
            ModelObject::Box(_) => "box",
            ModelObject::Sphere(_) => "sphere",
            ModelObject::Cylinder(_) => "cylinder",
            ModelObject::Plane(_) => "plane",
            ModelObject::Torus(_) => "torus",
            ModelObject::Text(_) => "text",
            ModelObject::Path(_) => "path",
            ModelObject::Blocks(_) => "blocks",
        }
    }

    /// Every kind's name, in order.
    pub const KINDS: &'static [&'static str] = &[
        "asset", "box", "sphere", "cylinder", "plane", "torus", "text", "path", "blocks",
    ];
}

/// A glTF model asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetObject {
    /// The model asset (`asset_…`).
    pub asset: Id,
    /// The length of the model's longest side, 0.4 when omitted; the model is centred on
    /// the middle of its bounds.
    #[serde(default = "asset_size", skip_serializing_if = "is_asset_size")]
    pub size: f64,
}

/// A box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoxObject {
    /// Width, height and depth (`[0.25, 0.25, 0.25]` when omitted).
    #[serde(default = "cube", skip_serializing_if = "is_cube")]
    pub size: [f64; 3],
    /// The radius of its rounded edges, at most half its smallest side.
    #[serde(default, skip_serializing_if = "is_default")]
    pub bevel: f64,
}

impl Default for BoxObject {
    fn default() -> BoxObject {
        BoxObject {
            size: cube(),
            bevel: 0.0,
        }
    }
}

/// A sphere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SphereObject {
    /// Its diameter (0.25 when omitted).
    #[serde(default = "solid_size", skip_serializing_if = "is_solid_size")]
    pub size: f64,
}

/// An upright cylinder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CylinderObject {
    /// Its diameter and height (`[0.2, 0.3]` when omitted).
    #[serde(default = "column", skip_serializing_if = "is_column")]
    pub size: [f64; 2],
}

/// A flat rectangle facing the viewer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaneObject {
    /// Its width and height (`[0.4, 0.4]` when omitted).
    #[serde(default = "plane_size", skip_serializing_if = "is_plane_size")]
    pub size: [f64; 2],
}

/// A ring facing the viewer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TorusObject {
    /// Its outer diameter (0.25 when omitted).
    #[serde(default = "solid_size", skip_serializing_if = "is_solid_size")]
    pub size: f64,
    /// The diameter of its tube, less than half the outer diameter (0.08 when omitted).
    #[serde(default = "thickness", skip_serializing_if = "is_thickness")]
    pub thickness: f64,
}

/// Text extruded into depth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextObject {
    /// The text, 1 to 200 characters; `\n` starts a new line. Lines are centred.
    pub text: String,
    /// The font size (0.12 when omitted).
    #[serde(default = "text_size", skip_serializing_if = "is_text_size")]
    pub size: f64,
    /// The font family: the bundled Inter or a project font asset's family (Inter when
    /// omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Bold letters.
    #[serde(default, skip_serializing_if = "is_default")]
    pub bold: bool,
    /// Italic letters.
    #[serde(default, skip_serializing_if = "is_default")]
    pub italic: bool,
    /// How deep the letters are, 0 to 1 (0.04 when omitted; animate `model.depth`).
    #[serde(default = "depth", skip_serializing_if = "is_depth")]
    pub depth: f64,
    /// How far the letters' front and back edges are chamfered, at most half the depth.
    #[serde(default, skip_serializing_if = "is_default")]
    pub bevel: f64,
}

/// An outline extruded into depth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathObject {
    /// SVG path data (`M 0 0 L 10 0 ...`, y down); nested contours are holes, as with the
    /// even-odd rule.
    pub path: String,
    /// The width of the outline's bounds; its height follows the outline (0.3 when
    /// omitted).
    #[serde(default = "path_size", skip_serializing_if = "is_path_size")]
    pub size: f64,
    /// How deep it is, 0 to 1 (0.04 when omitted; animate `model.depth`).
    #[serde(default = "depth", skip_serializing_if = "is_depth")]
    pub depth: f64,
    /// How far its front and back edges are chamfered, at most half the depth.
    #[serde(default, skip_serializing_if = "is_default")]
    pub bevel: f64,
}

/// A grid of blocks on the ground, centred on the object's position, growing upwards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlocksObject {
    /// Blocks across, 1 to 64 (8 when omitted).
    #[serde(default = "grid", skip_serializing_if = "is_grid")]
    pub columns: u32,
    /// Blocks in depth, 1 to 64 (8 when omitted).
    #[serde(default = "grid", skip_serializing_if = "is_grid")]
    pub rows: u32,
    /// The width and depth of each block (0.05 when omitted).
    #[serde(default = "block", skip_serializing_if = "is_block")]
    pub block: f64,
    /// The street between blocks (0.015 when omitted).
    #[serde(default = "gap", skip_serializing_if = "is_gap")]
    pub gap: f64,
    /// The lowest and highest block, `[0.02, 0.2]` when omitted; heights are spread by the
    /// square of a random number, so tall blocks are fewer.
    #[serde(default = "heights", skip_serializing_if = "is_heights")]
    pub heights: [f64; 2],
    /// The share of lots that hold a block, 0 to 1 (1 when omitted).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub fill: f64,
    /// How far the blocks have grown, 0 (flat) to 1 (full height); 1 when omitted
    /// (animate `model.growth`).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub growth: f64,
    /// Chooses the heights and empty lots; another seed gives another city.
    #[serde(default, skip_serializing_if = "is_default")]
    pub seed: u32,
}

impl Default for BlocksObject {
    fn default() -> BlocksObject {
        BlocksObject {
            columns: grid(),
            rows: grid(),
            block: block(),
            gap: gap(),
            heights: heights(),
            fill: one(),
            growth: one(),
            seed: 0,
        }
    }
}

/// A simple physically based material (after the metallic-roughness model of glTF 2.0).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Material {
    /// Its colour as `#RRGGBB` (`#D0D0D0` when omitted).
    #[serde(default = "grey", skip_serializing_if = "is_grey")]
    pub color: Color,
    /// An image asset (`asset_…`) printed over the colour by its own transparency, so a
    /// logo with a transparent background sits on the colour. On a box face it covers the
    /// face; on other solids it wraps around them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<Id>,
    /// How much of a face the texture covers, centred, 0.05 to 1 (1 when omitted).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub texture_size: f64,
    /// 0 for paint, plastic, paper or wood, 1 for metal.
    #[serde(default, skip_serializing_if = "is_default")]
    pub metallic: f64,
    /// 0 for a mirror finish, 1 for a fully matte one (0.5 when omitted).
    #[serde(default = "roughness", skip_serializing_if = "is_roughness")]
    pub roughness: f64,
    /// The colour of light it gives off by itself, as `#RRGGBB`; none when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissive: Option<Color>,
    /// How strongly it gives off that light, 0 to 100 (1 when omitted); above 1 it
    /// outshines its surroundings and a `light.glow` effect blooms it.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub emissive_strength: f64,
    /// 0 (clear) to 1 (solid).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f64,
}

impl Default for Material {
    fn default() -> Material {
        Material {
            color: grey(),
            texture: None,
            texture_size: one(),
            metallic: 0.0,
            roughness: roughness(),
            emissive: None,
            emissive_strength: one(),
            opacity: one(),
        }
    }
}

/// Materials of single faces: of a box, of each block of a grid, or the front and back of
/// extruded text and paths. Faces are named as the viewer sees them at rest: `front` faces
/// the viewer, `top` faces up.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Faces {
    /// The face towards the viewer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front: Option<Material>,
    /// The face away from the viewer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<Material>,
    /// The face on the left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<Material>,
    /// The face on the right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<Material>,
    /// The face on top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<Material>,
    /// The face underneath.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<Material>,
}

impl Faces {
    /// The names of the faces, in the order of [`Faces::all`].
    pub const NAMES: [&'static str; 6] = ["front", "back", "left", "right", "top", "bottom"];

    /// The faces in the order of [`Faces::NAMES`].
    pub fn all(&self) -> [Option<&Material>; 6] {
        [
            self.front.as_ref(),
            self.back.as_ref(),
            self.left.as_ref(),
            self.right.as_ref(),
            self.top.as_ref(),
            self.bottom.as_ref(),
        ]
    }

    fn all_mut(&mut self) -> [&mut Option<Material>; 6] {
        [
            &mut self.front,
            &mut self.back,
            &mut self.left,
            &mut self.right,
            &mut self.top,
            &mut self.bottom,
        ]
    }
}

/// The light on an object: a sky light from every direction, coloured from above and
/// below, and a key light from one direction that casts a soft shadow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Light {
    /// How strong the sky light is, 0 to 4 (0.35 when omitted; animate
    /// `model.light.ambient`).
    #[serde(default = "ambient", skip_serializing_if = "is_ambient")]
    pub ambient: f64,
    /// The sky light's colour from above (white when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub sky: Color,
    /// The sky light's colour from below, as light bounced off the ground (`#6B645C` when
    /// omitted).
    #[serde(default = "ground_color", skip_serializing_if = "is_ground_color")]
    pub ground: Color,
    /// Where the key light comes from around the vertical axis: 0 from the viewer's side,
    /// 90 from the right, -90 from the left, 180 from behind (-35 when omitted; animate
    /// `model.light.azimuth`).
    #[serde(default = "azimuth", skip_serializing_if = "is_azimuth")]
    pub azimuth: f64,
    /// How high the key light is above the horizon, -90 to 90 (50 when omitted; animate
    /// `model.light.elevation`).
    #[serde(default = "elevation", skip_serializing_if = "is_elevation")]
    pub elevation: f64,
    /// The key light's colour (white when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub color: Color,
    /// How strong the key light is, 0 to 10: a white matte surface facing it shows this
    /// much of its colour (1 when omitted; animate `model.light.intensity`).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub intensity: f64,
    /// Whether the key light casts a shadow, on the object itself and on the floor (true
    /// when omitted).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub shadows: bool,
    /// How soft the shadow's edge is, 0 (sharp) to 1 (very soft); 0.35 when omitted.
    #[serde(default = "softness", skip_serializing_if = "is_softness")]
    pub softness: f64,
}

impl Default for Light {
    fn default() -> Light {
        Light {
            ambient: ambient(),
            sky: Color::default(),
            ground: ground_color(),
            azimuth: azimuth(),
            elevation: elevation(),
            color: Color::default(),
            intensity: one(),
            shadows: yes(),
            softness: softness(),
        }
    }
}

impl Light {
    /// The unit direction towards the key light in scene coordinates (x right, y down, z
    /// away from the viewer).
    pub fn direction(&self) -> [f64; 3] {
        let (a, e) = (self.azimuth.to_radians(), self.elevation.to_radians());
        [a.sin() * e.cos(), -e.sin(), -a.cos() * e.cos()]
    }
}

/// A level floor under an object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Floor {
    /// True (the default) shows only the shadow it catches, so the object can stand on a
    /// photo or video below it; false draws the floor in `color`.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub catcher: bool,
    /// The colour of a drawn floor (`#9A9A9A` when omitted).
    #[serde(default = "floor_color", skip_serializing_if = "is_floor_color")]
    pub color: Color,
    /// How dark the shadow on a catcher is, 0 to 1 (0.6 when omitted; animate
    /// `model.shadow`).
    #[serde(default = "shadow", skip_serializing_if = "is_shadow")]
    pub shadow: f64,
    /// How far below the object's lowest point at rest the floor lies, -1 to 1; negative
    /// sinks the object into it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub gap: f64,
    /// The floor's width and depth, 0.01 to 20 (4 when omitted).
    #[serde(default = "floor_size", skip_serializing_if = "is_floor_size")]
    pub size: f64,
}

impl Default for Floor {
    fn default() -> Floor {
        Floor {
            catcher: yes(),
            color: floor_color(),
            shadow: shadow(),
            gap: 0.0,
            size: floor_size(),
        }
    }
}

/// An animatable setting of 3D objects (ADR-0040).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelProperty {
    /// The property path, such as `model.light.azimuth`.
    pub path: &'static str,
    /// The lowest value allowed.
    pub min: Option<f64>,
    /// The highest value allowed.
    pub max: Option<f64>,
    /// What it means.
    pub meaning: &'static str,
}

/// The animatable settings of 3D objects.
pub const MODEL_PROPERTIES: &[ModelProperty] = &[
    ModelProperty {
        path: "model.emissive_strength",
        min: Some(0.0),
        max: Some(100.0),
        meaning: "multiplies the light every material of the object gives off by itself, 0 to 100",
    },
    ModelProperty {
        path: "model.light.azimuth",
        min: None,
        max: None,
        meaning: "where the key light comes from around the vertical axis in degrees: 0 from the viewer's side, 90 from the right",
    },
    ModelProperty {
        path: "model.light.elevation",
        min: Some(-90.0),
        max: Some(90.0),
        meaning: "how high the key light is above the horizon in degrees",
    },
    ModelProperty {
        path: "model.light.intensity",
        min: Some(0.0),
        max: Some(10.0),
        meaning: "how strong the key light is, 0 to 10",
    },
    ModelProperty {
        path: "model.light.ambient",
        min: Some(0.0),
        max: Some(4.0),
        meaning: "how strong the sky light is, 0 to 4",
    },
    ModelProperty {
        path: "model.shadow",
        min: Some(0.0),
        max: Some(1.0),
        meaning: "how dark the shadow on the floor's catcher is, 0 to 1 (objects with a floor)",
    },
    ModelProperty {
        path: "model.depth",
        min: Some(0.0),
        max: Some(1.0),
        meaning: "how deep extruded text or an extruded path is, a fraction of the canvas width",
    },
    ModelProperty {
        path: "model.growth",
        min: Some(0.0),
        max: Some(1.0),
        meaning: "how far a grid's blocks have grown, 0 (flat) to 1 (full height)",
    },
];

/// The model properties as entries of the closed vocabulary (ADR-0011 item 3).
pub(crate) fn model_properties() -> Vec<Property> {
    MODEL_PROPERTIES
        .iter()
        .map(|p| Property {
            path: p.path,
            kind: PropertyKind::Number,
            applies: Applies::Model,
            animatable: true,
            min: p.min,
            max: p.max,
            choices: &[],
            meaning: p.meaning,
        })
        .collect()
}

impl ModelContent {
    /// The plain value of an animatable model property (`model.light.azimuth`, ...);
    /// `None` when the path is not one or the object has no such setting (`model.depth` of
    /// a sphere, `model.shadow` without a floor).
    pub fn value(&self, path: &str) -> Option<KeyValue> {
        let number = match path {
            "model.emissive_strength" => self.emissive_strength,
            "model.light.azimuth" => self.light.azimuth,
            "model.light.elevation" => self.light.elevation,
            "model.light.intensity" => self.light.intensity,
            "model.light.ambient" => self.light.ambient,
            "model.shadow" => self.floor.as_ref()?.shadow,
            "model.depth" => match &self.object {
                ModelObject::Text(text) => text.depth,
                ModelObject::Path(path) => path.depth,
                _ => return None,
            },
            "model.growth" => match &self.object {
                ModelObject::Blocks(blocks) => blocks.growth,
                _ => return None,
            },
            _ => return None,
        };
        Some(KeyValue::Number(number))
    }

    /// Sets the plain value of an animatable model property; false when `path` is not one,
    /// the object has no such setting, or `value` is not a number.
    pub fn set_value(&mut self, path: &str, value: KeyValue) -> bool {
        let KeyValue::Number(v) = value else {
            return false;
        };
        let slot = match path {
            "model.emissive_strength" => &mut self.emissive_strength,
            "model.light.azimuth" => &mut self.light.azimuth,
            "model.light.elevation" => &mut self.light.elevation,
            "model.light.intensity" => &mut self.light.intensity,
            "model.light.ambient" => &mut self.light.ambient,
            "model.shadow" => match &mut self.floor {
                Some(floor) => &mut floor.shadow,
                None => return false,
            },
            "model.depth" => match &mut self.object {
                ModelObject::Text(text) => &mut text.depth,
                ModelObject::Path(path) => &mut path.depth,
                _ => return false,
            },
            "model.growth" => match &mut self.object {
                ModelObject::Blocks(blocks) => &mut blocks.growth,
                _ => return false,
            },
            _ => return false,
        };
        *slot = v;
        true
    }

    /// The model asset it shows, if any.
    pub fn model_asset(&self) -> Option<&Id> {
        match &self.object {
            ModelObject::Asset(object) => Some(&object.asset),
            _ => None,
        }
    }

    /// The image assets its materials print, in order, without repeats.
    pub fn texture_assets(&self) -> Vec<&Id> {
        let mut assets: Vec<&Id> = Vec::new();
        let materials = std::iter::once(self.material.as_ref()).chain(self.faces.all());
        for texture in materials.flatten().filter_map(|m| m.texture.as_ref()) {
            if !assets.contains(&texture) {
                assets.push(texture);
            }
        }
        assets
    }

    /// True when it uses `asset`: as its model or as a texture.
    pub fn uses(&self, asset: &Id) -> bool {
        self.model_asset() == Some(asset) || self.texture_assets().contains(&asset)
    }

    /// Points every use of `old` (its model or a texture) at `new`.
    pub fn replace_asset(&mut self, old: &Id, new: &Id) {
        if let ModelObject::Asset(object) = &mut self.object
            && &object.asset == old
        {
            object.asset = new.clone();
        }
        let faces = self.faces.all_mut().into_iter();
        for material in std::iter::once(&mut self.material).chain(faces) {
            if let Some(material) = material
                && material.texture.as_ref() == Some(old)
            {
                material.texture = Some(new.clone());
            }
        }
    }

    /// Problems with these settings, as messages naming the field; empty when valid. The
    /// assets are checked against the project by [`model_issues`].
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut need = |ok: bool, message: String| {
            if !ok {
                problems.push(message);
            }
        };
        let within = |v: f64, low: f64, high: f64| v.is_finite() && v >= low && v <= high;
        let sized = |v: f64| within(v, 0.001, MAX_SIZE);
        let extent = "must be from 0.001 to 4 canvas widths";
        match &self.object {
            ModelObject::Asset(o) => need(sized(o.size), format!("object.size {extent}")),
            ModelObject::Box(o) => {
                need(
                    o.size.iter().all(|&v| sized(v)),
                    format!("each of object.size {extent}"),
                );
                let smallest = o.size.iter().copied().fold(f64::INFINITY, f64::min);
                need(
                    within(o.bevel, 0.0, smallest / 2.0),
                    "object.bevel must be from 0 to half the box's smallest side".into(),
                );
            }
            ModelObject::Sphere(o) => need(sized(o.size), format!("object.size {extent}")),
            ModelObject::Cylinder(o) => need(
                o.size.iter().all(|&v| sized(v)),
                format!("each of object.size {extent}"),
            ),
            ModelObject::Plane(o) => need(
                o.size.iter().all(|&v| sized(v)),
                format!("each of object.size {extent}"),
            ),
            ModelObject::Torus(o) => {
                need(sized(o.size), format!("object.size {extent}"));
                need(
                    within(o.thickness, 0.001, o.size / 2.0),
                    "object.thickness must be from 0.001 to half the torus's size".into(),
                );
            }
            ModelObject::Text(o) => {
                let chars = o.text.chars().count();
                need(
                    !o.text.trim().is_empty() && chars <= MAX_TEXT_CHARS,
                    "object.text needs 1 to 200 characters that are not all spaces".into(),
                );
                need(
                    within(o.size, 0.005, 2.0),
                    "object.size must be from 0.005 to 2 canvas widths".into(),
                );
                need(
                    o.font.as_ref().is_none_or(|f| !f.trim().is_empty()),
                    "object.font must name a family".into(),
                );
                extruded(&mut need, o.depth, o.bevel);
            }
            ModelObject::Path(o) => {
                if let Err(e) = crate::parse_path_data(&o.path, 1e-9) {
                    need(false, format!("object.path: {e}"));
                }
                need(sized(o.size), format!("object.size {extent}"));
                extruded(&mut need, o.depth, o.bevel);
            }
            ModelObject::Blocks(o) => {
                need(
                    (1..=MAX_GRID).contains(&o.columns) && (1..=MAX_GRID).contains(&o.rows),
                    "object.columns and object.rows must be from 1 to 64".into(),
                );
                need(
                    within(o.block, 0.001, 1.0),
                    "object.block must be from 0.001 to 1 canvas width".into(),
                );
                need(
                    within(o.gap, 0.0, 1.0),
                    "object.gap must be from 0 to 1 canvas width".into(),
                );
                let [low, high] = o.heights;
                need(
                    within(low, 0.0, MAX_SIZE) && within(high, low, MAX_SIZE),
                    "object.heights must be [lowest, highest] from 0 to 4, lowest first".into(),
                );
                need(
                    within(o.fill, 0.0, 1.0),
                    "object.fill must be from 0 to 1".into(),
                );
                need(
                    within(o.growth, 0.0, 1.0),
                    "object.growth must be from 0 to 1".into(),
                );
            }
        }
        let faced = matches!(self.object, ModelObject::Box(_) | ModelObject::Blocks(_));
        let capped = matches!(self.object, ModelObject::Text(_) | ModelObject::Path(_));
        let sides = Faces {
            front: None,
            back: None,
            ..self.faces.clone()
        };
        need(
            faced || (capped && sides == Faces::default()) || self.faces == Faces::default(),
            "faces are for boxes and blocks, and front and back for extruded text and paths".into(),
        );
        let named = std::iter::once(("material", self.material.as_ref()))
            .chain(Faces::NAMES.iter().copied().zip(self.faces.all()));
        for (name, material) in named {
            let Some(m) = material else { continue };
            let name = if name == "material" {
                "material".to_string()
            } else {
                format!("faces.{name}")
            };
            need(
                within(m.metallic, 0.0, 1.0) && within(m.roughness, 0.0, 1.0),
                format!("{name}: metallic and roughness must be from 0 to 1"),
            );
            need(
                within(m.opacity, 0.0, 1.0),
                format!("{name}: opacity must be from 0 to 1"),
            );
            need(
                within(m.emissive_strength, 0.0, 100.0),
                format!("{name}: emissive_strength must be from 0 to 100"),
            );
            need(
                within(m.texture_size, 0.05, 1.0),
                format!("{name}: texture_size must be from 0.05 to 1"),
            );
        }
        need(
            within(self.emissive_strength, 0.0, 100.0),
            "emissive_strength must be from 0 to 100".into(),
        );
        let l = &self.light;
        need(
            within(l.ambient, 0.0, 4.0),
            "light.ambient must be from 0 to 4".into(),
        );
        need(
            within(l.intensity, 0.0, 10.0),
            "light.intensity must be from 0 to 10".into(),
        );
        need(
            l.azimuth.is_finite() && within(l.elevation, -90.0, 90.0),
            "light.azimuth must be a number of degrees, light.elevation from -90 to 90".into(),
        );
        need(
            within(l.softness, 0.0, 1.0),
            "light.softness must be from 0 to 1".into(),
        );
        if let Some(f) = &self.floor {
            need(
                within(f.shadow, 0.0, 1.0),
                "floor.shadow must be from 0 to 1".into(),
            );
            need(
                within(f.gap, -1.0, 1.0),
                "floor.gap must be from -1 to 1 canvas width".into(),
            );
            need(
                within(f.size, 0.01, 20.0),
                "floor.size must be from 0.01 to 20 canvas widths".into(),
            );
        }
        problems
    }
}

/// Checks an extrusion's depth and bevel.
fn extruded(need: &mut impl FnMut(bool, String), depth: f64, bevel: f64) {
    let within = |v: f64, low: f64, high: f64| v.is_finite() && v >= low && v <= high;
    need(
        within(depth, 0.0, 1.0),
        "object.depth must be from 0 to 1 canvas width".into(),
    );
    need(
        within(bevel, 0.0, depth.max(0.0) / 2.0),
        "object.bevel must be from 0 to half the depth".into(),
    );
}

/// Problems with a 3D object in `project` (ADR-0040), so commands can report them before
/// they change anything: its settings, its model asset and its textures.
pub fn model_issues(project: &Project, content: &ModelContent) -> Vec<Issue> {
    let issue = |message: String| Issue {
        path: "model".into(),
        code: "fosscut.model.object".into(),
        message,
    };
    let mut issues: Vec<Issue> = content.problems().into_iter().map(issue).collect();
    let mut check = |asset: &Id, kind: AssetKind, role: &str| match project.media.get(asset) {
        None => issues.push(Issue {
            code: "fosscut.model.missing_asset".into(),
            ..issue(format!("the {role} asset {asset} does not exist"))
        }),
        Some(found) if found.kind != kind => issues.push(issue(format!(
            "the {role} asset {asset} is a {:?} asset, not {kind:?}",
            found.kind
        ))),
        Some(_) => {}
    };
    if let Some(asset) = content.model_asset() {
        check(asset, AssetKind::Model, "model");
    }
    for texture in content.texture_assets() {
        check(texture, AssetKind::Image, "texture");
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_default_object_is_a_grey_cube_written_as_nothing() {
        let plain = ModelContent::default();
        assert_eq!(serde_json::to_value(&plain).unwrap(), json!({}));
        assert_eq!(
            serde_json::from_value::<ModelContent>(json!({})).unwrap(),
            plain
        );
        assert!(plain.problems().is_empty(), "{:?}", plain.problems());
        assert!(serde_json::from_value::<ModelContent>(json!({"bogus": 1})).is_err());
        assert!(
            serde_json::from_value::<ModelContent>(json!({"object": {"kind": "box", "bogus": 1}}))
                .is_err()
        );
    }

    #[test]
    fn objects_round_trip_sparsely() {
        let shipping = ModelContent {
            object: ModelObject::Box(BoxObject {
                size: [0.3, 0.22, 0.3],
                bevel: 0.008,
            }),
            material: Some(Material {
                color: Color::parse("#D9822B").unwrap(),
                roughness: 0.85,
                ..Material::default()
            }),
            faces: Faces {
                front: Some(Material {
                    texture: Some(Id::new(crate::IdKind::Asset, 2)),
                    texture_size: 0.6,
                    ..Material::default()
                }),
                ..Faces::default()
            },
            floor: Some(Floor::default()),
            ..ModelContent::default()
        };
        let value = serde_json::to_value(&shipping).unwrap();
        assert_eq!(
            value["object"],
            json!({"kind": "box", "size": [0.3, 0.22, 0.3], "bevel": 0.008})
        );
        assert_eq!(
            value["material"],
            json!({"color": "#D9822B", "roughness": 0.85})
        );
        assert_eq!(
            value["faces"],
            json!({"front": {"texture": "asset_0002", "texture_size": 0.6}})
        );
        assert_eq!(value["floor"], json!({}));
        assert!(value.get("light").is_none());
        let back: ModelContent = serde_json::from_value(value).unwrap();
        assert_eq!(back, shipping);

        for object in [
            json!({"kind": "asset", "asset": "asset_0001"}),
            json!({"kind": "sphere", "size": 0.5}),
            json!({"kind": "cylinder"}),
            json!({"kind": "plane", "size": [1.0, 0.5]}),
            json!({"kind": "torus", "thickness": 0.05}),
            json!({"kind": "text", "text": "SALE", "bold": true, "bevel": 0.004}),
            json!({"kind": "path", "path": "M0 0 L10 0 L5 8 Z"}),
            json!({"kind": "blocks", "rows": 12, "seed": 7}),
        ] {
            let content: ModelContent =
                serde_json::from_value(json!({ "object": object })).unwrap();
            assert_eq!(
                serde_json::to_value(&content).unwrap()["object"],
                object,
                "{object}"
            );
            assert!(
                ModelObject::KINDS.contains(&content.object.kind_name()),
                "{object}"
            );
        }
    }

    #[test]
    fn model_clips_round_trip_beside_the_clip_fields() {
        let content = ModelContent {
            emissive_strength: 3.0,
            ..ModelContent::default()
        };
        let mut clip = crate::Clip::new(
            Id::new(crate::IdKind::Clip, 3),
            fosscut_time::Flicks::ZERO,
            fosscut_time::Flicks::from_seconds(2).unwrap(),
            crate::ClipContent::Model {
                model: Box::new(content),
            },
        );
        clip.opacity = 0.5;
        let json = serde_json::to_value(&clip).unwrap();
        assert_eq!(json["kind"], "model");
        assert_eq!(json["model"]["emissive_strength"], 3.0);
        assert_eq!(json["opacity"], 0.5);
        let back: crate::Clip = serde_json::from_value(json).unwrap();
        assert_eq!(back, clip);
        assert_eq!(back.content.kind_name(), "model");
    }

    #[test]
    fn problems_name_the_field() {
        let bad = |content: serde_json::Value| {
            serde_json::from_value::<ModelContent>(content)
                .unwrap()
                .problems()
        };
        let first = |content: serde_json::Value| bad(content).first().cloned().unwrap_or_default();
        assert!(
            first(json!({"object": {"kind": "box", "size": [0.1, 9.0, 0.1]}})).contains("size")
        );
        assert!(first(json!({"object": {"kind": "box", "bevel": 0.2}})).contains("bevel"));
        assert!(
            first(json!({"object": {"kind": "torus", "thickness": 0.2}})).contains("thickness")
        );
        assert!(first(json!({"object": {"kind": "text", "text": "  "}})).contains("text"));
        assert!(
            first(json!({"object": {"kind": "text", "text": "A", "depth": 0.02, "bevel": 0.02}}))
                .contains("bevel")
        );
        assert!(first(json!({"object": {"kind": "path", "path": "nonsense"}})).contains("path"));
        assert!(first(json!({"object": {"kind": "blocks", "rows": 0}})).contains("rows"));
        assert!(
            first(json!({"object": {"kind": "blocks", "heights": [0.3, 0.1]}})).contains("heights")
        );
        assert!(
            first(json!({"object": {"kind": "sphere"}, "faces": {"top": {}}})).contains("faces")
        );
        let title = json!({"kind": "text", "text": "A"});
        assert!(bad(json!({"object": title, "faces": {"front": {}, "back": {}}})).is_empty());
        assert!(first(json!({"object": title, "faces": {"top": {}}})).contains("faces"));
        assert!(first(json!({"material": {"roughness": 2.0}})).contains("roughness"));
        assert!(first(json!({"faces": {"left": {"opacity": -1.0}}})).contains("faces.left"));
        assert!(first(json!({"light": {"elevation": 120.0}})).contains("elevation"));
        assert!(first(json!({"light": {"softness": 3.0}})).contains("softness"));
        assert!(first(json!({"floor": {"shadow": 2.0}})).contains("floor.shadow"));
        assert!(first(json!({"emissive_strength": 500.0})).contains("emissive_strength"));
    }

    #[test]
    fn animatable_values_read_and_write_by_path() {
        let mut content: ModelContent = serde_json::from_value(json!({
            "object": {"kind": "text", "text": "Hi"},
            "floor": {}
        }))
        .unwrap();
        for property in MODEL_PROPERTIES {
            let path = property.path;
            if path == "model.growth" {
                assert_eq!(content.value(path), None, "text has no growth");
                assert!(!content.set_value(path, KeyValue::Number(0.5)));
                continue;
            }
            let Some(KeyValue::Number(v)) = content.value(path) else {
                panic!("{path}");
            };
            assert!(
                content.set_value(path, KeyValue::Number(v + 0.25)),
                "{path}"
            );
            assert_eq!(
                content.value(path),
                Some(KeyValue::Number(v + 0.25)),
                "{path}"
            );
        }
        let mut blocks = ModelContent {
            object: ModelObject::Blocks(BlocksObject::default()),
            ..ModelContent::default()
        };
        assert!(blocks.set_value("model.growth", KeyValue::Number(0.3)));
        assert_eq!(blocks.value("model.growth"), Some(KeyValue::Number(0.3)));
        assert_eq!(blocks.value("model.shadow"), None, "no floor");
        assert!(!blocks.set_value("model.shadow", KeyValue::Number(0.3)));
        assert!(!blocks.set_value("model.light.azimuth", KeyValue::Pair([1.0, 2.0])));
        assert_eq!(model_properties().len(), MODEL_PROPERTIES.len());
    }

    #[test]
    fn assets_are_found_and_replaced() {
        let model = Id::new(crate::IdKind::Asset, 1);
        let logo = Id::new(crate::IdKind::Asset, 2);
        let other = Id::new(crate::IdKind::Asset, 3);
        let mut content = ModelContent {
            object: ModelObject::Asset(AssetObject {
                asset: model.clone(),
                size: 0.4,
            }),
            material: Some(Material {
                texture: Some(logo.clone()),
                ..Material::default()
            }),
            ..ModelContent::default()
        };
        assert_eq!(content.model_asset(), Some(&model));
        assert_eq!(content.texture_assets(), vec![&logo]);
        assert!(content.uses(&model) && content.uses(&logo) && !content.uses(&other));
        content.replace_asset(&logo, &other);
        assert_eq!(content.texture_assets(), vec![&other]);
        content.replace_asset(&model, &logo);
        assert_eq!(content.model_asset(), Some(&logo));
    }

    #[test]
    fn the_key_light_points_where_its_angles_say() {
        let mut light = Light {
            azimuth: 0.0,
            elevation: 0.0,
            ..Light::default()
        };
        let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        assert!(
            close(light.direction(), [0.0, 0.0, -1.0]),
            "from the viewer"
        );
        light.azimuth = 90.0;
        assert!(close(light.direction(), [1.0, 0.0, 0.0]), "from the right");
        light.elevation = 90.0;
        assert!(close(light.direction(), [0.0, -1.0, 0.0]), "from above");
    }
}
