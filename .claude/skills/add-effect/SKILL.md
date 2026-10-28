---
name: add-effect
description: Add or change a FossCut render effect, transition, mask, blend mode or color adjustment with its WGSL passes, parameter schema, animatable properties and golden-frame tests. Use for anything that changes how frames look.
---

# Add an effect

Governing decisions: [ADR-0011](../../../docs/adr/0011-effects-and-animatable-properties.md) (effects and property vocabulary), [ADR-0008](../../../docs/adr/0008-render-and-audio-engine.md) (render engine and color pipeline), [ADR-0015](../../../docs/adr/0015-clean-room-policy.md) (clean room).

## Clean-room first

Implement from public formulas, papers or standards (for example the W3C Compositing and Blending specification for blend modes, published color science for transfer functions). The research describes what CapCut offers and in which order it applies adjustments; it contains no shader code you may use. Never open CapCut shader files to "check" an implementation.

## Steps

1. **Define the type:** a namespaced id (`color.hsl`, `mask.ellipse`, `transition.push`), a version starting at 1, and the inputs (one for filters, two for transitions).
2. **Parameter schema:** for each parameter give name, type, default, range, unit, animatable flag and a one-line description written for an agent ("Feather width as a fraction of the mask's smaller side, 0 to 1"). Prefer normalized, resolution-independent units. Defaults must produce an identity result where that makes sense.
3. **Animatable paths:** animatable parameters appear as `effects.<instance id>.<param>` in the generated property vocabulary. Check the vocabulary test passes after adding them.
4. **WGSL passes** in `fosscut-render`: work in the linear working space defined by the color pipeline; respect premultiplied alpha; avoid branches on per-pixel data where possible; document any approximation. Validate shaders with `naga` in tests.
5. **Register** the effect in the registry so `capabilities` lists it with its schema.
6. **Placement in the pipeline:** state where it applies relative to other adjustments (for color tools, follow the order documented in the research, reimplemented) and why.
7. **Golden-frame tests** using the `golden-frame-tests` skill: default parameters, each parameter at its extremes, an animated case at two keyframes, and alpha edges for masks and transitions.
8. **Commands:** effects are added, changed, enabled and removed through the generic effect commands; only add a specialized command if agents need a simpler shape (use the `add-command` skill).
9. **Docs:** effect reference entry (what it does, parameters, examples of agent usage).
10. Run the `pre-pr-check` skill.

## Done when

The effect is discoverable through `capabilities`, renders identically in preview and export within tolerance, has golden tests at defaults and extremes, and needs no downloaded package to render.
