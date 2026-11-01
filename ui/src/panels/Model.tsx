import { Box, Lightbulb } from "lucide-react";
import { useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import type { ClipView } from "../engine/types";
import { type MessageKey, t } from "../i18n";
import { Glyph } from "../ui/controls";
import { KeyedFields } from "./Keyed";
import { MODEL_DEFAULTS, modelFields, modelKeyedSet, modelOf, modelSet } from "./modelFields";
import { Swatch } from "./Text";
import { picked } from "./textFields";

/** A material number from 0 to 1, shown in %, committed on blur or Enter. */
function Amount({ label, value, onCommit }: { label: string; value: number; onCommit: (value: number) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    const shown = Number(draft);
    if (draft !== null && draft.trim() !== "" && shown >= 0 && shown <= 100 && shown / 100 !== value) onCommit(Math.round(shown * 10) / 1000);
    setDraft(null);
  };
  return (
    <label className="field">
      <span />
      <span>{label}</span>
      <input
        type="number"
        step={5}
        min={0}
        max={100}
        value={draft ?? String(Math.round(value * 100))}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
          if (event.key === "Escape") setDraft(null);
        }}
      />
      <span className="unit">%</span>
    </label>
  );
}

/** Extruded text's words, committed on blur or Enter. */
function Words({ text, onCommit }: { text: string; onCommit: (text: string) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null && draft.trim() && draft !== text) onCommit(draft.trim());
    setDraft(null);
  };
  return (
    <input
      className="label-input"
      aria-label={t("objects.text")}
      value={draft ?? text}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") commit();
        if (event.key === "Escape") setDraft(null);
      }}
    />
  );
}

/**
 * A 3D object clip (ADR-0040): a preset to start from, what it is, its material and glow,
 * and its light and floor. Numbers that animate take keyframes; the object is placed by
 * its 3D transform and seen through the timeline's camera.
 */
export function Model({ editor, state, clip }: { editor: Editor; state: EditorState; clip: ClipView }) {
  const m = modelOf(clip);
  const fields = modelFields(m);
  const set = (patch: Record<string, unknown>) => void editor.edit([modelSet(clip.id, patch)]);
  const presets = state.catalog?.library?.models ?? [];
  const solid = m.kind !== "asset";
  return (
    <>
      <section className="effects text-panel model-panel">
        <h2>
          <span>{t("objects.title")}</span>
          <Glyph icon={Box} size="sm" />
        </h2>
        <p className="kind-line">{t(`objects.kind.${m.kind}` as MessageKey)}</p>
        {presets.length > 0 && (
          <label className="choice" title={t("objects.preset.help")}>
            <span>{t("objects.preset")}</span>
            <select value="" onChange={(event) => event.target.value && void editor.edit([{ command: "model.set", args: { clip: clip.id, preset: event.target.value } }])}>
              <option value="">{t("objects.preset.apply")}</option>
              {presets.map((preset) => (
                <option key={preset.name} value={preset.name} title={preset.description}>
                  {preset.title}
                </option>
              ))}
            </select>
          </label>
        )}
        {m.kind === "text" && <Words key={m.text} text={m.text} onCommit={(text) => void editor.edit([{ command: "model.set", args: { clip: clip.id, text } }])} />}
        <h3>{t("objects.material")}</h3>
        {solid || m.material ? (
          <>
            <Swatch label={t("objects.color")} value={m.color} onCommit={(rgb) => set({ material: { color: picked(m.color, rgb) } })} />
            <div className="fields" key={`material:${m.metallic}:${m.roughness}`}>
              <Amount label={t("objects.metallic")} value={m.metallic} onCommit={(metallic) => set({ material: { metallic: metallic === MODEL_DEFAULTS.metallic ? null : metallic } })} />
              <Amount label={t("objects.roughness")} value={m.roughness} onCommit={(roughness) => set({ material: { roughness: roughness === MODEL_DEFAULTS.roughness ? null : roughness } })} />
            </div>
            <label className="toggle" title={t("objects.glow.help")}>
              <input type="checkbox" checked={m.emissive !== null} onChange={(event) => set({ material: { emissive: event.target.checked ? m.color : null } })} />
              <span>{t("objects.glow")}</span>
            </label>
            {m.emissive !== null && <Swatch label={t("objects.glow.color")} value={m.emissive} onCommit={(rgb) => set({ material: { emissive: picked(m.emissive ?? m.color, rgb) } })} />}
          </>
        ) : (
          <>
            <p className="hint">{t("objects.asset.materials")}</p>
            <button type="button" className="small" onClick={() => set({ material: { color: MODEL_DEFAULTS.color } })}>
              {t("objects.material.own")}
            </button>
          </>
        )}
        <KeyedFields editor={editor} clip={clip} playhead={state.playhead} fields={fields.shape} plain={(field, value) => modelKeyedSet(clip.id, field.path, value as number)} />
      </section>
      <section className="effects">
        <h2>
          <span>{t("objects.light")}</span>
          <Glyph icon={Lightbulb} size="sm" />
        </h2>
        <KeyedFields editor={editor} clip={clip} playhead={state.playhead} fields={fields.light} plain={(field, value) => modelKeyedSet(clip.id, field.path, value as number)} />
        <label className="toggle" title={t("objects.floor.help")}>
          <input type="checkbox" checked={m.floor} onChange={(event) => set({ floor: event.target.checked ? {} : null })} />
          <span>{t("objects.floor")}</span>
        </label>
        <p className="hint">{t("objects.placement.help")}</p>
      </section>
    </>
  );
}
