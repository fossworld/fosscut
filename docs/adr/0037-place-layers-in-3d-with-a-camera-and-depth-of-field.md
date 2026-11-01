# ADR-0037: Place layers in 3D with a camera and depth of field

- **Status:** Proposed
- **Date:** 2026-11-01
- **Deciders:** maintainers
- **Related:** [ADR-0004](0004-open-project-format.md), [ADR-0005](0005-time-model.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0008](0008-render-and-audio-engine.md), [ADR-0011](0011-effects-and-animatable-properties.md), [ADR-0025](0025-nest-timelines-in-clips-for-compound-clips.md), [ADR-0035](0035-render-temporal-effects-from-neighbouring-frames.md)

## Context

Motion-graphics ads place flat pictures in depth: product cards fly out of a phone, a city map lies tilted in perspective, a phone turns toward the viewer, and a camera pushes in while the background goes soft. Clip transforms are 2D (position, scale, stretch, rotation, flips), and `fosscut-render` composites layers as 2D quads. Classic motion-design tools solve this with 2.5D layers: flat layers placed in a 3D space, seen through a camera, drawn back to front among consecutive 3D layers, with a 2D layer between them ending the group. Real 3D objects (models, primitives) come later and must be able to join the same scene. The projection and the depth of field follow public textbook formulas: the pinhole perspective projection and the thin-lens circle of confusion.

## Decision

1. **Placement in depth** (format 0.16.0, additive with defaults, so older projects read unchanged). `Transform` gains `three_d` (bool), `position_z` (a fraction of the canvas width, positive away from the viewer), `rotation_x` (positive tips the top edge away), `rotation_y` (positive turns the right edge away) and `anchor` (the pivot of rotation and scale as fractions of the picture from its centre; moving it does not move the picture). The scene is in canvas pixels with the origin at the canvas centre, x right, y down, z away. A picture is scaled and stretched, turned in its plane (`rotation`), tilted (`rotation_x`), then turned (`rotation_y`), around its anchor. The vocabulary (ADR-0011) gains `transform.three_d` (not animatable) and the animatable `transform.position_z`, `transform.rotation_x`, `transform.rotation_y` and `transform.anchor`.
2. **Camera clips.** `ClipContent::Camera` (`"kind": "camera"`) sits on a video track and draws and sounds nothing. While it covers a time it is the view of its timeline's 3D layers; the enabled camera on the highest visible video track wins. Fields: `field_of_view` (degrees across the canvas width, 1 to 170, default 40), `position` and `position_z` (how far it moved forward from its framing distance, where the z = 0 plane exactly fills the frame), `point_of_interest` and `point_of_interest_z`, `orbit` (`[yaw, pitch]` degrees around the point of interest), `roll`, and the depth of field: `depth_of_field`, `focus_offset` (the plane in focus beyond the point of interest), `aperture` (0 to 1) and `max_blur` (0 to 0.2), all as fractions of the canvas width. Every number animates as `camera.<field>` (`Applies::Camera`). Validation reports `fosscut.model.camera` for values out of range and for effects, masks, transitions, animations or speed on a camera clip.
3. **Which camera.** A layer with `three_d` sees the timeline's camera, or the default camera (the defaults of item 2) when no camera clip covers the time. The default camera frames z = 0 exactly like 2D, so a 3D layer at rest renders identically to the 2D path. A 2D layer that is spatial anyway (`Transform::is_spatial`: depth, a tilt or turn, or an anchor) is seen through the default camera on its own; other 2D layers keep the 2D path unchanged.
4. **Rendering** (`fosscut_render::space`, `gpu/three_d.rs`, `shaders/projected.wgsl`). The planner gives every spatial layer a `LayerSpace` (camera, scene membership, view depth of its position point, depth-of-field blur) and sorts each run of consecutive 3D layers farthest first, keeping track order for equal depths; a 2D, adjustment or transition layer ends a run. Layers are flat and sorted whole, so intersecting layers are not split. A projected layer is a quad whose corners are in homogeneous clip coordinates with `w = depth / focal length`, so the rasterizer maps the picture with perspective and the 4x multisampled canvas antialiases its edges; it is sampled anisotropically where foreshortened. Effects and masks run in the layer's own space before projection. Motion blur samples the 3D placements (the camera's own motion is not sampled). Compound clips render flat with their own timeline's camera and can be placed in 3D. The depth of field blurs each layer by the thin-lens circle of confusion at its depth, capped by `max_blur`: the layer is drawn alone on a canvas-sized texture and blurred there with a Gaussian of half that radius, so its edges soften too. Transition sides are projected but not defocused. Preview, frame grabs and export share `plan_frame` (ADR-0008).
5. **A public camera for later 3D objects.** `space::camera_at(timeline, time)` gives the `Camera` (eye, target, roll, field of view, focus) with `view`, `projection`, `clip_from_world`, `view_depth`, `to_canvas` and `blur_radius`; `layer_model` maps a layer's plane into the scene. A future layer kind takes part by setting its `space` with `scene: true` and a `depth`, so `order_by_depth` sorts it among the planar layers, and draws its triangles with `clip_from_world` into a canvas-sized texture with a depth buffer of its own.
6. **Commands and reach.** `camera.add` (a `Camera` track above the video tracks unless a track is given; to the timeline's end by default) and `camera.set` (a merge patch that refuses fields with keyframes, `fosscut.clip.property_animated`), with `fosscut.camera.invalid` and `fosscut.camera.not_camera`; the transform and camera properties through `clip.set` and `keyframe.set`; 3D presets appended to `animation.set` (`flip_in`, `flip_out`, `swing_in`, `swing_out`, `tilt_in`, `tilt_out`, `fly_in_3d`, `fly_out_3d`, combos `turntable` and `sway_3d`). The CLI has `fosscut camera add|set` and `clip set --three-d --position-z --rotation-x --rotation-y --anchor`; MCP reaches them through `edit`; `capabilities` lists all of them.

## Consequences

- Ads with cards in depth, tilted maps, turning devices and camera moves are made from ordinary clips, keyframes and presets, editable by agents through the existing property paths.
- Projects without 3D placement render exactly as before; render cache keys change once (`CACHE_KEY_VERSION` 3).
- Sorting whole layers by one depth fails for layers that intersect or for a large tilted plane with layers standing on it near its far edge; scenes are arranged to avoid that until intersecting layers are split.
- Each defocused layer costs a canvas-sized texture, its mipmaps and a two-pass blur.
- The field of view is across the width and the camera keeps its framing distance, so a depth scales a picture by the same factor on every canvas aspect, and widening the field of view is a dolly zoom rather than a zoom.

## Alternatives considered

- **A depth buffer over all layers:** correct for opaque intersections, but transparent and antialiased pictures, blend modes and adjustment layers need ordered compositing.
- **An absolute camera position, as some motion-design tools store it:** a default camera would depend on the field of view and the canvas, and an omitted field could not mean "frames like 2D".
- **A vertical field of view:** a vertical version of a project made with `timeline.duplicate` would change the strength of its perspective.
- **Depth of field from a per-pixel depth map:** right for tilted planes, but a flat layer at one depth is what the scenes need now; it can come with real 3D objects.
- **Blurring inside the layer's texture:** cheaper, but the outline stays hard.

## Validation

- Model: sparse round trips, older transforms read unchanged, camera validation and keyframes (`crates/fosscut-model/tests/space.rs`).
- Contract samples for `camera.add` and `camera.set`; command tests in `crates/fosscut-core/tests/camera.rs`.
- GPU (`crates/fosscut-render/tests/three_d.rs`): 3D layers at rest and anchor-only layers are pixel-identical to the 2D path; depth sorting within runs; dolly and orbit; depth of field by distance with soft edges; masks follow the layer; preview matches export above 38 dB.
- Golden frames: a turned card, a sorted stack, a camera dolly and orbit, the depth of field, and a tilted card with a mask and an effect.
- A CLI end-to-end test of depth, a camera push and the 2D fallback.
- Before Accepted: a maintainer reviews an ad-style scene made by an agent, and the cost of the depth of field is measured at 1080p.
