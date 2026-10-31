import { Database, Download, FolderOpen, Gauge, Keyboard, Pencil, Settings as Gear, SlidersHorizontal, Timer, X, type LucideIcon } from "lucide-react";
import { useEffect, useState } from "react";

import { CACHE_KINDS } from "../engine/cache";
import type { CacheKind, Preferences } from "../engine/types";
import { LANGUAGES, t } from "../i18n";
import { MAC, keyName, keystroke, pressed } from "../keyboard";
import { ACTIONS, type ActionId, GROUPS, SET_NAMES, contextOf, isSetName, shownKeys } from "../shortcuts";
import { type Editor, type EditorState, SETTINGS_SECTIONS, type SettingsSection, preferencesOf } from "./editor";
import {
  type ShortcutDraft,
  changedActions,
  clashes,
  cleared,
  draftOf,
  isDirty,
  recorded,
  recording,
  reset,
  toSave,
  withSet,
} from "./shortcutDraft";
import { readableSize } from "./Start";
import { THEME_CHOICES, setTheme, useThemeChoice } from "../theme/theme";
import { Glyph, IconButton, Segmented } from "../ui/controls";

/** The icon of each section of Settings. */
const SECTION_ICONS: Record<SettingsSection, LucideIcon> = {
  general: SlidersHorizontal,
  locations: FolderOpen,
  editing: Pencil,
  timeline: Timer,
  export: Download,
  performance: Gauge,
  cache: Database,
  shortcuts: Keyboard,
};

/** Each language's name in itself, as language choices show them. */
export const LANGUAGE_NAMES: Record<string, string> = { en: "English", hu: "Magyar" };

/** Frame rates offered for new projects; any rate from 12 to 120 may be typed. */
const RATES = ["23.976", "24", "25", "29.97", "30", "50", "59.94", "60"];

/** The value at a dotted preference key. */
export function valueAt(preferences: Preferences, key: string): unknown {
  return key.split(".").reduce<unknown>((value, part) => (value as Record<string, unknown> | undefined)?.[part], preferences);
}

interface Props {
  editor: Editor;
  state: EditorState;
}

/** What every control of a section needs. */
interface Control extends Props {
  /** The dotted preference key. */
  k: string;
  label: string;
  help?: string;
}

/** The engine's refusal of the last change of `k`, as it worded it. */
function Refusal({ state, k }: { state: EditorState; k: string }) {
  const error = state.settings.errors[k];
  return error ? (
    <p className="problem" role="alert">
      {error}
    </p>
  ) : null;
}

/** A switch. */
export function Toggle({ editor, state, k, label, help }: Control) {
  const on = valueAt(preferencesOf(state), k) === true;
  return (
    <div className="setting">
      <label className="toggle" title={help}>
        <input type="checkbox" checked={on} onChange={(event) => void editor.setPreference(k, event.target.checked)} />
        <span>{label}</span>
      </label>
      <Refusal state={state} k={k} />
    </div>
  );
}

/** A choice among values, each with its label. */
export function Choice({ editor, state, k, label, help, options }: Control & { options: [string, string][] }) {
  const value = String(valueAt(preferencesOf(state), k) ?? "");
  return (
    <div className="setting">
      <label className="choice" title={help}>
        <span>{label}</span>
        <select value={value} onChange={(event) => void editor.setPreference(k, event.target.value)}>
          {options.map(([option, name]) => (
            <option key={option} value={option}>
              {name}
            </option>
          ))}
        </select>
      </label>
      <Refusal state={state} k={k} />
    </div>
  );
}

/**
 * A number or a short text, saved when the field is left or Enter is pressed. A value the
 * engine refuses stays in the field, with the refusal below it.
 */
export function Entry({
  editor,
  state,
  k,
  label,
  help,
  unit,
  numeric = true,
  step,
  list,
}: Control & { unit?: string; numeric?: boolean; step?: number; list?: string }) {
  const saved = String(valueAt(preferencesOf(state), k) ?? "");
  const [draft, setDraft] = useState<string | null>(null);
  const [local, setLocal] = useState<string | null>(null);
  const commit = () => {
    if (draft === null || draft === saved) {
      setDraft(null);
      return;
    }
    const value = numeric ? Number(draft.replace(",", ".")) : draft.trim();
    if (numeric && (draft.trim() === "" || !Number.isFinite(value))) {
      setLocal(t("settings.number"));
      return;
    }
    setLocal(null);
    void editor.setPreference(k, value).then((done) => done && setDraft(null));
  };
  return (
    <div className="setting">
      <label className="choice" title={help}>
        <span>{label}</span>
        <input
          className="short"
          inputMode={numeric ? "decimal" : "text"}
          type="text"
          step={step}
          list={list}
          value={draft ?? saved}
          aria-invalid={Boolean(local || state.settings.errors[k])}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
            if (event.key === "Escape") {
              event.stopPropagation();
              setDraft(null);
              setLocal(null);
            }
          }}
        />
        {unit && <span className="unit">{unit}</span>}
      </label>
      {local ? (
        <p className="problem" role="alert">
          {local}
        </p>
      ) : (
        <Refusal state={state} k={k} />
      )}
    </div>
  );
}

/** The theme of this computer's window: dark, light or the system's. */
function Appearance() {
  const theme = useThemeChoice();
  return (
    <div className="setting">
      <div className="choice">
        <span>{t("settings.general.theme")}</span>
        <Segmented
          label={t("settings.general.theme")}
          value={theme}
          onChange={setTheme}
          options={THEME_CHOICES.map((choice) => ({ id: choice, label: t(`app.theme.${choice}`) }))}
        />
      </div>
      <p className="muted hint">{t("settings.general.theme.help")}</p>
    </div>
  );
}

export function GeneralSection({ editor, state }: Props) {
  const view = state.preferences;
  return (
    <>
      <Appearance />
      <Choice
        editor={editor}
        state={state}
        k="language"
        label={t("settings.general.language")}
        options={LANGUAGES.map((code) => [code, LANGUAGE_NAMES[code] ?? code])}
      />
      <Choice
        editor={editor}
        state={state}
        k="updates"
        label={t("settings.general.updates")}
        help={t("settings.general.updates.help")}
        options={[
          ["off", t("settings.general.updates.off")],
          ["notify", t("settings.general.updates.notify")],
          ["check", t("settings.general.updates.check")],
        ]}
      />
      <div className="buttons">
        <button disabled={preferencesOf(state).updates === "off"} onClick={() => void editor.checkUpdates()}>
          {t("settings.general.checkNow")}
        </button>
        {state.update?.newer && <span className="muted">{t("app.update.available", { version: state.update.latest ?? "", current: state.update.current ?? "" })}</span>}
      </div>
      <Toggle
        editor={editor}
        state={state}
        k="notifications"
        label={t("settings.general.notifications")}
        help={t("settings.general.notifications.help")}
      />
      {view && (
        <p className="muted hint" title={view.path}>
          {t("settings.general.file", { path: view.path })}
        </p>
      )}
      {view?.problem && (
        <p className="problem" role="alert">
          {t("settings.general.problem", { problem: view.problem })}
        </p>
      )}
    </>
  );
}

const PLACES = ["projects", "recordings", "exports"] as const;

export function LocationsSection({ editor, state }: Props) {
  const locations = preferencesOf(state).locations;
  return (
    <>
      {PLACES.map((place) => {
        const k = `locations.${place}`;
        const folder = locations[place];
        return (
          <div className="setting location" key={place}>
            <div className="choice">
              <span>{t(`settings.locations.${place}`)}</span>
              <span className="path" title={folder ?? undefined}>
                {folder ?? t(`settings.locations.${place}.default`)}
              </span>
            </div>
            <div className="buttons">
              <button onClick={() => void editor.pickLocation(place)}>{t("settings.locations.choose")}</button>
              <button disabled={folder === null} onClick={() => void editor.resetPreference(k)}>
                {t("settings.locations.useDefault")}
              </button>
            </div>
            <Refusal state={state} k={k} />
          </div>
        );
      })}
    </>
  );
}

export function EditingSection({ editor, state }: Props) {
  const control = { editor, state };
  return (
    <>
      <Entry {...control} k="editing.frame_step" label={t("settings.editing.frameStep")} help={t("settings.editing.frameStep.help")} unit={t("settings.unit.frames")} />
      <Entry {...control} k="editing.nudge_pixels" label={t("settings.editing.nudge")} help={t("settings.editing.nudge.help")} unit="px" />
      <Entry {...control} k="editing.image_duration" label={t("settings.editing.imageDuration")} help={t("settings.editing.imageDuration.help")} unit={t("settings.unit.seconds")} />
      <Entry {...control} k="editing.loudness_target" label={t("settings.editing.loudness")} help={t("settings.editing.loudness.help")} unit="LUFS" />
    </>
  );
}

export function TimelineSection({ editor, state }: Props) {
  const control = { editor, state };
  return (
    <>
      <Toggle {...control} k="timeline.scrub_audio" label={t("settings.timeline.scrubAudio")} help={t("settings.timeline.scrubAudio.help")} />
      <Toggle {...control} k="timeline.linked" label={t("settings.timeline.linked")} help={t("settings.timeline.linked.help")} />
      <Entry {...control} k="timeline.frame_rate" numeric={false} list="settings-rates" label={t("settings.timeline.frameRate")} help={t("settings.timeline.frameRate.help")} unit={t("settings.unit.fps")} />
      <datalist id="settings-rates">
        {RATES.map((rate) => (
          <option key={rate} value={rate} />
        ))}
      </datalist>
      <Choice
        {...control}
        k="timeline.time_display"
        label={t("settings.timeline.timeDisplay")}
        options={[
          ["timecode", t("settings.timeline.timeDisplay.timecode")],
          ["frames", t("settings.timeline.timeDisplay.frames")],
          ["seconds", t("settings.timeline.timeDisplay.seconds")],
        ]}
      />
    </>
  );
}

export function ExportSection({ editor, state }: Props) {
  const preferences = preferencesOf(state);
  const sounds = state.settings.sounds ?? [];
  const chosen = preferences.export.sound_name;
  const options: [string, string][] = sounds.map((sound) => [sound.name, sound.title]);
  if (!options.some(([name]) => name === chosen)) options.unshift([chosen, chosen]);
  return (
    <>
      <Toggle editor={editor} state={state} k="export.sound" label={t("settings.export.sound")} help={t("settings.export.sound.help")} />
      <div className="setting">
        <label className="choice">
          <span>{t("settings.export.soundName")}</span>
          <select
            value={chosen}
            disabled={!preferences.export.sound}
            onChange={(event) => void editor.setPreference("export.sound_name", event.target.value)}
          >
            {options.map(([name, title]) => (
              <option key={name} value={name}>
                {title}
              </option>
            ))}
          </select>
          <button onClick={() => void editor.playSound(chosen)} title={t("settings.export.play.help")}>
            {t("settings.export.play")}
          </button>
        </label>
        <Refusal state={state} k="export.sound_name" />
      </div>
    </>
  );
}

export function PerformanceSection({ editor, state }: Props) {
  const control = { editor, state };
  const preferences = preferencesOf(state);
  return (
    <>
      <Choice
        {...control}
        k="performance.hardware_decoding"
        label={t("settings.performance.decoding")}
        help={t("settings.performance.decoding.help")}
        options={[
          ["auto", t("settings.performance.decoding.auto")],
          ["off", t("settings.performance.decoding.off")],
        ]}
      />
      <Toggle {...control} k="performance.hardware_encoding" label={t("settings.performance.encoding")} help={t("settings.performance.encoding.help")} />
      <Choice
        {...control}
        k="performance.proxies"
        label={t("settings.performance.proxies")}
        help={t("settings.performance.proxies.help")}
        options={[
          ["off", t("settings.performance.proxies.off")],
          ["auto", t("settings.performance.proxies.auto")],
          ["on", t("settings.performance.proxies.on")],
        ]}
      />
      {preferences.performance.proxies !== "off" && (
        <Choice
          {...control}
          k="performance.proxy_size"
          label={t("settings.performance.proxySize")}
          options={[
            ["half", t("settings.performance.proxySize.half")],
            ["quarter", t("settings.performance.proxySize.quarter")],
          ]}
        />
      )}
      <Entry {...control} k="performance.render_cache_gb" label={t("settings.performance.renderCache")} help={t("settings.performance.renderCache.help")} unit="GB" />
      <Toggle {...control} k="performance.auto_render" label={t("settings.performance.autoRender")} help={t("settings.performance.autoRender.help")} />
      <Toggle {...control} k="performance.auto_remux" label={t("settings.performance.autoRemux")} help={t("settings.performance.autoRemux.help")} />
    </>
  );
}

/** The name of a cache. */
export function cacheName(kind: CacheKind): string {
  return t(`settings.cache.kind.${kind}`);
}

export function CacheSection({ editor, state }: Props) {
  const control = { editor, state };
  const usage = state.settings.cache;
  const sizes = new Map((usage?.caches ?? []).map((cache) => [cache.kind, cache]));
  const total = (usage?.caches ?? []).reduce((sum, cache) => sum + (cache.bytes ?? 0), 0);
  return (
    <>
      <Entry {...control} k="cache.max_age_days" label={t("settings.cache.maxAge")} help={t("settings.cache.maxAge.help")} unit={t("settings.unit.days")} />
      <Entry {...control} k="cache.limit_gb" label={t("settings.cache.limit")} help={t("settings.cache.limit.help")} unit="GB" />
      <h3>{t("settings.cache.sizes")}</h3>
      <table className="caches">
        <tbody>
          {CACHE_KINDS.map((kind) => {
            const cache = sizes.get(kind);
            const bytes = cache?.bytes ?? null;
            return (
              <tr key={kind} title={cache?.path}>
                <th scope="row">{cacheName(kind)}</th>
                <td>{bytes === null ? t("settings.cache.unknown") : readableSize(bytes)}</td>
                <td>
                  <button disabled={bytes === 0} onClick={() => void editor.clearCache(kind)}>
                    {t("settings.cache.clear")}
                  </button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <div className="buttons">
        <button disabled={total === 0 && usage !== null} onClick={() => void editor.clearCache("all")}>
          {t("settings.cache.clearAll")}
        </button>
        <button onClick={() => void editor.loadCacheUsage()}>{t("settings.cache.refresh")}</button>
        {usage?.freed_bytes !== undefined && (
          <span className="muted">{t("settings.cache.freed", { size: readableSize(usage.freed_bytes) })}</span>
        )}
      </div>
      {state.settings.cacheError && (
        <p className="problem" role="alert">
          {state.settings.cacheError}
        </p>
      )}
    </>
  );
}

/** The label of a shortcut action. */
export function actionLabel(id: ActionId): string {
  const angle = /^multicam\.angle_(\d)$/.exec(id);
  if (angle) return t("settings.action.multicam.angle", { angle: Number(angle[1]) });
  return t(`settings.action.${id as Exclude<ActionId, `multicam.angle_${string}`>}`);
}

/** Where an action's keys work, when only in part of the window. */
function contextNote(id: ActionId): string | null {
  const context = contextOf(id);
  if (context === "canvas") return t("settings.shortcuts.onCanvas");
  if (context === "timeline") return t("settings.shortcuts.onTimeline");
  return null;
}

/**
 * The Shortcuts section: a set, every action by group with its keys, recording new keys,
 * the clashes within the set, and resetting, saving or cancelling the changes.
 */
export function ShortcutsSection({ editor, state }: Props) {
  const preferences = preferencesOf(state);
  const [draft, setDraft] = useState<ShortcutDraft>(() => draftOf(preferences));
  const clash = clashes(draft);
  const dirty = isDirty(draft, preferences);
  const changed = new Set(changedActions(draft));

  useEffect(() => {
    if (!draft.recording) return;
    // The next key pressed becomes the action's keys; Escape stops without changing them.
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape" && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
        setDraft((d) => recording(d, null));
        return;
      }
      if (!keyName(event)) return;
      const combo = pressed({ ...keystroke(event), typing: false }, MAC);
      if (combo) setDraft((d) => recorded(d, combo));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [draft.recording]);

  const save = async () => {
    const { set, overrides } = toSave(draft);
    if (await editor.saveShortcuts(set, overrides)) setDraft({ ...draft, recording: null });
  };
  return (
    <>
      <label className="choice">
        <span>{t("settings.shortcuts.set")}</span>
        <select value={draft.set} onChange={(event) => isSetName(event.target.value) && setDraft(withSet(event.target.value))}>
          {SET_NAMES.map((set) => (
            <option key={set} value={set}>
              {t(`settings.shortcuts.set.${set}`)}
            </option>
          ))}
        </select>
      </label>
      <p className="muted hint">{t(`settings.shortcuts.set.${draft.set}.help`)}</p>
      {clash.size > 0 && (
        <p className="problem" role="alert">
          {t("settings.shortcuts.clashes", { count: clash.size })}
        </p>
      )}
      {GROUPS.map((group) => (
        <div key={group} className="shortcut-group">
          <h3>{t(`settings.shortcuts.group.${group}`)}</h3>
          <table className="shortcuts">
            <tbody>
              {ACTIONS.filter((action) => action.group === group).map(({ id }) => {
                const others = clash.get(id);
                const note = contextNote(id);
                return (
                  <tr key={id} className={others ? "clash" : changed.has(id) ? "changed" : undefined}>
                    <th scope="row">
                      {actionLabel(id)}
                      {note && <span className="muted"> {note}</span>}
                      {others && (
                        <span className="problem">
                          {" "}
                          {t("settings.shortcuts.clash", { actions: others.map(actionLabel).join(", ") })}
                        </span>
                      )}
                    </th>
                    <td className="keys">
                      {draft.recording === id ? (
                        <span className="recording">{t("settings.shortcuts.press")}</span>
                      ) : draft.keymap[id].length > 0 ? (
                        <kbd>{shownKeys(draft.keymap[id])}</kbd>
                      ) : (
                        <span className="muted">{t("settings.shortcuts.none")}</span>
                      )}
                    </td>
                    <td className="row-buttons">
                      <button
                        aria-pressed={draft.recording === id}
                        onClick={() => setDraft(recording(draft, draft.recording === id ? null : id))}
                        title={t("settings.shortcuts.record.help")}
                      >
                        {draft.recording === id ? t("settings.shortcuts.stop") : t("settings.shortcuts.record")}
                      </button>
                      <button disabled={draft.keymap[id].length === 0} onClick={() => setDraft(cleared(draft, id))}>
                        {t("settings.shortcuts.clear")}
                      </button>
                      <button disabled={!changed.has(id)} onClick={() => setDraft(reset(draft, id))}>
                        {t("settings.shortcuts.reset")}
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ))}
      <div className="buttons sticky">
        <button disabled={changed.size === 0} onClick={() => setDraft(reset(draft, null))}>
          {t("settings.shortcuts.resetAll")}
        </button>
        <span className="spacer" />
        <button disabled={!dirty} onClick={() => setDraft(draftOf(preferences))}>
          {t("settings.cancel")}
        </button>
        <button className="primary" disabled={!dirty || clash.size > 0} onClick={() => void save()}>
          {t("settings.save")}
        </button>
      </div>
      <Refusal state={state} k="shortcuts" />
    </>
  );
}

const SECTIONS: Record<SettingsSection, (props: Props) => React.JSX.Element> = {
  general: GeneralSection,
  locations: LocationsSection,
  editing: EditingSection,
  timeline: TimelineSection,
  export: ExportSection,
  performance: PerformanceSection,
  cache: CacheSection,
  shortcuts: ShortcutsSection,
};

/** The Settings dialog: the person's preferences by section, each change saved at once. */
export function Settings({ editor, state }: Props) {
  const section = state.settings.section;
  const Section = SECTIONS[section];
  return (
    <div className="dialog-backdrop" onClick={(event) => event.target === event.currentTarget && editor.closeSettings()}>
      <div className="dialog settings" role="dialog" aria-modal="true" aria-label={t("settings.title")}>
        <header className="dialog-head">
          <span className="dialog-icon" aria-hidden="true">
            <Glyph icon={Gear} />
          </span>
          <h2>{t("settings.title")}</h2>
          <IconButton icon={X} label={t("settings.close")} onClick={() => editor.closeSettings()} />
        </header>
        <div className="settings-body">
          <nav aria-label={t("settings.sections")}>
            {SETTINGS_SECTIONS.map((id) => (
              <button
                key={id}
                type="button"
                className={id === section ? "active" : ""}
                aria-current={id === section ? "page" : undefined}
                onClick={() => editor.showSettings(id)}
              >
                <Glyph icon={SECTION_ICONS[id]} size="sm" />
                <span>{t(`settings.section.${id}`)}</span>
              </button>
            ))}
          </nav>
          <section className="settings-section" aria-label={t(`settings.section.${section}`)}>
            <Section editor={editor} state={state} />
          </section>
        </div>
      </div>
    </div>
  );
}

