// A sample project for working on the interface in a plain browser (`pnpm dev`, outside
// the desktop app). Development builds only; edits are simulated and nothing is saved.

import { clip, entry, moment, track } from "../test/fixtures";
import { FakeEngine } from "./fake";
import type { LibraryItem, LibraryKind, TimelineView } from "./types";

/** A sample of the built-in library, as `library_list` answers. */
function library(): LibraryItem[] {
  const item = (kind: LibraryKind, name: string, title: string, description: string, extra: Partial<LibraryItem> = {}) => ({
    name,
    kind,
    title,
    description,
    tags: [],
    commercial: true,
    ...extra,
  });
  return [
    item("shape", "rounded-rectangle", "Rounded rectangle", "A rectangle with soft corners."),
    item("shape", "circle", "Circle", "A circle."),
    item("shape", "star", "Star", "A five-pointed star."),
    item("shape", "arrow", "Arrow", "A bold arrow pointing right."),
    item("sticker", "badge-new", "New badge", "A red pill with NEW in white capitals.", { tags: ["promo"] }),
    item("sticker", "badge-sale", "Sale badge", "A yellow burst with SALE in black capitals.", { tags: ["promo", "discount"] }),
    item("sticker", "subscribe", "Subscribe button", "A red button reading SUBSCRIBE, for calls to action."),
    item("sticker", "speech-bubble", "Speech bubble", "A white speech bubble with a short line in it."),
    item("background", "sunset", "Sunset", "Warm orange fading to pink from top to bottom."),
    item("background", "night", "Night", "A purple glow fading to black at the edges."),
    item("template", "vertical-promo", "vertical-promo", "A 9:16 promo: a headline, your shots with dissolves and an end card."),
    item("template", "quote-card", "quote-card", "A quotation typed out on a dark gradient, with its author."),
    item("title", "lower-third", "lower-third", "A name and a role at the lower left."),
    item("title", "headline", "headline", "A big centred title rising in word by word."),
    item("text_style", "neon", "neon", "Pale pink letters glowing magenta."),
    item("text_style", "highlight", "highlight", "Black letters on a bright yellow box."),
    item("caption_style", "karaoke", "karaoke", "Words light up as they are spoken."),
    item("caption_style", "highlight", "highlight", "The spoken word in a bright box."),
    item("look", "teal_orange", "Teal and orange", "Cool shadows and warm skin tones."),
    item("look", "noir", "Noir", "Contrasty black and white."),
    item("sound", "whoosh", "Whoosh", "A quick rush of air for transitions.", { seconds: 0.6, tags: ["transition"] }),
    item("sound", "pop", "Pop", "A short bright pop for things appearing.", { seconds: 0.25 }),
    item("sound", "ding", "Ding", "A clear bell for a reveal.", { seconds: 1.4 }),
    item("music", "lofi-beat", "Lo-fi beat", "A relaxed loop under talking.", { seconds: 16, bpm: 80 }),
    item("music", "upbeat-pop", "Upbeat pop", "A bright, driving loop for promos.", { seconds: 8, bpm: 120 }),
  ];
}

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
      timelines: [
        {
          id: "timeline_0000",
          name: "Launch video",
          format: { width: 1920, height: 1080, rate: "30" },
          duration: moment(24),
          tracks: 4,
          clips: 8,
        },
        {
          id: "timeline_0030",
          name: "Vertical cut",
          format: { width: 1080, height: 1920, rate: "30" },
          duration: moment(24),
          tracks: 4,
          clips: 8,
        },
        {
          id: "timeline_0051",
          name: "Multicam 1",
          role: "compound",
          format: { width: 1920, height: 1080, rate: "30" },
          duration: moment(10),
          tracks: 2,
          clips: 2,
        },
      ],
      assets: 6,
      fonts: ["Inter", "Lora"],
      text_presets: ["brand title"],
    },
  };
  engine.recent = ["/demo/Launch.fosscut"];
  engine.connected = [{ id: 1, name: "agent:claude-code", since_ms: Date.now() - 60_000 }];
  const multicam = (id: string, start: number, end: number, angle: string) =>
    clip(id, "timeline", start, end, {
      timeline: "timeline_0051",
      timeline_name: "Multicam 1",
      angle,
      angles: [
        { id: "track_0053", name: "Angle 1: interview.mp4" },
        { id: "track_0052", name: "Angle 2: b-roll.mp4" },
      ],
    });
  const timeline = (): TimelineView => ({
    timeline: "timeline_0000",
    name: "Timeline 1",
    revision: engine.revision,
    format: { width: 1920, height: 1080, rate: "30" },
    duration: moment(24),
    markers: [
      { id: "marker_0020", at: moment(4), name: "beat" },
      { id: "marker_0021", at: moment(15), name: "drop", color: "#ffcc33" },
    ],
    tracks: [
      track("track_0002", "video", 0, [
        clip("clip_0003", "video", 0, 8, {
          asset: "asset_0001",
          asset_name: "interview.mp4",
          group: "group_0022",
          transition: { type: "dissolve", duration: moment(1) },
          animations: { in: { preset: "fade", duration: 352_800_000 } },
        }),
        clip("clip_0006", "video", 8, 15, {
          asset: "asset_0002",
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
        clip("clip_0007", "video", 15, 24, {
          asset: "asset_0001",
          asset_name: "interview.mp4",
          speed: { rate: 1.5, interpolate: "blend" },
          source_used: moment(13.5),
        }),
      ]),
      track("track_0004", "audio", 1, [
        clip("clip_0008", "audio", 0, 24, {
          asset: "asset_0003",
          asset_name: "music.m4a",
          volume: 0.6,
          audio: { fade_in: 705_600_000, fade_out: 2 * 705_600_000 },
        }),
      ]),
      track("track_0005", "text", 2, [
        clip("clip_0009", "text", 0.5, 3.5, {
          text: "Meet FossCut",
          group: "group_0022",
          style: { size: 0.1, color: "#FFFFFF", bold: true, case: "upper", shadow: { enabled: true, blur: 0.15 } },
          reveal: { unit: "word", motion: "rise", duration: moment(1) },
        }),
      ]),
      track("track_0010", "caption", 3, [
        clip("clip_0011", "caption", 1, 2.2, { text: "Edit by talking" }),
        clip("clip_0012", "caption", 2.2, 3.6, { text: "to your agent" }),
      ]),
      track("track_0040", "video", 4, [
        // A multicam clip cut once: the interview angle, then the b-roll angle.
        multicam("clip_0055", 0, 6, "track_0053"),
        multicam("clip_0056", 6, 10, "track_0052"),
        clip("clip_0041", "shape", 16, 20, {
          shape: "rectangle",
          text: "SALE",
          transform: { position: [0.3, -0.3] },
          content: {
            shape: "rectangle",
            size: [0.18, 0.08],
            roundness: 1,
            fill: { color: "#FF3B5C", gradient: { to: "#FF9A3C", angle: 90 } },
            label: { text: "SALE", style: { size: 0.045, color: "#FFFFFF", bold: true } },
          },
        }),
      ]),
    ],
  });
  engine.library = library();
  engine.captureDevices = [
    { kind: "camera", name: "FaceTime HD Camera", format: "avfoundation", input: "0:" },
    { kind: "screen", name: "Capture screen 0", format: "avfoundation", input: "1:" },
    { kind: "microphone", name: "MacBook Pro Microphone", format: "avfoundation", input: ":0" },
    { kind: "microphone", name: "USB Audio Interface", format: "avfoundation", input: ":1" },
  ];
  // Measured edits: two shot cuts in a clip, and a tracked region lost for half a second.
  engine.sceneCuts = [moment(10).flicks, moment(12.5).flicks];
  engine.trackLost = [{ start: moment(10).flicks, end: moment(10.5).flicks }];
  engine.exportListing = {
    presets: [
      { name: "mp4-h264", description: "H.264 and AAC in MP4: plays everywhere (hardware encoder)", extension: "mp4", available: true },
      { name: "mp4-hevc", description: "HEVC and AAC in MP4: smaller files (hardware encoder)", extension: "mp4", available: true },
      { name: "mp4-av1", description: "AV1 and AAC in MP4: small files, modern players", extension: "mp4", available: false },
      { name: "mov-prores", description: "ProRes 422 and AAC in QuickTime: for other editors", extension: "mov", available: true },
      { name: "webm-vp9", description: "VP9 and Opus in WebM: for the web", extension: "webm", available: true },
      { name: "m4a-aac", description: "AAC audio only", extension: "m4a", available: true },
    ],
    resolutions: [
      { name: "480p", pixels: 480 },
      { name: "720p", pixels: 720 },
      { name: "1080p", pixels: 1080 },
      { name: "2k", pixels: 1440 },
      { name: "4k", pixels: 2160 },
      { name: "8k", pixels: 4320 },
    ],
  };
  engine.exportPicks = "/demo/Launch video (demo).mp4";
  engine.exportFrames = 720;
  engine.exportStep = 24;
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
      {
        type: "audio.denoise",
        version: 1,
        title: "Reduce noise",
        category: "audio",
        summary: "Reduces steady background noise.",
        params: [{ ...amount("amount", "how much noise is removed"), min: 0, max: 1, default: 0.7, animatable: false }],
      },
      {
        type: "audio.vocals",
        version: 1,
        title: "Isolate or remove voice",
        category: "audio",
        summary: "Keeps or removes the centred voice of a stereo mix.",
        params: [
          {
            name: "mode",
            kind: "choice",
            default: "isolate",
            choices: ["isolate", "remove"],
            animatable: false,
            meaning: "isolate keeps the voice; remove keeps the rest",
          },
        ],
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
    speed_presets: [
      { preset: "montage", meaning: "alternates fast and slow" },
      { preset: "hero", meaning: "slow motion through the middle" },
      { preset: "bullet", meaning: "very slow through the middle" },
    ],
    animations: [
      { preset: "fade", applies: "in or out", meaning: "from or to transparent" },
      { preset: "pop", applies: "in or out", meaning: "springs to size" },
      { preset: "pulse", applies: "combo", meaning: "beats larger and back" },
    ],
    text: {
      fonts: ["Inter"],
      presets: [
        { name: "outline", description: "Bold white letters with a black outline." },
        { name: "neon", description: "Pale pink letters glowing magenta." },
        { name: "highlight", description: "Black letters on a bright yellow box." },
      ],
      templates: [
        { name: "lower-third", description: "A name and a role at the lower left.", lines: ["Alex Morgan", "Founder"] },
        { name: "headline", description: "A big centred title rising in word by word.", lines: ["Big news", "What changed"] },
        { name: "end-card", description: "A closing message with a call to action.", lines: ["Thanks for watching", "Follow for more"] },
      ],
      reveal: { units: ["letter", "word", "line"], motions: ["appear", "fade", "rise"] },
      cases: ["as_written", "upper", "lower", "title"],
    },
  };
  engine.answers["media.list"] = {
    assets: [
      { id: "asset_0001", name: "interview.mp4", kind: "video", duration: moment(16), used_by: ["clip_0003"], folder: "Footage", audio: { channels: 2 } },
      { id: "asset_0002", name: "b-roll.mp4", kind: "video", duration: moment(7), used_by: ["clip_0006"], folder: "Footage/B-roll", audio: null },
      { id: "asset_0003", name: "music.m4a", kind: "audio", duration: moment(24), used_by: ["clip_0008"] },
      { id: "asset_0004", name: "old take.mp4", kind: "video", duration: moment(12), used_by: [], missing: true },
      { id: "asset_0015", name: "film.cube", kind: "lut", used_by: [] },
      { id: "asset_0017", name: "Lora.ttf", kind: "font", used_by: [], families: ["Lora"] },
    ],
    folders: ["Footage", "Footage/B-roll"],
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
