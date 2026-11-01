# ADR-0040: Render lit 3D objects through the timeline's camera

- **Status:** Proposed
- **Date:** 2026-11-01
- **Deciders:** maintainers
- **Related:** [ADR-0004](0004-open-project-format.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0008](0008-render-and-audio-engine.md), [ADR-0011](0011-effects-and-animatable-properties.md), [ADR-0014](0014-licensing.md), [ADR-0015](0015-clean-room-policy.md), [ADR-0028](0028-a-built-in-library-made-by-fosscut.md), [ADR-0037](0037-place-layers-in-3d-with-a-camera-and-depth-of-field.md)

## Context

Motion-graphics ads need real solids, not only flat pictures in depth: a cardboard box with a logo on it standing on a wooden floor with a soft shadow, extruded titles with bevelled edges, a dark city of blocks under a glowing route, a product on a podium. ADR-0037 placed flat layers in 3D and gave every timeline a camera, and promised that a layer kind drawing triangles could join the same scene. Agents need objects they can name and change through the existing property paths, and the render contract (ADR-0008) needs every frame to be a pure function of the project and the time. Shading follows public formulas: the glTF 2.0 metallic-roughness model (Khronos, appendix B), the GGX distribution (Walter et al., EGSR 2007), Smith-Schlick visibility as in Karis (SIGGRAPH 2013), Schlick's Fresnel term, a hemisphere sky light, and percentage-closer filtered shadow maps (Reeves, Salesin and Cook, SIGGRAPH 1987).

## Decision

1. **Model assets** (format 0.16.0, additive). `.glb` files, and `.gltf` files with the buffers and PNG or JPEG images they name, import through `media.add` (`fosscut media import`, MCP `media_import`) as `AssetKind::Model`. `fosscut_engine::model` copies the files into the bundle's `models/<hash>/` folder by content, refuses URIs that leave the file's folder or name another host, and probes `MediaInfo.model`: meshes, triangles, materials, images, the bounds' size and the file's own generator and copyright lines. The `gltf` crate (MIT OR Apache-2.0) reads them; its `import` feature decodes images with the `image` crate.
2. **3D object clips.** `ClipContent::Model { model: Box<ModelContent> }` (`"kind": "model"`) sits on a video track. The object lives under its own key so its fields never meet the clip's. `ModelContent` holds:
   - `object`, tagged by `kind`: `asset` (a model asset and the length of its longest side), `box` (size and a rounded `bevel`), `sphere`, `cylinder`, `plane`, `torus`, `text` (extruded: text, size, font family, bold, italic, depth, chamfer `bevel`), `path` (SVG path data extruded the same way; `model.add` turns a shape clip's closed outline into it) and `blocks` (columns, rows, block, gap, a height range spread by the square of a `splitmix64` number of the seed, fill, growth).
   - `material`, a physically based material: colour, a `texture` image asset printed over the colour by its transparency and covering `texture_size` of a flat face, metallic, roughness, emissive colour and strength, opacity. A model asset keeps its file's materials unless `material` is given. `faces` give a box or the blocks a material per face (`front`, `back`, `left`, `right`, `top`, `bottom`), extruded text and paths a `front` and `back`.
   - `emissive_strength`, multiplying every material's light.
   - `light`, per object clip: a sky light (`ambient`, `sky` and `ground` colours) and one key light (`azimuth`, `elevation`, `color`, `intensity`, `shadows`, `softness`). Lighting is per clip because each clip renders on its own texture and composites as a layer, so one clip's object cannot shade another's anyway; presets give consistent defaults.
   - `floor`: a level plane under the object's lowest point at rest. As a shadow catcher (the default) it shows only the shadow, darkened by `shadow`, so the object stands on footage below; otherwise it is drawn in its colour. It fades out towards its edge.
   - **Units:** sizes, depths, bevels and gaps are fractions of the canvas width, the unit of depth of ADR-0037, so objects are true solids on every canvas. Object space has x right, y down and z away; glTF models are turned to it, centred on their bounds and fitted to their size.
   - **Placement:** the clip's transform places the object as in ADR-0037, with `three_d` implied. Depth scales by the uniform `scale`; the anchor is a fraction of the object's width and height. Effects, masks, blend modes, opacity, animations and transitions apply to the drawn picture.
3. **Animation** (ADR-0011, item 3) with `Applies::Model`: `model.emissive_strength`, `model.light.azimuth`, `model.light.elevation`, `model.light.intensity`, `model.light.ambient`, `model.shadow` (with a floor), `model.depth` (text and paths) and `model.growth` (blocks). Transform keyframes animate placement as before. glTF skins, morph targets, embedded animations, cameras, lights, normal and occlusion maps and texture transforms are not read.
4. **Rendering** (`fosscut_render::objects`, `gpu/objects.rs`, `shaders/objects.wgsl`). The planner makes a `LayerSource::Model` layer with the object at its time and its texture frames; `place_layers` gives it a scene `LayerSpace` from its position, so `order_by_depth` sorts it among planar 3D layers and the depth of field blurs it whole. The renderer first draws the object's depth from the key light into a square map fitted to its bounding sphere, sized so a 9 by 9 grid of hardware-filtered comparisons spans the penumbra `softness` asks for, then draws the mesh with `Camera::clip_from_world`, a depth buffer of its own and 4x multisampling into a canvas-sized premultiplied linear texture: solid parts, then the floor, then see-through parts. That texture composites like any picture at the canvas's size. Meshes and file images are cached on the GPU by what they are made from. Everything is a pure function of the plan, so preview, frame grabs and export agree. Motion blur does not sample objects yet.
5. **Library** (ADR-0028): FossCut's own presets in `fosscut-core/presets/library/models.json`, `shipping-box`, `city-blocks`, `podium`, `extruded-title`, `neon-ring` and `chrome-sphere`, each with a suggested placement, listed by `capabilities` under `library.models` and by `fosscut_engine::library` as kind `model`.
6. **Commands and reach.** `model.add` (a preset, a model `asset`, `text`, a `shape_clip`, a merge patch `set`, a transform; on a track of 3D objects above the footage unless a track is given; 5 seconds by default) and `model.set` (a preset, sources and a merge patch; a patch naming another `object.kind` starts that object afresh; settings with keyframes are refused with `fosscut.clip.property_animated`). Errors: `fosscut.object.invalid`, `fosscut.object.unreadable` (a model file that cannot be read when rendering), `fosscut.asset.not_found`, `fosscut.library.unknown_item`. `clip.add` refuses a model asset and points to `model.add`. Model and texture assets count as used for `media.remove` and follow `media.replace`. The CLI has `fosscut model presets|add|set`; MCP reaches them through `edit`, `media_import` and `library`, with no new tool. `capabilities` lists the commands, the properties and, under `models`, the kinds, faces, units, limits and schema.

## Consequences

- Product shots, extruded titles and city maps are built from data agents can read and change, and stay editable and undoable.
- An object's shadow falls only on itself and its own floor: objects in different clips do not shadow each other, and a grid of blocks shadows itself because it is one clip.
- Floors stay level, so tilting a scene is the camera's job (orbit pitch), not the object's `rotation_x`.
- Each object costs a shadow map, a multisampled canvas-sized target and a mipmapped texture. In a debug build on an Apple M3 Pro, objects add 3 to 9 ms to a 1080p frame (11 ms with the depth of field's blur).
- See-through parts are not sorted against each other, and polygon outlines are filled by the even-odd rule.
- Bundles grow by the size of the model files they use.

## Alternatives considered

- **Lights as clips or timeline settings:** one light rig for every object reads well, but clips render alone and cannot shadow each other, so a shared light would only move the numbers; it can come with shared scene rendering.
- **One depth buffer for all 3D layers and objects:** correct for intersections, but planar layers need ordered, blended compositing (ADR-0037).
- **A general scene graph or a 3D engine crate:** more than motion graphics needs, harder to keep deterministic and render-parity safe, and heavier to build for the web.
- **Reading other 3D formats (OBJ, FBX, USD):** glTF is the open, documented interchange format; others can be converted to it.
- **Variance or exponential shadow maps:** cheaper wide blurs, but they leak light at contacts; the fixed PCF grid is exact and the same on every adapter.

## Validation

- Model: sparse round trips beside the clip's fields, validation of every range, asset kinds and textures, and the vocabulary (`crates/fosscut-model/src/model3d.rs`, `tests/objects.rs`).
- Meshes: sizes and outward normals of every solid, rounded edges, extrusion caps with holes and bevels that never fold, glyph caps covering their outlines exactly, seeded grids, glTF nodes, axes, materials and computed normals from a model generated in the test.
- Commands: contract samples for `model.add` and `model.set`, presets, sources, typed errors, keyframes, `media.remove` and `media.replace` (`crates/fosscut-core/tests/model.rs`); import into the bundle with the files a `.gltf` names, refusal of files outside its folder, and rendering from the copy (`crates/fosscut-engine/tests/model.rs`); a CLI end-to-end test with frame grabs that match across processes and an export.
- GPU (`crates/fosscut-render/tests/objects.rs`): soft shadows on a catcher, printed faces, sorting between cards, depth of field, glTF materials and textures, determinism on a fresh renderer, preview matching export above 38 dB.
- Golden frames, reviewed: a shipping box with a logo and a soft shadow over planks, a city of blocks seen by a tilted camera, extruded and bevelled letters, a ring between two cards, a sphere in the depth of field, and a glTF house.
- Before Accepted: a maintainer reviews an ad made by an agent with these objects, and an optimized build is measured at 1080p.
