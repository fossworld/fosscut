// Preferences for the stand-in engines (tests and `pnpm dev` in a plain browser): the
// defaults of `fosscut_engine::preferences` and its refusals, worded as it words them,
// held in memory. The desktop app reads and writes the real file instead.

import type { ApiError, Preferences, PreferencesView } from "./types";

/** The defaults, as `fosscut_engine::preferences::Preferences::default()` has them. */
export function defaultPreferences(): Preferences {
  return {
    version: 1,
    locations: { projects: null, recordings: null, exports: null },
    cache: { max_age_days: 30, limit_gb: 20 },
    editing: { frame_step: 10, nudge_pixels: 10, image_duration: 5, loudness_target: -14 },
    timeline: { scrub_audio: true, linked: true, frame_rate: "30", time_display: "timecode" },
    export: { sound: true, sound_name: "ding" },
    performance: {
      hardware_decoding: "auto",
      hardware_encoding: true,
      proxies: "off",
      proxy_size: "half",
      render_cache_gb: 10,
      auto_render: true,
      auto_remux: true,
    },
    language: "en",
    updates: "notify",
    notifications: true,
    shortcuts: { set: "fosscut", overrides: {} },
  };
}

const LANGUAGES = ["en", "hu"];
const SETS = ["fosscut", "classic"];

function invalid(message: string): ApiError {
  return { code: "fosscut.preferences.invalid", message };
}

const RANGES: Record<string, [number, number]> = {
  "cache.max_age_days": [0, 3650],
  "cache.limit_gb": [0, 100_000],
  "editing.frame_step": [1, 1000],
  "editing.nudge_pixels": [1, 1000],
  "editing.image_duration": [0.1, 3600],
  "editing.loudness_target": [-40, -5],
  "performance.render_cache_gb": [0, 10_000],
};

const CHOICES: Record<string, string[]> = {
  "timeline.time_display": ["timecode", "frames", "seconds"],
  "performance.hardware_decoding": ["auto", "off"],
  "performance.proxies": ["off", "auto", "on"],
  "performance.proxy_size": ["half", "quarter"],
  updates: ["off", "notify", "check"],
};

/** The frames per second of a rate as `fosscut export --fps` takes it, or null. */
function framesPerSecond(text: string): number | null {
  const [num, den = "1"] = text.split("/");
  const value = Number(num) / Number(den);
  return Number.isFinite(value) && value > 0 ? value : null;
}

/** Throws the engine's refusal of `preferences`, naming the first value out of range. */
export function validatePreferences(preferences: Preferences, sounds: string[]): void {
  const at = (key: string): unknown =>
    key.split(".").reduce<unknown>((value, part) => (value as Record<string, unknown> | undefined)?.[part], preferences);
  for (const [key, [low, high]] of Object.entries(RANGES)) {
    const value = at(key);
    if (typeof value !== "number" || !Number.isFinite(value) || value < low || value > high) {
      throw invalid(`${key} must be from ${low} to ${high}, not ${String(value)}`);
    }
  }
  for (const [key, choices] of Object.entries(CHOICES)) {
    const value = at(key);
    if (typeof value !== "string" || !choices.includes(value)) {
      throw invalid(`${key}: unknown variant \`${String(value)}\`, expected one of ${choices.map((c) => `\`${c}\``).join(", ")}`);
    }
  }
  const rate = framesPerSecond(preferences.timeline.frame_rate);
  if (rate === null) throw invalid(`timeline.frame_rate: ${preferences.timeline.frame_rate} is not a frame rate`);
  if (rate < 12 || rate > 120) throw invalid(`timeline.frame_rate must be from 12 to 120, not ${rate}`);
  if (!LANGUAGES.includes(preferences.language)) {
    throw invalid(`language ${preferences.language} is not one of ${LANGUAGES.join(", ")}`);
  }
  if (!SETS.includes(preferences.shortcuts.set)) {
    throw invalid(`shortcuts.set ${preferences.shortcuts.set} is not one of ${SETS.join(", ")}`);
  }
  if (!sounds.includes(preferences.export.sound_name)) {
    throw invalid(`export.sound_name ${preferences.export.sound_name} is not a library sound`);
  }
  for (const key of ["projects", "recordings", "exports"] as const) {
    const place = preferences.locations[key];
    if (place !== null && !place.startsWith("/") && !/^[A-Za-z]:[\\/]/.test(place)) {
      throw invalid(`locations.${key} must be an absolute path`);
    }
  }
}

/** Preferences kept in memory, set and reset as the desktop's commands do. */
export class PreferenceStandIn {
  current: Preferences = defaultPreferences();

  constructor(
    private readonly sounds: string[] = ["ding", "pop", "whoosh"],
    readonly path = "/stand-in/preferences.json",
  ) {}

  view(): PreferencesView {
    return {
      path: this.path,
      preferences: structuredClone(this.current),
      defaults: defaultPreferences(),
      languages: [...LANGUAGES],
      shortcut_sets: [...SETS],
    };
  }

  /** Sets the dotted `key` to `value` (null for its default), or throws the refusal. */
  set(key: string, value: unknown): PreferencesView {
    if (!key || key === "version") throw invalid("name a preference, for example editing.frame_step");
    const next = structuredClone(this.current) as unknown as Record<string, unknown>;
    const overrides = "shortcuts.overrides.";
    if (key.startsWith(overrides)) {
      const action = key.slice(overrides.length);
      const shortcuts = next.shortcuts as Preferences["shortcuts"];
      if (value === null) delete shortcuts.overrides[action];
      else if (typeof value === "string") shortcuts.overrides[action] = value;
      else throw invalid(`${key}: keys are text such as "Shift+S", not ${JSON.stringify(value)}`);
    } else {
      const parts = key.split(".");
      const last = parts.pop() as string;
      let target = next;
      let defaults = defaultPreferences() as unknown as Record<string, unknown>;
      for (const part of parts) {
        const child = target[part];
        if (typeof child !== "object" || child === null) throw unknown(key);
        target = child as Record<string, unknown>;
        defaults = defaults[part] as Record<string, unknown>;
      }
      if (!Object.hasOwn(target, last)) throw unknown(key);
      const before = target[last];
      let given = value === null ? structuredClone(defaults[last]) : value;
      if (typeof before === "string" && typeof given === "number") given = String(given);
      const kind = (v: unknown) => (v === null ? "null" : typeof v);
      if (before !== null && given !== null && kind(before) !== kind(given)) {
        throw invalid(`${key}: invalid type: ${kind(given)} ${JSON.stringify(given)}, expected ${kind(before)}`);
      }
      target[last] = given;
    }
    validatePreferences(next as unknown as Preferences, this.sounds);
    this.current = next as unknown as Preferences;
    return this.view();
  }
}

function unknown(key: string): ApiError {
  return invalid(`there is no preference ${key}; \`fosscut preferences\` lists them`);
}
