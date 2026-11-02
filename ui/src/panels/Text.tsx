import { AlignCenter, AlignLeft, AlignRight, Bold, Italic, type LucideIcon, Underline } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import type { Editor } from "../app/editor";
import type { Catalog, ClipView } from "../engine/types";
import { type MessageKey, t } from "../i18n";
import { english } from "../i18n/catalogue";
import { MAC, formatCombo } from "../keyboard";
import {
  ALIGNS,
  CASES,
  LOOK_NUMBERS,
  type Look,
  NUMBERS,
  REVEAL_EASINGS,
  REVEAL_MOTIONS,
  REVEAL_ORDERS,
  REVEAL_UNITS,
  type RevealDraft,
  picked,
  presetOptions,
  revealChange,
  revealOf,
  rgbOf,
  shownNumber,
  styleOf,
  stylePatch,
  textChange,
  valueAt,
} from "./textFields";
import { Glyph } from "../ui/controls";

const STYLE_ICONS: Record<"bold" | "italic" | "underline", LucideIcon> = { bold: Bold, italic: Italic, underline: Underline };
const ALIGN_ICONS: Record<string, LucideIcon> = { left: AlignLeft, center: AlignCenter, right: AlignRight };

const words = (name: string) => name.replaceAll("_", " ").replaceAll("-", " ");

/** A value the engine names, in the interface's language when known, else in its words. */
function named(prefix: string, value: string): string {
  const key = `${prefix}.${value}`;
  return Object.hasOwn(english, key) ? t(key as MessageKey) : words(value);
}

/** The text, committed on blur or Mod+Enter rather than on every keystroke. */
function TextArea({ text, onCommit }: { text: string; onCommit: (text: string) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null) onCommit(draft);
    setDraft(null);
  };
  return (
    <textarea
      className="text-input"
      rows={3}
      aria-label={t("text.input")}
      title={t("text.input.keys", { apply: formatCombo("Mod+Enter", MAC), cancel: formatCombo("Escape", MAC) })}
      value={draft ?? text}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
          event.preventDefault();
          commit();
        }
        if (event.key === "Escape") setDraft(null);
      }}
    />
  );
}

/** A number in its shown unit, committed on blur or Enter. */
function Amount({ path, value, onCommit }: { path: string; value: number; onCommit: (shown: number) => void }) {
  const spec = NUMBERS[path]!;
  const shown = shownNumber(path, value);
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null && draft.trim() !== "") onCommit(Number(draft));
    setDraft(null);
  };
  return (
    <label className="field">
      <span />
      <span>{t(spec.label)}</span>
      <input
        type="number"
        step={spec.step}
        min={spec.min}
        max={spec.max}
        value={draft ?? String(shown)}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
          if (event.key === "Escape") setDraft(null);
        }}
      />
      <span className="unit">{spec.unit}</span>
    </label>
  );
}

/** A colour swatch; the edit is sent once the picker closes, not while it moves. */
export function Swatch({ label, value, onCommit }: { label: string; value: string; onCommit: (rgb: string) => void }) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const input = ref.current;
    if (!input) return;
    const done = () => onCommit(input.value);
    input.addEventListener("change", done);
    return () => input.removeEventListener("change", done);
  }, [onCommit]);
  return (
    <label className="choice">
      <span>{label}</span>
      <input ref={ref} key={value} type="color" defaultValue={rgbOf(value)} aria-label={label} />
    </label>
  );
}

/** Presets saved in the project and built in, and saving this style as one. */
function Presets({ editor, clip, catalog }: { editor: Editor; clip: ClipView; catalog: Catalog | null }) {
  const saved = editor.getState().opened?.info.text_presets ?? [];
  const options = presetOptions(saved, catalog?.text.presets ?? []);
  const [naming, setNaming] = useState<string | null>(null);
  const save = () => {
    if (naming === null || !naming.trim()) return;
    void editor.saveTextPreset(clip.id, naming).then((done) => done && setNaming(null));
  };
  const own = options.filter((o) => o.saved);
  const builtIn = options.filter((o) => !o.saved);
  return (
    <>
      <label className="choice">
        <span>{t("text.preset")}</span>
        <select
          value=""
          disabled={options.length === 0}
          onChange={(event) => event.target.value && void editor.setText(clip.id, { preset: event.target.value })}
        >
          <option value="">{t("text.preset.apply")}</option>
          {own.length > 0 && (
            <optgroup label={t("text.preset.project")}>
              {own.map((o) => (
                <option key={o.name} value={o.name} title={o.description}>
                  {o.name}
                </option>
              ))}
            </optgroup>
          )}
          {builtIn.length > 0 && (
            <optgroup label={t("text.preset.builtIn")}>
              {builtIn.map((o) => (
                <option key={o.name} value={o.name} title={o.description}>
                  {words(o.name)}
                </option>
              ))}
            </optgroup>
          )}
        </select>
      </label>
      {naming === null ? (
        <div className="buttons">
          <button title={t("text.preset.keep")} onClick={() => setNaming("")}>
            {t("text.preset.saveAs")}
          </button>
        </div>
      ) : (
        <div className="buttons naming">
          <input
            autoFocus
            placeholder={t("text.preset.name")}
            aria-label={t("text.preset.name")}
            value={naming}
            onChange={(event) => setNaming(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") save();
              if (event.key === "Escape") setNaming(null);
            }}
          />
          <button disabled={!naming.trim()} onClick={save}>
            {t("text.save")}
          </button>
          <button onClick={() => setNaming(null)}>{t("text.cancel")}</button>
        </div>
      )}
    </>
  );
}

/** A look's switch, colour and sizes; the fields show while it is on. */
function LookFields({
  look,
  clip,
  onPatch,
}: {
  look: Look;
  clip: ClipView;
  onPatch: (path: string, shown: unknown) => void;
}) {
  const style = styleOf(clip);
  const on = style[look].enabled;
  return (
    <div className="look">
      <label className="toggle">
        <input type="checkbox" checked={on} onChange={(event) => onPatch(`${look}.enabled`, event.target.checked)} />
        <span>{t(`text.look.${look}`)}</span>
      </label>
      {on && (
        <div className="fields">
          <Swatch
            label={t("text.color")}
            value={style[look].color}
            onCommit={(rgb) => onPatch(`${look}.color`, picked(style[look].color, rgb))}
          />
          {LOOK_NUMBERS[look].map((path) => (
            <Amount key={path} path={path} value={valueAt(style, path) as number} onCommit={(shown) => onPatch(path, shown)} />
          ))}
        </div>
      )}
    </div>
  );
}

/** How the text appears: unit by unit, how, how fast, and whether it leaves the same way. */
function Reveal({ editor, clip, catalog }: { editor: Editor; clip: ClipView; catalog: Catalog | null }) {
  const reveal = revealOf(clip);
  const units = catalog?.text.reveal.units ?? REVEAL_UNITS;
  // Every motion the model knows, with any more the engine lists.
  const motions = [...new Set([...REVEAL_MOTIONS, ...(catalog?.text.reveal.motions ?? [])])];
  const change = (draft: Partial<RevealDraft>) => void editor.setText(clip.id, revealChange(clip, draft));
  const easing = typeof reveal.easing === "string" ? reveal.easing : "custom";
  return (
    <>
      <label className="choice">
        <span>{t("text.reveal")}</span>
        <select value={reveal.unit} onChange={(event) => change({ unit: event.target.value as RevealDraft["unit"] })}>
          <option value="">{t("text.reveal.none")}</option>
          {units.map((unit) => (
            <option key={unit} value={unit}>
              {named("text.reveal.unit", unit)}
            </option>
          ))}
        </select>
      </label>
      {reveal.unit && (
        <>
          <label className="choice">
            <span>{t("text.reveal.motion")}</span>
            <select value={reveal.motion} onChange={(event) => change({ motion: event.target.value as RevealDraft["motion"] })}>
              {motions.map((motion) => (
                <option key={motion} value={motion}>
                  {named("text.reveal.motion", motion)}
                </option>
              ))}
            </select>
          </label>
          <RevealSeconds seconds={reveal.seconds} onCommit={(seconds) => change({ seconds })} />
          <label className="choice" title={t("text.reveal.easing.help")}>
            <span>{t("text.reveal.easing")}</span>
            <select value={easing} onChange={(event) => event.target.value !== "custom" && change({ easing: event.target.value as RevealDraft["easing"] })}>
              <option value="">{t("text.reveal.easing.natural")}</option>
              {easing === "custom" && <option value="custom">{t("text.reveal.easing.custom")}</option>}
              {REVEAL_EASINGS.map((name) => (
                <option key={name} value={name}>
                  {named("text.reveal.easing", name)}
                </option>
              ))}
            </select>
          </label>
          <label className="choice" title={t("text.reveal.order.help")}>
            <span>{t("text.reveal.order")}</span>
            <select value={reveal.order} onChange={(event) => change({ order: event.target.value as RevealDraft["order"] })}>
              {REVEAL_ORDERS.map((order) => (
                <option key={order} value={order}>
                  {named("text.reveal.order", order)}
                </option>
              ))}
            </select>
          </label>
          <RevealOverlap overlap={reveal.overlap} onCommit={(overlap) => change({ overlap })} />
          <label className="toggle" title={t("text.reveal.exit.help")}>
            <input type="checkbox" checked={reveal.exit} onChange={(event) => change({ exit: event.target.checked })} />
            <span>{t("text.reveal.exit")}</span>
          </label>
          {reveal.exit && (
            <label className="choice" title={t("text.reveal.exitMotion.help")}>
              <span>{t("text.reveal.exitMotion")}</span>
              <select value={reveal.exitMotion} onChange={(event) => change({ exitMotion: event.target.value as RevealDraft["exitMotion"] })}>
                <option value="">{t("text.reveal.exitMotion.same")}</option>
                {motions.map((motion) => (
                  <option key={motion} value={motion}>
                    {named("text.reveal.motion", motion)}
                  </option>
                ))}
              </select>
            </label>
          )}
        </>
      )}
    </>
  );
}

/** How much each part's motion overlaps the next one's, in % from 0 (one after another) to 100 (all at once). */
function RevealOverlap({ overlap, onCommit }: { overlap: number; onCommit: (overlap: number) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null && draft.trim() !== "") onCommit(Number(draft) / 100);
    setDraft(null);
  };
  return (
    <label className="field" title={t("text.reveal.overlap.help")}>
      <span />
      <span>{t("text.reveal.overlap")}</span>
      <input
        type="number"
        step={5}
        min={0}
        max={100}
        value={draft ?? String(Math.round(overlap * 100))}
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

function RevealSeconds({ seconds, onCommit }: { seconds: number; onCommit: (seconds: number) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null && draft.trim() !== "") onCommit(Number(draft));
    setDraft(null);
  };
  return (
    <label className="field" title={t("text.reveal.duration.help")}>
      <span />
      <span>{t("text.reveal.duration")}</span>
      <input
        type="number"
        step={0.1}
        min={0.05}
        value={draft ?? String(seconds)}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
          if (event.key === "Escape") setDraft(null);
        }}
      />
      <span className="unit">{t("text.seconds")}</span>
    </label>
  );
}

/**
 * The selected text clip's words and look: its text, presets, typography, spacing,
 * outline, box, shadow and glow, and how it is revealed. Every edit is one `text.set`.
 */
/** The text's style as the clip has it, and how a change of one of its paths is sent. */
function useStyle(editor: Editor, clip: ClipView, catalog: Catalog | null) {
  const style = styleOf(clip);
  const listed = editor.getState().opened?.info.fonts ?? catalog?.text.fonts ?? ["Inter"];
  const fonts = listed.includes(style.font) ? listed : [...listed, style.font];
  const patch = (path: string, shown: unknown) => {
    const change = stylePatch(clip, path, shown, listed);
    if (change) void editor.setText(clip.id, { style: change });
  };
  return { style, fonts, patch };
}

/** The words, the saved styles, and the typography: font, size, colour, emphasis, case and alignment. */
export function TextContent({ editor, clip, catalog }: { editor: Editor; clip: ClipView; catalog: Catalog | null }) {
  const { style, fonts, patch } = useStyle(editor, clip, catalog);
  const pressed = (path: "bold" | "italic" | "underline") => (
    <button
      type="button"
      className={style[path] ? "icon-button ghost active" : "icon-button ghost"}
      aria-pressed={style[path]}
      aria-label={t(`text.${path}`)}
      title={t(`text.${path}`)}
      onClick={() => patch(path, !style[path])}
    >
      <Glyph icon={STYLE_ICONS[path]} />
    </button>
  );
  return (
    <section className="effects text-panel">
      <h2>{t("text.title")}</h2>
      <TextArea
        key={clip.text}
        text={clip.text ?? ""}
        onCommit={(text) => void editor.setText(clip.id, textChange(clip, text))}
      />
      <Presets editor={editor} clip={clip} catalog={catalog} />
      <h3>{t("text.typography")}</h3>
      <label className="choice">
        <span>{t("text.font")}</span>
        <select value={style.font} onChange={(event) => patch("font", event.target.value)}>
          {fonts.map((font) => (
            <option key={font} value={font}>
              {font}
            </option>
          ))}
        </select>
      </label>
      <div className="fields">
        <Amount path="size" value={style.size} onCommit={(shown) => patch("size", shown)} />
      </div>
      <Swatch label={t("text.color")} value={style.color} onCommit={(rgb) => patch("color", picked(style.color, rgb))} />
      <div className="buttons tool-row">
        <span className="segmented-icons" role="group" aria-label={t("text.typography")}>
          {pressed("bold")}
          {pressed("italic")}
          {pressed("underline")}
        </span>
        <span className="segmented-icons" role="group" aria-label={t("text.alignment")}>
          {ALIGNS.map((align) => (
            <button
              key={align}
              type="button"
              className={style.align === align ? "icon-button ghost active" : "icon-button ghost"}
              aria-pressed={style.align === align}
              aria-label={t(`text.align.${align}`)}
              title={t(`text.align.${align}`)}
              onClick={() => patch("align", align)}
            >
              <Glyph icon={ALIGN_ICONS[align] ?? AlignLeft} />
            </button>
          ))}
        </span>
      </div>
      <label className="choice">
        <span>{t("text.case")}</span>
        <select value={style.case} onChange={(event) => patch("case", event.target.value)}>
          {(catalog?.text.cases ?? CASES).map((c) => (
            <option key={c} value={c}>
              {named("text.case", c)}
            </option>
          ))}
        </select>
      </label>
    </section>
  );
}

/** Spacing and the looks around the letters: outline, box, shadow and glow. */
export function TextStyleFields({ editor, clip, catalog }: { editor: Editor; clip: ClipView; catalog: Catalog | null }) {
  const { style, patch } = useStyle(editor, clip, catalog);
  return (
    <section className="effects text-panel">
      <h2>{t("text.spacing")}</h2>
      <div className="fields">
        <Amount path="letter_spacing" value={style.letter_spacing} onCommit={(shown) => patch("letter_spacing", shown)} />
        <Amount path="line_height" value={style.line_height} onCommit={(shown) => patch("line_height", shown)} />
      </div>
      <h3>{t("text.appearance")}</h3>
      {(["stroke", "box", "shadow", "glow"] as const).map((look) => (
        <LookFields key={look} look={look} clip={clip} onPatch={patch} />
      ))}
    </section>
  );
}

/** How the text appears: letter by letter, word by word or line by line. */
export function TextReveal({ editor, clip, catalog }: { editor: Editor; clip: ClipView; catalog: Catalog | null }) {
  return (
    <section className="effects text-panel">
      <h2>{t("text.reveal")}</h2>
      <Reveal editor={editor} clip={clip} catalog={catalog} />
    </section>
  );
}

/** Every part of a text clip's panel, one after another. */
export function Text({ editor, clip, catalog }: { editor: Editor; clip: ClipView; catalog: Catalog | null }) {
  return (
    <>
      <TextContent editor={editor} clip={clip} catalog={catalog} />
      <TextStyleFields editor={editor} clip={clip} catalog={catalog} />
      <TextReveal editor={editor} clip={clip} catalog={catalog} />
    </>
  );
}
