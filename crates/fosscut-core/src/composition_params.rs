//! Parameters of library web compositions (ADR-0042): typed values a composition declares
//! (colours, texts, numbers, switches, choices, point lists, groups) and slots for media
//! (images, videos, SVG drawings, 3D models and other web compositions). Front ends resolve
//! them here, write them into the composition's folder as `params.json`, and copy the slots'
//! files beside it, so the page reads only its own folder. Compositions made with the
//! Three.js scene kit share the scene parameters of `presets/library/scene-params.json`;
//! those made with the overlay kit share the timing and theme parameters of
//! `presets/library/overlay-params.json`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{ApiError, codes};

const SCENE_PARAMS: &str = include_str!("../presets/library/scene-params.json");
const OVERLAY_PARAMS: &str = include_str!("../presets/library/overlay-params.json");

/// Seconds an overlay's exit takes (`EXIT` in `overlay-kit/overlay.js`): an overlay whose
/// length is not given lasts `appear` plus `hold`, plus this when `exit` is on.
pub const OVERLAY_EXIT_SECONDS: f64 = 0.4;

/// Every file of the Three.js scene kit, by its path in a composition's folder: FossCut's
/// stage module and Three.js itself (MIT, `presets/library/vendor/three/README.md`).
pub(crate) const THREE_KIT_FILES: [(&str, &[u8]); 10] = [
    (
        "kit/stage.js",
        include_bytes!("../presets/library/scene-kit/stage.js"),
    ),
    (
        "vendor/three/LICENSE",
        include_bytes!("../presets/library/vendor/three/LICENSE"),
    ),
    (
        "vendor/three/three.module.min.js",
        include_bytes!("../presets/library/vendor/three/three.module.min.js"),
    ),
    (
        "vendor/three/three.core.min.js",
        include_bytes!("../presets/library/vendor/three/three.core.min.js"),
    ),
    (
        "vendor/three/addons/renderers/CSS3DRenderer.js",
        include_bytes!("../presets/library/vendor/three/addons/renderers/CSS3DRenderer.js"),
    ),
    (
        "vendor/three/addons/loaders/SVGLoader.js",
        include_bytes!("../presets/library/vendor/three/addons/loaders/SVGLoader.js"),
    ),
    (
        "vendor/three/addons/loaders/GLTFLoader.js",
        include_bytes!("../presets/library/vendor/three/addons/loaders/GLTFLoader.js"),
    ),
    (
        "vendor/three/addons/geometries/RoundedBoxGeometry.js",
        include_bytes!("../presets/library/vendor/three/addons/geometries/RoundedBoxGeometry.js"),
    ),
    (
        "vendor/three/addons/utils/BufferGeometryUtils.js",
        include_bytes!("../presets/library/vendor/three/addons/utils/BufferGeometryUtils.js"),
    ),
    (
        "vendor/three/addons/utils/SkeletonUtils.js",
        include_bytes!("../presets/library/vendor/three/addons/utils/SkeletonUtils.js"),
    ),
];

/// Every file of the overlay kit, by its path in a composition's folder: FossCut's motion,
/// theme and media script and its style sheet, and the bundled Inter font (SIL Open Font
/// License, `fosscut-render/assets/fonts/inter/README.md`) in the weights the overlays use.
pub(crate) const OVERLAY_KIT_FILES: [(&str, &[u8]); 6] = [
    (
        "kit/overlay.js",
        include_bytes!("../presets/library/overlay-kit/overlay.js"),
    ),
    (
        "kit/overlay.css",
        include_bytes!("../presets/library/overlay-kit/overlay.css"),
    ),
    (
        "kit/fonts/Inter-Regular.ttf",
        include_bytes!("../../fosscut-render/assets/fonts/inter/Inter-Regular.ttf"),
    ),
    (
        "kit/fonts/Inter-Bold.ttf",
        include_bytes!("../../fosscut-render/assets/fonts/inter/Inter-Bold.ttf"),
    ),
    (
        "kit/fonts/Inter-ExtraBold.ttf",
        include_bytes!("../../fosscut-render/assets/fonts/inter/Inter-ExtraBold.ttf"),
    ),
    (
        "kit/fonts/OFL.txt",
        include_bytes!("../../fosscut-render/assets/fonts/inter/OFL.txt"),
    ),
];

/// A kit of shared files a composition is made with.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompositionKit {
    /// FossCut's Three.js scene kit (`kit/stage.js`) with Three.js under `vendor/three/`,
    /// and the scene parameters (transparency, background, exposure, tone mapping, bloom,
    /// depth of field, motion blur and the canvas).
    Three,
    /// FossCut's overlay kit for explainer cards (`kit/overlay.js`, `kit/overlay.css` and
    /// the Inter font): keyframed motion, light and dark themes with an accent colour,
    /// media slots with generated placeholders, and the timing parameters `appear`,
    /// `hold`, `exit` and `speed` with the theme and the canvas.
    Overlay,
}

impl CompositionKit {
    /// The kit's files by their path in a composition's folder.
    pub fn files(self) -> &'static [(&'static str, &'static [u8])] {
        match self {
            CompositionKit::Three => &THREE_KIT_FILES,
            CompositionKit::Overlay => &OVERLAY_KIT_FILES,
        }
    }

    /// The parameters every composition of the kit has.
    pub fn params(self) -> Vec<CompositionParam> {
        match self {
            CompositionKit::Three => serde_json::from_str(SCENE_PARAMS).unwrap_or_default(),
            CompositionKit::Overlay => serde_json::from_str(OVERLAY_PARAMS).unwrap_or_default(),
        }
    }
}

/// What kind of value a composition parameter takes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    /// A colour as `#RRGGBB` or `#RRGGBBAA` (`#RGB` is expanded).
    Color,
    /// Text, at most `max_length` characters (200 when not given).
    Text,
    /// A number from `min` to `max`, a whole number when `integer`.
    Number,
    /// `true` or `false`.
    Bool,
    /// One of `choices`.
    Choice,
    /// A list of `[x, y]` pairs, each coordinate from `min` to `max`, with 1 to
    /// `max_length` points (64 when not given).
    Points,
    /// Media for the page: an asset id of a kind in `accepts`, or for compositions also the
    /// name of a library composition. Empty (`null`) uses the page's own.
    Slot,
    /// Several values in an object, described by `fields`.
    Group,
    /// A list of texts, at most `max_length` of them (12 when not given), each at most
    /// 200 characters; it may be empty.
    Texts,
    /// A list of numbers, each from `min` to `max` and whole when `integer`, at most
    /// `max_length` of them (64 when not given); it may be empty.
    Numbers,
}

/// The media a slot takes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    /// A still image asset (PNG, JPEG, WebP and other formats the browser shows).
    Image,
    /// A video asset; the browser must play its format (H.264 MP4 or WebM).
    Video,
    /// A vector drawing asset (`.svg`).
    Svg,
    /// A 3D model asset (`.glb`, or `.gltf` with its files).
    Model,
    /// A web composition asset, or a library composition by name, shown live at its own
    /// clock.
    Composition,
}

impl SlotKind {
    /// The name used in parameters and `params.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            SlotKind::Image => "image",
            SlotKind::Video => "video",
            SlotKind::Svg => "svg",
            SlotKind::Model => "model",
            SlotKind::Composition => "composition",
        }
    }
}

/// One parameter a library composition declares.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompositionParam {
    /// The name, unique within the composition (or the group).
    pub name: String,
    /// The kind of value.
    #[serde(rename = "type")]
    pub kind: ParamType,
    /// What it changes.
    pub description: String,
    /// The value when none is given; `null` for slots and canvas values that keep the
    /// page's own.
    #[serde(default)]
    pub default: Value,
    /// Smallest number, or smallest coordinate of points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Largest number, or largest coordinate of points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Numbers must be whole.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub integer: bool,
    /// Most characters of text, or most points, texts or numbers in a list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    /// The values a choice takes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// The media a slot takes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepts: Vec<SlotKind>,
    /// The values of a group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<CompositionParam>,
}

/// The canvas values a composition's parameters may change in its meta tag.
pub const CANVAS_PARAMS: [&str; 4] = ["width", "height", "duration", "fps"];

/// The parameters of a composition made with `kit` and declaring `own`, with the kit's
/// defaults replaced by `defaults` (groups field by field). A composition's own parameter
/// replaces the kit's of the same name.
pub(crate) fn all_params(
    own: &[CompositionParam],
    kit: Option<CompositionKit>,
    defaults: &Map<String, Value>,
) -> Vec<CompositionParam> {
    let mut params = own.to_vec();
    if let Some(kit) = kit {
        for mut param in kit.params() {
            if params.iter().any(|p| p.name == param.name) {
                continue;
            }
            if let Some(value) = defaults.get(&param.name) {
                apply_default(&mut param, value);
            }
            params.push(param);
        }
    }
    params
}

fn apply_default(param: &mut CompositionParam, value: &Value) {
    match (param.kind, value) {
        (ParamType::Group, Value::Object(fields)) => {
            for field in &mut param.fields {
                if let Some(value) = fields.get(&field.name) {
                    apply_default(field, value);
                }
            }
            param.default = group_default(&param.fields);
        }
        _ => param.default = value.clone(),
    }
}

fn group_default(fields: &[CompositionParam]) -> Value {
    Value::Object(
        fields
            .iter()
            .map(|f| (f.name.clone(), f.default.clone()))
            .collect(),
    )
}

/// The default of every parameter, groups as objects of their fields' defaults.
pub fn defaults(params: &[CompositionParam]) -> Map<String, Value> {
    params
        .iter()
        .map(|p| {
            let value = if p.kind == ParamType::Group {
                group_default(&p.fields)
            } else {
                p.default.clone()
            };
            (p.name.clone(), value)
        })
        .collect()
}

fn invalid(composition: &str, message: String) -> ApiError {
    ApiError::new(
        codes::LIBRARY_INVALID_PARAMS,
        format!("{composition}: {message}"),
    )
    .with_hint(
        "library list (kind composition) shows each composition's params with their types, ranges and defaults.",
    )
}

/// Every parameter of `params` with the value given in `given` (an object, or null for
/// none) or its default, checked against its type. Slots in `media` are filled, in the
/// order the slots are declared, unless `given` names them. Unknown names are refused.
pub fn resolve(
    composition: &str,
    params: &[CompositionParam],
    given: Option<&Value>,
    media: &[String],
) -> Result<Map<String, Value>, ApiError> {
    let given = match given {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(other) => {
            return Err(invalid(
                composition,
                format!("params must be an object of names and values, not {other}"),
            ));
        }
    };
    let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
    if let Some(unknown) = given.keys().find(|k| !names.contains(&k.as_str())) {
        return Err(invalid(
            composition,
            format!(
                "it has no parameter {unknown:?}; its parameters are {}",
                names.join(", ")
            ),
        ));
    }
    let mut out = Map::new();
    let mut media = media.iter();
    for param in params {
        let value = match given.get(&param.name) {
            Some(value) => check(composition, param, &param.name, value)?,
            None if param.kind == ParamType::Slot => match media.next() {
                Some(asset) => check(composition, param, &param.name, &Value::from(asset.clone()))?,
                None => param.default.clone(),
            },
            None if param.kind == ParamType::Group => {
                check(composition, param, &param.name, &Value::Object(Map::new()))?
            }
            None => param.default.clone(),
        };
        out.insert(param.name.clone(), value);
    }
    if let Some(extra) = media.next() {
        return Err(invalid(
            composition,
            format!("{extra} was given for a slot, but every slot is already filled"),
        ));
    }
    Ok(out)
}

/// `value` checked against `param` (`path` names it in messages), normalized: colours in
/// upper case with six or eight digits, groups with every field.
fn check(
    composition: &str,
    param: &CompositionParam,
    path: &str,
    value: &Value,
) -> Result<Value, ApiError> {
    let fail = |message: String| Err(invalid(composition, format!("{path} {message}")));
    if value.is_null() {
        // Slots can always be emptied; other values only when their default is empty.
        return if param.default.is_null() || param.kind == ParamType::Slot {
            Ok(Value::Null)
        } else {
            fail("cannot be null".into())
        };
    }
    match param.kind {
        ParamType::Color => match value.as_str().and_then(normalize_color) {
            Some(color) => Ok(Value::from(color)),
            None => fail(format!("must be a colour like \"#FF7A2F\", not {value}")),
        },
        ParamType::Text => {
            let limit = param.max_length.unwrap_or(200);
            match value.as_str() {
                Some(text) if text.chars().count() <= limit => Ok(value.clone()),
                Some(_) => fail(format!("must be at most {limit} characters")),
                None => fail(format!("must be text, not {value}")),
            }
        }
        ParamType::Number => match value.as_f64() {
            Some(n) => check_number(param, n)
                .map(|n| number(param, n))
                .or_else(fail),
            None => fail(format!("must be a number, not {value}")),
        },
        ParamType::Bool => match value {
            Value::Bool(_) => Ok(value.clone()),
            _ => fail(format!("must be true or false, not {value}")),
        },
        ParamType::Choice => match value.as_str() {
            Some(choice) if param.choices.iter().any(|c| c == choice) => Ok(value.clone()),
            _ => fail(format!(
                "must be one of {}, not {value}",
                param.choices.join(", ")
            )),
        },
        ParamType::Points => check_points(param, value).or_else(fail),
        ParamType::Texts => check_texts(param, value).or_else(fail),
        ParamType::Numbers => check_numbers(param, value).or_else(fail),
        ParamType::Slot => match value.as_str() {
            Some(text) if text.starts_with("asset_") && text.len() > 6 => Ok(value.clone()),
            Some(name)
                if param.accepts.contains(&SlotKind::Composition)
                    && crate::library_files::compositions()
                        .iter()
                        .any(|c| c.name == name && !c.has_composition_slot()) =>
            {
                Ok(value.clone())
            }
            _ => fail(format!(
                "takes an asset id ({}){}, not {value}",
                param
                    .accepts
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                if param.accepts.contains(&SlotKind::Composition) {
                    " or the name of a library composition without slots for compositions"
                } else {
                    ""
                }
            )),
        },
        ParamType::Group => {
            let Some(fields) = value.as_object() else {
                return fail(format!("must be an object, not {value}"));
            };
            let names: Vec<&str> = param.fields.iter().map(|f| f.name.as_str()).collect();
            if let Some(unknown) = fields.keys().find(|k| !names.contains(&k.as_str())) {
                return fail(format!(
                    "has no field {unknown:?}; its fields are {}",
                    names.join(", ")
                ));
            }
            let mut out = Map::new();
            for field in &param.fields {
                let value = match fields.get(&field.name) {
                    Some(value) => {
                        check(composition, field, &format!("{path}.{}", field.name), value)?
                    }
                    None => field.default.clone(),
                };
                out.insert(field.name.clone(), value);
            }
            Ok(Value::Object(out))
        }
    }
}

fn check_number(param: &CompositionParam, n: f64) -> Result<f64, String> {
    if !n.is_finite() {
        return Err("must be a finite number".into());
    }
    if param.integer && n.fract() != 0.0 {
        return Err(format!("must be a whole number, not {n}"));
    }
    if let Some(min) = param.min
        && n < min
    {
        return Err(format!("must be at least {min}, not {n}"));
    }
    if let Some(max) = param.max
        && n > max
    {
        return Err(format!("must be at most {max}, not {n}"));
    }
    Ok(n)
}

fn number(param: &CompositionParam, n: f64) -> Value {
    if param.integer {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

fn check_points(param: &CompositionParam, value: &Value) -> Result<Value, String> {
    let limit = param.max_length.unwrap_or(64);
    let Some(points) = value.as_array() else {
        return Err(format!("must be a list of [x, y] pairs, not {value}"));
    };
    if points.is_empty() || points.len() > limit {
        return Err(format!("must have 1 to {limit} points"));
    }
    let mut out = Vec::new();
    for point in points {
        let pair = point
            .as_array()
            .filter(|p| p.len() == 2)
            .and_then(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
            .ok_or_else(|| format!("has {point}, not an [x, y] pair of numbers"))?;
        let mut checked = Vec::new();
        for n in pair {
            let n = check_number(param, n).map_err(|e| format!("coordinate {e}"))?;
            checked.push(number(param, n));
        }
        out.push(Value::Array(checked));
    }
    Ok(Value::Array(out))
}

fn list<'a>(value: &'a Value, limit: usize, what: &str) -> Result<&'a Vec<Value>, String> {
    let Some(items) = value.as_array() else {
        return Err(format!("must be a list of {what}, not {value}"));
    };
    if items.len() > limit {
        return Err(format!("must have at most {limit} {what}"));
    }
    Ok(items)
}

fn check_texts(param: &CompositionParam, value: &Value) -> Result<Value, String> {
    let items = list(value, param.max_length.unwrap_or(12), "texts")?;
    for item in items {
        match item.as_str() {
            Some(text) if text.chars().count() <= 200 => {}
            Some(_) => return Err("has a text longer than 200 characters".into()),
            None => return Err(format!("has {item}, not a text")),
        }
    }
    Ok(value.clone())
}

fn check_numbers(param: &CompositionParam, value: &Value) -> Result<Value, String> {
    let items = list(value, param.max_length.unwrap_or(64), "numbers")?;
    let mut out = Vec::new();
    for item in items {
        let n = item
            .as_f64()
            .ok_or_else(|| format!("has {item}, not a number"))?;
        let n = check_number(param, n).map_err(|e| format!("item {e}"))?;
        out.push(number(param, n));
    }
    Ok(Value::Array(out))
}

/// `#RGB`, `#RRGGBB` or `#RRGGBBAA` in upper case with six or eight digits.
fn normalize_color(text: &str) -> Option<String> {
    let hex = text.trim().strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let full: String = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => hex.to_string(),
        _ => return None,
    };
    Some(format!("#{}", full.to_ascii_uppercase()))
}

/// The length of an overlay with `params` when its `duration` is not given: `appear` plus
/// `hold`, plus the exit when `exit` is on, in seconds rounded up to a millisecond.
pub(crate) fn overlay_duration(params: &Map<String, Value>) -> Option<f64> {
    if params.get("duration").and_then(Value::as_f64).is_some() {
        return None;
    }
    let appear = params.get("appear").and_then(Value::as_f64)?;
    let hold = params.get("hold").and_then(Value::as_f64)?;
    let exit = params.get("exit").and_then(Value::as_bool).unwrap_or(true);
    let seconds = appear + hold + if exit { OVERLAY_EXIT_SECONDS } else { 0.0 };
    // Up to the next millisecond, without letting float error in the sum add one.
    Some((seconds * 1000.0 - 1e-6).ceil() / 1000.0)
}

/// `html` with the given canvas values (`width`, `height`, `duration` in seconds, `fps`) in
/// its `fosscut:composition` meta tag; values that are null or missing keep the page's own.
pub(crate) fn with_canvas(html: &str, values: &Map<String, Value>) -> String {
    let changes: Vec<(&str, String)> = CANVAS_PARAMS
        .iter()
        .filter_map(|key| {
            let value = values.get(*key)?.as_f64()?;
            Some((*key, format_number(value)))
        })
        .collect();
    if changes.is_empty() {
        return html.to_string();
    }
    let lower = html.to_ascii_lowercase();
    let Some(name) = lower.find("name=\"fosscut:composition\"") else {
        return html.to_string();
    };
    let Some(tag_start) = lower[..name].rfind("<meta") else {
        return html.to_string();
    };
    let tag_end = lower[tag_start..]
        .find('>')
        .map_or(lower.len(), |e| tag_start + e);
    let Some(open) = lower[tag_start..tag_end]
        .find("content=\"")
        .map(|c| tag_start + c + "content=\"".len())
    else {
        return html.to_string();
    };
    let Some(close) = html[open..tag_end].find('"').map(|c| open + c) else {
        return html.to_string();
    };
    let mut pairs: Vec<(String, String)> = html[open..close]
        .split([',', ';'])
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((key.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();
    for (key, value) in changes {
        match pairs.iter_mut().find(|(k, _)| k == key) {
            Some(pair) => pair.1 = value,
            None => pairs.push((key.to_string(), value)),
        }
    }
    let content = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}{content}{}", &html[..open], &html[close..])
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn params() -> Vec<CompositionParam> {
        serde_json::from_value(json!([
            { "name": "tint", "type": "color", "default": "#FF7A2F", "description": "d" },
            { "name": "title", "type": "text", "default": "Hello", "max_length": 5, "description": "d" },
            { "name": "count", "type": "number", "integer": true, "min": 0, "max": 10, "default": 3, "description": "d" },
            { "name": "speed", "type": "number", "min": 0.5, "default": 1, "description": "d" },
            { "name": "loop", "type": "bool", "default": false, "description": "d" },
            { "name": "look", "type": "choice", "choices": ["a", "b"], "default": "a", "description": "d" },
            { "name": "path", "type": "points", "min": -5, "max": 5, "max_length": 3, "default": [[0, 0], [1, 1]], "description": "d" },
            { "name": "logo", "type": "slot", "accepts": ["image", "svg"], "default": null, "description": "d" },
            { "name": "screen", "type": "slot", "accepts": ["composition", "video"], "default": "shop-app", "description": "d" },
            { "name": "glow", "type": "group", "description": "d", "fields": [
                { "name": "strength", "type": "number", "min": 0, "max": 2, "default": 1, "description": "d" },
                { "name": "on", "type": "bool", "default": true, "description": "d" }
            ] }
        ]))
        .unwrap()
    }

    #[test]
    fn defaults_fill_what_is_not_given() {
        let all = resolve("x", &params(), None, &[]).unwrap();
        assert_eq!(all["tint"], "#FF7A2F");
        assert_eq!(all["count"], 3);
        assert_eq!(all["logo"], Value::Null);
        assert_eq!(all["screen"], "shop-app");
        assert_eq!(all["glow"], json!({ "strength": 1, "on": true }));
        assert_eq!(Value::Object(all), Value::Object(defaults(&params())));
    }

    #[test]
    fn given_values_are_checked_and_normalized() {
        let given = json!({
            "tint": "#0af", "count": 7.0, "speed": 2.5, "loop": true, "look": "b",
            "path": [[-5, 5], [2, 2.5]], "logo": "asset_0003", "screen": null,
            "glow": { "strength": 0.25 }
        });
        let all = resolve("x", &params(), Some(&given), &[]).unwrap();
        assert_eq!(all["tint"], "#00AAFF");
        assert_eq!(all["count"], json!(7), "whole numbers stay whole");
        assert_eq!(all["path"], json!([[-5.0, 5.0], [2.0, 2.5]]));
        assert_eq!(all["logo"], "asset_0003");
        assert_eq!(all["screen"], Value::Null, "a slot is emptied with null");
        assert_eq!(all["glow"], json!({ "strength": 0.25, "on": true }));

        let refused = |given: Value| {
            let error = resolve("x", &params(), Some(&given), &[]).unwrap_err();
            assert_eq!(error.code, codes::LIBRARY_INVALID_PARAMS, "{given}");
            error.message
        };
        assert!(refused(json!({ "colour": "#fff" })).contains("no parameter \"colour\""));
        assert!(refused(json!({ "tint": "orange" })).contains("tint must be a colour"));
        assert!(refused(json!({ "tint": "#12345" })).contains("tint"));
        assert!(refused(json!({ "title": "Too long" })).contains("at most 5"));
        assert!(refused(json!({ "count": 2.5 })).contains("whole number"));
        assert!(refused(json!({ "count": 11 })).contains("at most 10"));
        assert!(refused(json!({ "speed": 0.1 })).contains("at least 0.5"));
        assert!(refused(json!({ "speed": "fast" })).contains("must be a number"));
        assert!(refused(json!({ "loop": 1 })).contains("true or false"));
        assert!(refused(json!({ "look": "c" })).contains("one of a, b"));
        assert!(refused(json!({ "path": [] })).contains("1 to 3 points"));
        assert!(refused(json!({ "path": [[0, 0], [9, 0]] })).contains("coordinate"));
        assert!(refused(json!({ "path": [[0]] })).contains("[x, y] pair"));
        assert!(refused(json!({ "logo": "shop-app" })).contains("asset id (image, svg)"));
        assert!(refused(json!({ "screen": "no-such" })).contains("library composition"));
        assert!(refused(json!({ "glow": { "size": 1 } })).contains("glow has no field"));
        assert!(refused(json!({ "glow": { "strength": 3 } })).contains("glow.strength"));
        assert!(refused(json!({ "tint": null })).contains("cannot be null"));
        assert!(refused(json!([1])).contains("must be an object"));
    }

    #[test]
    fn media_fill_the_slots_in_order() {
        let all = resolve(
            "x",
            &params(),
            None,
            &["asset_0001".into(), "asset_0002".into()],
        )
        .unwrap();
        assert_eq!(
            (all["logo"].as_str(), all["screen"].as_str()),
            (Some("asset_0001"), Some("asset_0002"))
        );
        let named = json!({ "logo": "asset_0005" });
        let all = resolve("x", &params(), Some(&named), &["asset_0001".into()]).unwrap();
        assert_eq!(
            (all["logo"].as_str(), all["screen"].as_str()),
            (Some("asset_0005"), Some("asset_0001"))
        );
        let error = resolve(
            "x",
            &params(),
            None,
            &["asset_1".into(), "asset_2".into(), "asset_3".into()],
        )
        .unwrap_err();
        assert!(error.message.contains("every slot is already filled"));
    }

    #[test]
    fn the_kit_adds_scene_params_with_the_compositions_defaults() {
        let own: Vec<CompositionParam> = serde_json::from_value(json!([
            { "name": "background", "type": "color", "default": "#111111", "description": "mine" }
        ]))
        .unwrap();
        let mut overrides = Map::new();
        overrides.insert("bloom".into(), json!({ "strength": 2 }));
        overrides.insert("exposure".into(), json!(1.5));
        let all = all_params(&own, Some(CompositionKit::Three), &overrides);
        let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        for name in [
            "background",
            "transparent",
            "bloom",
            "depth_of_field",
            "motion_blur",
            "width",
            "fps",
        ] {
            assert!(names.contains(&name), "{name}");
        }
        assert_eq!(names.iter().filter(|n| **n == "background").count(), 1);
        let values = defaults(&all);
        assert_eq!(
            values["background"], "#111111",
            "the composition's own wins"
        );
        assert_eq!(values["exposure"], 1.5);
        assert_eq!(values["bloom"]["strength"], 2);
        assert_eq!(
            values["bloom"]["radius"], 0.6,
            "other fields keep the kit's"
        );
        assert_eq!(values["motion_blur"]["samples"], 8);
        assert_eq!(values["width"], Value::Null);
        assert!(all_params(&own, None, &overrides).len() == 1);
        // Every kit file is in place.
        for (path, bytes) in CompositionKit::Three.files() {
            assert!(!bytes.is_empty(), "{path}");
        }
    }

    #[test]
    fn canvas_values_rewrite_the_meta_tag() {
        let html = r#"<head><meta charset="utf-8"><meta name="fosscut:composition" content="width=1080, height=1920, fps=30, duration=7"></head>"#;
        let mut values = Map::new();
        assert_eq!(with_canvas(html, &values), html);
        values.insert("width".into(), json!(540));
        values.insert("duration".into(), json!(2.5));
        values.insert("height".into(), Value::Null);
        assert_eq!(
            with_canvas(html, &values),
            r#"<head><meta charset="utf-8"><meta name="fosscut:composition" content="width=540, height=1920, fps=30, duration=2.5"></head>"#
        );
        assert_eq!(with_canvas("<p>no tag</p>", &values), "<p>no tag</p>");
    }

    fn lists() -> Vec<CompositionParam> {
        serde_json::from_value(json!([
            { "name": "steps", "type": "texts", "max_length": 3, "default": ["a", "b"], "description": "d" },
            { "name": "values", "type": "numbers", "min": 0, "max": 10, "max_length": 4, "default": [1, 2], "description": "d" },
            { "name": "counts", "type": "numbers", "integer": true, "min": 0, "max": 9, "default": [3], "description": "d" }
        ]))
        .unwrap()
    }

    #[test]
    fn lists_of_texts_and_numbers_are_checked() {
        let all = resolve("x", &lists(), None, &[]).unwrap();
        assert_eq!(all["steps"], json!(["a", "b"]));
        assert_eq!(all["values"], json!([1, 2]));
        let given = json!({ "steps": [], "values": [0.5, 10], "counts": [4.0] });
        let all = resolve("x", &lists(), Some(&given), &[]).unwrap();
        assert_eq!(all["steps"], json!([]), "a list may be empty");
        assert_eq!(all["values"], json!([0.5, 10.0]));
        assert_eq!(all["counts"], json!([4]), "whole numbers stay whole");

        let refused = |given: Value| {
            let error = resolve("x", &lists(), Some(&given), &[]).unwrap_err();
            assert_eq!(error.code, codes::LIBRARY_INVALID_PARAMS, "{given}");
            error.message
        };
        assert!(refused(json!({ "steps": "one" })).contains("list of texts"));
        assert!(refused(json!({ "steps": ["a", "b", "c", "d"] })).contains("at most 3 texts"));
        assert!(refused(json!({ "steps": [1] })).contains("not a text"));
        assert!(refused(json!({ "steps": ["x".repeat(201)] })).contains("longer than 200"));
        assert!(refused(json!({ "values": [1, 2, 3, 4, 5] })).contains("at most 4 numbers"));
        assert!(refused(json!({ "values": [11] })).contains("item must be at most 10"));
        assert!(refused(json!({ "values": ["1"] })).contains("not a number"));
        assert!(refused(json!({ "counts": [1.5] })).contains("whole number"));
        assert!(refused(json!({ "steps": null })).contains("cannot be null"));
    }

    #[test]
    fn the_overlay_kit_adds_timing_theme_and_canvas_params() {
        let mut overrides = Map::new();
        overrides.insert("hold".into(), json!(5));
        overrides.insert("theme".into(), json!("dark"));
        let all = all_params(&[], Some(CompositionKit::Overlay), &overrides);
        let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "appear", "hold", "exit", "speed", "theme", "accent", "width", "height",
                "duration", "fps"
            ]
        );
        let values = defaults(&all);
        assert_eq!(values["hold"], 5);
        assert_eq!(values["theme"], "dark");
        assert_eq!(values["appear"], 0);
        assert_eq!(values["exit"], true);
        assert_eq!(values["duration"], Value::Null);
        // Every kit file is in place: the scripts, the style sheet and the fonts.
        let files = CompositionKit::Overlay.files();
        for (path, bytes) in files {
            assert!(!bytes.is_empty(), "{path}");
            if path.ends_with(".ttf") {
                assert_eq!(&bytes[..4], &[0, 1, 0, 0], "{path} is a TrueType font");
            }
        }
        let script = files
            .iter()
            .find(|(p, _)| *p == "kit/overlay.js")
            .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
            .unwrap();
        assert!(
            script.contains(&format!("const EXIT = {OVERLAY_EXIT_SECONDS};")),
            "the kit's exit lasts as long as fosscut-core expects"
        );
        let style = files
            .iter()
            .find(|(p, _)| *p == "kit/overlay.css")
            .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
            .unwrap();
        for (path, _) in files.iter().filter(|(p, _)| p.ends_with(".ttf")) {
            let font = path.trim_start_matches("kit/");
            assert!(style.contains(&format!("url('{font}')")), "{font}");
        }
    }

    #[test]
    fn overlays_last_until_their_exit_is_over() {
        let mut params = Map::new();
        params.insert("appear".into(), json!(0.5));
        params.insert("hold".into(), json!(2));
        params.insert("exit".into(), json!(true));
        params.insert("duration".into(), Value::Null);
        assert_eq!(overlay_duration(&params), Some(2.9));
        params.insert("exit".into(), json!(false));
        assert_eq!(overlay_duration(&params), Some(2.5));
        params.insert("hold".into(), json!(1.0 / 3.0));
        assert_eq!(
            overlay_duration(&params),
            Some(0.834),
            "rounded up to a millisecond"
        );
        params.insert("duration".into(), json!(4));
        assert_eq!(overlay_duration(&params), None, "a given length wins");
        assert_eq!(overlay_duration(&Map::new()), None);
    }
}
