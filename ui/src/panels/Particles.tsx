import { Dices, Plus, Sparkles, Wind, X } from "lucide-react";
import { useEffect, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import type { ClipView, SpriteKind } from "../engine/types";
import { type MessageKey, t } from "../i18n";
import { Glyph, IconButton, Segmented } from "../ui/controls";
import { KeyedFields } from "./Keyed";
import type { KeyedField } from "./keyedFields";
import { EMITTER_SHAPES, MAX_COLORS, SPRITES, particleFields, particlesOf, particlesSet, recolored } from "./particleFields";
import { Swatch } from "./Text";
import { picked } from "./textFields";

/** A whole number committed on blur or Enter. */
function Seed({ value, onCommit }: { value: number; onCommit: (seed: number) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    const seed = Number(draft);
    if (draft !== null && Number.isInteger(seed) && seed >= 0 && seed <= 4294967295 && seed !== value) onCommit(seed);
    setDraft(null);
  };
  return (
    <label className="field" title={t("particles.seed.help")}>
      <span />
      <span>{t("particles.seed")}</span>
      <input
        type="number"
        step={1}
        min={0}
        value={draft ?? String(value)}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
          if (event.key === "Escape") setDraft(null);
        }}
      />
      <span className="unit" />
    </label>
  );
}

/**
 * A particle clip's emitter (ADR-0038): a preset to start from, how many are born and for
 * how long, how they move, how they look (size, sprite, colours) and where they are born.
 * Animatable settings take keyframes, read by each particle when it is born.
 */
export function Particles({ editor, state, clip }: { editor: Editor; state: EditorState; clip: ClipView }) {
  useEffect(() => {
    void editor.loadLibrary();
  }, [editor]);
  const playhead = state.playhead;
  const p = particlesOf(clip);
  const fields = particleFields(p);
  const set = (path: string, value: unknown) => void editor.edit([particlesSet(clip.id, path, value)]);
  const plain = (field: KeyedField, value: number | [number, number]) => particlesSet(clip.id, field.path.slice("particles.".length), value);
  const group = (key: keyof typeof fields) => <KeyedFields editor={editor} clip={clip} playhead={playhead} fields={fields[key]} plain={plain} />;
  const presets = (state.library ?? []).filter((item) => item.kind === "particles");
  const colours = (colors: string[] | null) => colors && set("colors", colors);
  return (
    <>
      <section className="effects particles-panel">
        <h2>
          <span>{t("particles.title")}</span>
          <Glyph icon={Sparkles} size="sm" />
        </h2>
        <label className="choice" title={t("particles.preset.help")}>
          <span>{t("particles.preset")}</span>
          <select
            value=""
            disabled={presets.length === 0}
            onChange={(event) =>
              event.target.value && void editor.edit([{ command: "particles.set", args: { clip: clip.id, preset: event.target.value } }])
            }
          >
            <option value="">{t("particles.preset.apply")}</option>
            {presets.map((item) => (
              <option key={item.name} value={item.name} title={item.description}>
                {item.title}
              </option>
            ))}
          </select>
        </label>
        {group("emission")}
        <div className="fields seed-row">
          <Seed value={p.seed} onCommit={(seed) => set("seed", seed)} />
          <IconButton
            icon={Dices}
            size="sm"
            label={t("particles.seed.shuffle")}
            onClick={() => set("seed", Math.floor(Math.random() * 1_000_000))}
          />
        </div>
      </section>
      <section className="effects">
        <h2>
          <span>{t("particles.motion")}</span>
          <Glyph icon={Wind} size="sm" />
        </h2>
        {group("motion")}
      </section>
      <section className="effects">
        <h2>{t("particles.look")}</h2>
        {group("look")}
        <label className="choice">
          <span>{t("particles.sprite")}</span>
          <select value={p.sprite} onChange={(event) => set("sprite.kind", event.target.value as SpriteKind)}>
            {(p.sprite === "image" ? [...SPRITES, "image" as const] : SPRITES).map((kind) => (
              <option key={kind} value={kind}>
                {t(`particles.sprite.${kind}` as MessageKey)}
              </option>
            ))}
          </select>
        </label>
        <div className="choice">
          <span>{t("particles.blend")}</span>
          <Segmented
            label={t("particles.blend")}
            value={p.blend}
            onChange={(blend) => set("blend", blend)}
            options={[
              { id: "add", label: t("particles.blend.add"), help: t("particles.blend.add.help") },
              { id: "normal", label: t("particles.blend.normal"), help: t("particles.blend.normal.help") },
            ]}
          />
        </div>
        <h3>{t("particles.colors")}</h3>
        <ul className="color-list" aria-label={t("particles.colors")}>
          {p.colors.map((color, index) => (
            <li key={`${index}:${color}`}>
              <Swatch
                label={t("particles.color", { number: index + 1 })}
                value={color}
                onCommit={(rgb) => colours(recolored(p.colors, index, picked(color, rgb) ?? color))}
              />
              {p.colors.length > 1 && (
                <IconButton icon={X} size="sm" label={t("particles.color.remove")} onClick={() => colours(recolored(p.colors, index, null))} />
              )}
            </li>
          ))}
        </ul>
        <div className="buttons">
          <button type="button" className="small" disabled={p.colors.length >= MAX_COLORS} onClick={() => colours(recolored(p.colors, p.colors.length, "#FFFFFF"))}>
            <Glyph icon={Plus} size="sm" />
            {t("particles.color.add")}
          </button>
          {p.colors.length > 1 && (
            <label className="choice compact">
              <span>{t("particles.colorBy")}</span>
              <select value={p.color_by} onChange={(event) => set("color_by", event.target.value)}>
                <option value="life">{t("particles.colorBy.life")}</option>
                <option value="random">{t("particles.colorBy.random")}</option>
              </select>
            </label>
          )}
        </div>
      </section>
      <section className="effects">
        <h2>{t("particles.emitter")}</h2>
        <label className="choice">
          <span>{t("particles.emitter.shape")}</span>
          <select value={p.emitter_shape} onChange={(event) => set("emitter.shape", event.target.value)}>
            {EMITTER_SHAPES.map((shape) => (
              <option key={shape} value={shape}>
                {t(`particles.emitter.${shape}` as MessageKey)}
              </option>
            ))}
          </select>
        </label>
        {group("place")}
        <p className="hint">{t("particles.keyframes.help")}</p>
      </section>
    </>
  );
}
