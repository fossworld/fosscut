// A sample project for working on the interface in a plain browser (`pnpm dev`, outside
// the desktop app). Development builds only; edits are simulated and nothing is saved.

import { clip, entry, moment, track } from "../test/fixtures";
import { FakeEngine } from "./fake";
import type { TimelineView } from "./types";

export function demoEngine(): FakeEngine {
  const engine = new FakeEngine();
  engine.revision = 6;
  engine.opened = {
    bundle: "/demo/Launch.fosscut",
    info: {
      id: "project_demo",
      name: "Launch video (demo)",
      revision: 6,
      can_undo: true,
      can_redo: false,
      primary_timeline: "timeline_0000",
      timelines: [],
      assets: 3,
    },
  };
  engine.recent = ["/demo/Launch.fosscut"];
  engine.connected = [{ id: 1, name: "agent:claude-code", since_ms: Date.now() - 60_000 }];
  const timeline = (): TimelineView => ({
    timeline: "timeline_0000",
    name: "Timeline 1",
    revision: engine.revision,
    format: { width: 1920, height: 1080, rate: "30" },
    duration: moment(24),
    tracks: [
      track("track_0002", "video", 0, [
        clip("clip_0003", "video", 0, 8, {
          asset_name: "interview.mp4",
          transition: { type: "dissolve", duration: moment(1) },
          animations: { in: { preset: "fade", duration: 352_800_000 } },
        }),
        clip("clip_0006", "video", 8, 15, {
          asset_name: "b-roll.mp4",
          picture: [1920, 1080],
          masks: [{ id: "mask_0016", shape: "ellipse", size: [0.7, 0.8], feather: 0.3 }],
          effects: [
            { id: "effect_0013", type: "color.adjust", params: { temperature: 0.3, contrast: 0.2 } },
            { id: "effect_0014", type: "color.curves", params: { master: [[0, 0], [0.3, 0.22], [0.7, 0.78], [1, 1]] } },
          ],
          transform: { scale: 1.1 },
          blend: "normal",
          keyframes: {
            "transform.scale": [
              { at: moment(8), offset: moment(0), value: 1, interpolation: { bezier: [0.42, 0, 0.58, 1] } },
              { at: moment(15), offset: moment(7), value: 1.25, interpolation: "linear" },
            ],
          },
        }),
        clip("clip_0007", "video", 15, 24, { asset_name: "interview.mp4" }),
      ]),
      track("track_0004", "audio", 1, [
        clip("clip_0008", "audio", 0, 24, {
          asset_name: "music.m4a",
          volume: 0.6,
          audio: { fade_in: 705_600_000, fade_out: 2 * 705_600_000 },
        }),
      ]),
      track("track_0005", "text", 2, [clip("clip_0009", "text", 0.5, 3.5, { text: "Meet FossCut" })]),
      track("track_0010", "caption", 3, [
        clip("clip_0011", "caption", 1, 2.2, { text: "Edit by talking" }),
        clip("clip_0012", "caption", 2.2, 3.6, { text: "to your agent" }),
      ]),
    ],
  });
  engine.answers["project.info"] = () => ({ ...engine.opened?.info, revision: engine.revision });
  engine.answers["timeline.read"] = timeline;
  const amount = (name: string, meaning: string) => ({
    name,
    kind: "number" as const,
    default: 0,
    min: -1,
    max: 1,
    animatable: true,
    meaning,
  });
  const curve = (name: string) => ({ name, kind: "curve" as const, default: "identity", min: 0, max: 1, animatable: false, meaning: name });
  engine.answers["capabilities"] = {
    effects: [
      {
        type: "color.adjust",
        version: 1,
        title: "Adjust",
        category: "color",
        summary: "Exposure, white balance, contrast and saturation.",
        params: [
          { ...amount("exposure", "stops"), min: -4, max: 4 },
          amount("temperature", "cooler or warmer"),
          amount("tint", "greener or more magenta"),
          amount("contrast", "flatter or punchier"),
          amount("saturation", "less or more color"),
        ],
      },
      {
        type: "color.curves",
        version: 1,
        title: "Curves",
        category: "color",
        summary: "Tone curves.",
        params: [curve("master"), curve("red"), curve("green"), curve("blue")],
      },
      {
        type: "color.vignette",
        version: 1,
        title: "Vignette",
        category: "color",
        summary: "Darker edges.",
        params: [amount("amount", "edge darkness")],
      },
    ],
    looks: [
      { name: "warm", title: "Warm", description: "A gentle warm cast." },
      { name: "noir", title: "Noir", description: "Contrasty black and white." },
    ],
    transitions: [
      { type: "dissolve", title: "Dissolve", summary: "A cross-fade.", params: [] },
      {
        type: "push",
        title: "Push",
        summary: "The next clip pushes this one out.",
        params: [{ name: "direction", kind: "choice", default: "left", choices: ["left", "right", "up", "down"], animatable: false, meaning: "where it moves" }],
      },
    ],
    animations: [
      { preset: "fade", applies: "in or out", meaning: "from or to transparent" },
      { preset: "pop", applies: "in or out", meaning: "springs to size" },
      { preset: "pulse", applies: "combo", meaning: "beats larger and back" },
    ],
  };
  engine.answers["media.list"] = {
    assets: [
      { id: "asset_0001", name: "interview.mp4", kind: "video" },
      { id: "asset_0015", name: "film.cube", kind: "lut" },
    ],
  };
  engine.answers["history.list"] = {
    entries: [
      entry(6, "agent:claude-code", ["captions.add"]),
      entry(5, "agent:claude-code", ["text.add"]),
      entry(4, "human", ["clip.split"]),
      entry(3, "agent:claude-code", ["clip.add", "clip.add", "clip.add"]),
      entry(2, "agent:claude-code", ["media.add"]),
    ].map((e) => ({ ...e, at_ms: Date.now() - (7 - e.seq) * 45_000 })),
  };
  return engine;
}
