// A sample project for working on the interface in a plain browser (`pnpm dev`, outside
// the desktop app). Development builds only; edits are simulated and nothing is saved.

import { clip, entry, moment, track } from "../test/fixtures";
import { FakeEngine, standInFrame } from "./fake";
import { STAND_IN_VOICES, standInListing } from "./models";
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
    item("particles", "sparks", "Sparks", "Hot sparks fountaining up and falling back, cooling from white to orange.", { tags: ["fire"] }),
    item("particles", "embers", "Embers", "Glowing embers drifting up on warm air, already in flight on the first frame.", { tags: ["fire", "glow"] }),
    item("particles", "dust", "Dust in light", "Fine dust motes floating slowly all over the frame."),
    item("particles", "snow", "Snow", "Soft snowflakes falling with a light breeze.", { tags: ["winter"] }),
    item("particles", "confetti", "Confetti", "Tumbling pieces of confetti in party colours.", { tags: ["party"] }),
    item("particles", "bokeh", "Bokeh", "Large soft circles of out-of-focus light."),
    item("particles", "spark-gather", "Gathering sparks", "Sparks converging into the centre for a logo reveal.", { tags: ["logo"] }),
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
      assets: 8,
      fonts: ["Inter", "Lora"],
      text_presets: ["brand title"],
    },
  };
  engine.recent = ["/demo/Launch.fosscut"];
  // The project browser's cards, with stand-in frames, and a project in the trash.
  const summary = (name: string, path: string, seconds: number, bytes: number, daysAgo: number) => ({
    path,
    name,
    duration: moment(seconds),
    timelines: 2,
    assets: 6,
    bytes,
    modified_ms: Date.now() - daysAgo * 86_400_000,
    thumbnail: standInFrame(path, seconds * 235_200_000, 320, 180),
  });
  engine.summaries = [
    summary("Launch video (demo)", "/demo/Launch.fosscut", 24, 2_400_000, 0),
    summary("Interview cut", "/demo/Interview.fosscut", 312, 8_100_000, 1),
    summary("Shorts from the talk", "/demo/Talk shorts.fosscut", 58, 1_300_000, 3),
    summary("Holiday reel", "/demo/Holiday.fosscut", 96, 4_700_000, 9),
  ];
  engine.trashed = [{ id: "t-demo", name: "Old draft", original: "/demo/Old draft.fosscut", trashed_ms: Date.now() - 2 * 86_400_000 }];
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
          speed: { rate: 1.5, interpolate: "flow" },
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
      // Motion graphics (ADR-0036 to ADR-0038): a route drawn on with a dot riding it, a card
      // turned in 3D, sparks, and a camera pushing in.
      track("track_0070", "video", 5, [
        clip("clip_0071", "shape", 0, 6, {
          shape: "path",
          transform: { position: [0, 0.1] },
          content: {
            shape: "path",
            size: [0.7, 0.4],
            points: [{ at: [-1, 1] }, { at: [-0.2, -0.6], in: [-0.6, -0.4], out: [0.2, -0.8] }, { at: [1, -0.2] }],
            closed: false,
            fill: null,
            stroke: { color: "#FF7A45", width: 0.012, cap: "round", join: "round" },
            trim_end: 0,
          },
          animations: { in: { preset: "draw_on", duration: 1_411_200_000 } },
          keyframes: {
            "shape.trim_end": [
              { at: moment(0), offset: moment(0), value: 0, interpolation: { bezier: [0.42, 0, 0.58, 1] } },
              { at: moment(2), offset: moment(2), value: 1, interpolation: "linear" },
            ],
          },
        }),
        clip("clip_0074", "particles", 6, 12, {
          sprite: "spark",
          rate: 160,
          content: {
            emitter: { position: [0, 0.35] },
            rate: 160,
            life: 1.1,
            speed: 1.3,
            spread: 70,
            gravity: 2.2,
            size: 0.006,
            colors: ["#FFF6D8", "#FFB347", "#FF5A1F"],
            sprite: { kind: "spark", softness: 0.6 },
            seed: 11,
          },
        }),
      ]),
      track("track_0072", "video", 6, [
        clip("clip_0073", "shape", 0, 6, {
          shape: "ellipse",
          follow: { clip: "clip_0071", progress: 0, orient: false },
          content: { shape: "ellipse", size: [0.03, 0.053], fill: { color: "#FFFFFF" } },
          keyframes: {
            "follow.progress": [
              { at: moment(0), offset: moment(0), value: 0, interpolation: { bezier: [0.42, 0, 0.58, 1] } },
              { at: moment(2), offset: moment(2), value: 1, interpolation: "linear" },
            ],
          },
        }),
        clip("clip_0075", "image", 7, 13, {
          asset: "asset_0018",
          asset_name: "logo.svg",
          picture: [800, 400],
          transform: { three_d: true, position_z: 0.2, rotation_y: -25, anchor: [-0.5, 0] },
          animations: { in: { preset: "swing_in", duration: 705_600_000, delay: 235_200_000 } },
          effects: [{ id: "effect_0076", type: "light.glow", params: { radius: 0.06, intensity: 1.4 } }],
        }),
        clip("clip_0079", "model", 14, 20, {
          object: "text",
          text: "FossCut",
          transform: { rotation_y: -20, rotation_x: 10 },
          content: {
            object: { kind: "text", text: "FossCut", depth: 0.05 },
            material: { color: "#FF7A45", roughness: 0.35 },
            light: { azimuth: -40, elevation: 45 },
            floor: { shadow: 0.5 },
          },
          keyframes: {
            "model.light.azimuth": [
              { at: moment(14), offset: moment(0), value: -60, interpolation: "linear" },
              { at: moment(20), offset: moment(6), value: 40, interpolation: "linear" },
            ],
          },
        }),
      ]),
      track("track_0077", "video", 7, [
        clip("clip_0078", "camera", 0, 24, {
          text: "Camera",
          camera: { field_of_view: 45, depth_of_field: true, aperture: 0.2 },
          keyframes: {
            "camera.position_z": [
              { at: moment(7), offset: moment(7), value: 0, interpolation: { bezier: [0.42, 0, 0.58, 1] } },
              { at: moment(13), offset: moment(13), value: 0.3, interpolation: "linear" },
            ],
          },
        }),
      ], "Camera"),
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
  const light = (name: string, title: string, summary: string, params: ReturnType<typeof amount>[]) => ({
    type: name,
    version: 1,
    title,
    category: "light",
    summary,
    params,
  });
  engine.answers["capabilities"] = {
    effects: [
      light("light.glow", "Glow", "Soft light around the layer's bright parts that spreads past its edges.", [
        { ...amount("radius", "how far the light spreads"), min: 0, max: 0.5, default: 0.04 },
        { ...amount("intensity", "how bright the light is"), min: 0, max: 8, default: 1 },
        { ...amount("threshold", "only parts at least this bright glow"), min: 0, max: 1, default: 0 },
      ]),
      light("light.sweep", "Light sweep", "A band of light that moves across the layer, such as a sheen over a logo.", [
        { ...amount("position", "where the band is"), min: 0, max: 1, default: 0.5 },
        { ...amount("angle", "the direction the band moves"), min: -180, max: 180, default: 20 },
        { ...amount("width", "the band's width"), min: 0.01, max: 1, default: 0.15 },
      ]),
      light("light.streaks", "Light streaks", "Thin lines of light from the layer's highlights, fading with distance.", [
        { ...amount("length", "how far the streaks reach"), min: 0, max: 1, default: 0.3 },
        { ...amount("intensity", "how bright the streaks are"), min: 0, max: 8, default: 1 },
      ]),
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
        type: "key.subject",
        version: 1,
        title: "Remove background",
        category: "key",
        summary: "Keeps the subject and removes the background, from the clip's cached matte.",
        params: [
          { name: "source", kind: "choice", default: "person", choices: ["person", "subject"], animatable: false, meaning: "which cached matte" },
          { ...amount("threshold", "where the edge falls"), min: 0, max: 1, default: 0.5 },
          { ...amount("feather", "how soft the edge is"), min: 0, max: 0.1, default: 0.004 },
          { name: "invert", kind: "bool", default: false, animatable: false, meaning: "removes the subject instead" },
          { name: "background", kind: "choice", default: "none", choices: ["none", "color", "blur"], animatable: false, meaning: "what shows behind" },
          { name: "color", kind: "color", default: "#000000", animatable: false, meaning: "the background's color" },
          { ...amount("blur", "the background's blur"), min: 0, max: 0.2, default: 0.02 },
        ],
      },
      {
        type: "body.outline",
        version: 1,
        title: "Body outline",
        category: "body",
        summary: "Draws a line around the person, from the clip's cached pose.",
        params: [
          { name: "color", kind: "color", default: "#FFFFFF", animatable: false, meaning: "the line's color" },
          { ...amount("width", "the line's width"), min: 0, max: 0.1, default: 0.01 },
        ],
      },
      {
        type: "body.glow",
        version: 1,
        title: "Body glow",
        category: "body",
        summary: "Surrounds the person with colored light, from the clip's cached pose.",
        params: [
          { name: "color", kind: "color", default: "#66CCFF", animatable: false, meaning: "the light's color" },
          { ...amount("strength", "how bright the light is"), min: 0, max: 2, default: 1 },
        ],
      },
      {
        type: "body.sparkle",
        version: 1,
        title: "Body sparkle",
        category: "body",
        summary: "Scatters twinkling sparkles along the person, from the clip's cached pose.",
        params: [
          { name: "color", kind: "color", default: "#FFE08A", animatable: false, meaning: "the sparkles' color" },
          { ...amount("amount", "how many sparkles"), min: 0, max: 1, default: 0.5 },
        ],
      },
      {
        type: "motion.blur",
        version: 1,
        title: "Motion blur",
        category: "motion",
        summary: "Smears moving things along their motion, as a slower shutter would.",
        params: [{ ...amount("amount", "how long the shutter stays open"), min: 0, max: 1, default: 0.5 }],
      },
      {
        type: "detail.denoise",
        version: 1,
        title: "Reduce picture noise",
        category: "detail",
        summary: "Calms grain and sensor noise while keeping edges.",
        params: [{ ...amount("strength", "how much noise is removed"), min: 0, max: 1, default: 0.5 }],
      },
      {
        type: "color.deflicker",
        version: 1,
        title: "Deflicker",
        category: "color",
        summary: "Evens out brightness that pulses from frame to frame.",
        params: [{ ...amount("strength", "how much the brightness is evened"), min: 0, max: 1, default: 0.7 }],
      },
      {
        type: "voice.pitch",
        version: 1,
        title: "Voice pitch",
        category: "audio",
        summary: "Raises or lowers a voice without changing its speed.",
        params: [{ ...amount("semitones", "how far the pitch moves"), min: -12, max: 12, default: 0, animatable: false }],
      },
      {
        type: "voice.robot",
        version: 1,
        title: "Robot voice",
        category: "audio",
        summary: "A metallic, buzzing voice.",
        params: [{ ...amount("amount", "how robotic"), min: 0, max: 1, default: 0.8, animatable: false }],
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
    library: {
      models: [
        { name: "shipping-box", title: "Shipping box", description: "An orange cardboard box with rounded edges, casting a soft shadow on the footage below." },
        { name: "city-blocks", title: "City blocks", description: "A dark city of blocks of varied heights under a cool moonlight; grow it with keyframes." },
        { name: "podium", title: "Podium", description: "A low white round stage for a product, lit from above." },
        { name: "extruded-title", title: "Extruded title", description: "Bold white letters with depth and bevelled edges." },
        { name: "neon-ring", title: "Neon ring", description: "A thin ring that glows magenta by itself." },
        { name: "chrome-sphere", title: "Chrome sphere", description: "A polished metal ball with a soft shadow." },
      ],
    },
    voice_presets: [
      { name: "deep", title: "Deep", description: "A lower, fuller voice." },
      { name: "robot", title: "Robot", description: "A metallic, buzzing voice." },
      { name: "phone_call", title: "Phone call", description: "A narrow voice, as on a call." },
      { name: "stadium", title: "Stadium", description: "A voice in a big echoing space." },
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
      { preset: "blur_in", applies: "in", meaning: "comes into focus from a soft blur while fading in" },
      { preset: "blur_out", applies: "out", meaning: "blurs away while fading out" },
      { preset: "pop_in", applies: "in", meaning: "grows past full size and settles" },
      { preset: "rise_in", applies: "in", meaning: "rises a short way into place" },
      { preset: "swoop_in", applies: "in", meaning: "sweeps in on a curve" },
      { preset: "glow_in", applies: "in", meaning: "fades in with a pulse of light" },
      { preset: "glow_out", applies: "out", meaning: "flares with light while fading out" },
      { preset: "flip_in", applies: "in", meaning: "turns over to face the viewer (3D)" },
      { preset: "swing_in", applies: "in", meaning: "swings in like a door (3D)" },
      { preset: "fly_in_3d", applies: "in", meaning: "flies in from deep behind the canvas (3D)" },
      { preset: "draw_on", applies: "in", meaning: "draws a shape's outline on" },
      { preset: "draw_off", applies: "out", meaning: "erases a shape's outline" },
      { preset: "pulse", applies: "combo", meaning: "beats larger and back" },
      { preset: "turntable", applies: "combo", meaning: "turns around its vertical axis (3D)" },
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
      { id: "asset_0018", name: "logo.svg", kind: "vector", used_by: ["clip_0075"], video: { width: 800, height: 400 }, vector: { view_box: [0, 0, 200, 100], paths: 4 } },
      { id: "asset_0019", name: "Search bar", kind: "web", duration: moment(6), used_by: [], video: { width: 1080, height: 1920 } },
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
  // A proxy and a remuxed copy in use, one more proxy being made, and a newer release.
  engine.sources = { asset_0001: { proxy: "half" }, asset_0002: "remux" };
  engine.mediaWork = { proxies: { done: 1, total: 2 }, remux: { done: 0, total: 0 }, failed: 0 };
  engine.latestRelease = { version: "0.2.0", url: "https://github.com/fossworld/fosscut/releases" };
  // Local models (ADR-0033): the runtime, the pose model and English to Hungarian are
  // installed; any-subject mattes, voices and Hungarian to English ask to install first.
  engine.modelListing = standInListing(["onnxruntime", "mediapipe-pose-full", "opus-mt-en-hu"]);
  engine.installReads = 6;
  engine.analysisReads = 8;
  engine.voices = STAND_IN_VOICES;
  engine.captionPages = [
    { clip: "clip_0011", text: "Edit by talking" },
    { clip: "clip_0012", text: "to your agent" },
  ];
  engine.shortCandidates = [
    {
      span: { start: moment(1), end: moment(35.2) },
      text: "The moment we stopped editing by hand and started describing the cut, the whole thing took an afternoon instead of a week.",
      score: 0.91,
      seconds: 34.2,
      reasons: { words_per_second: 2.9, pause_share: 0.08, loudness_variation_lu: 5.2 },
      timeline_start: 1 * 705_600_000,
      timeline_end: Math.round(35.2 * 705_600_000),
    },
    {
      span: { start: moment(40), end: moment(62.5) },
      text: "Here is the trick nobody tells you: say what you want to keep, and let the agent find it.",
      score: 0.78,
      seconds: 22.5,
      reasons: { words_per_second: 3.3, pause_share: 0.05, loudness_variation_lu: 3.9 },
      timeline_start: 40 * 705_600_000,
      timeline_end: Math.round(62.5 * 705_600_000),
    },
    {
      span: { start: moment(80), end: moment(108) },
      text: "We tried it on a two hour interview and came back with twelve clips worth posting.",
      score: 0.64,
      seconds: 28,
      reasons: { words_per_second: 2.4, pause_share: 0.14, loudness_variation_lu: 4.4 },
      timeline_start: 80 * 705_600_000,
      timeline_end: 108 * 705_600_000,
    },
  ];
  engine.translations = { "Edit by talking": "Szerkesszen beszéddel", "to your agent": "az ügynökével" };
  return engine;
}
