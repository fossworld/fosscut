import { useEffect, useRef, useState } from "react";

import type { Editor } from "../app/editor";
import type { Catalog, ClipView } from "../engine/types";
import {
  ALIGNS,
  CASES,
  LOOK_NUMBERS,
  type Look,
  NUMBERS,
  REVEAL_MOTIONS,
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

const words = (name: string) => name.replaceAll("_", " ").replaceAll("-", " ");

const CASE_NAMES: Record<string, string> = {
  as_written: "As written",
  upper: "UPPER CASE",
  lower: "lower case",
  title: "Title Case",
};

const LOOK_NAMES: Record<Look, string> = {
  stroke: "Outline",
  box: "Box",
  shadow: "Shadow",
  glow: "Glow",
};

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
      aria-label="Text"
      title="Mod+Enter applies; Escape cancels"
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
      <span>{spec.label}</span>
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
function Swatch({ label, value, onCommit }: { label: string; value: string; onCommit: (rgb: string) => void }) {
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
        <span>Preset</span>
        <select
          value=""
          disabled={options.length === 0}
          onChange={(event) => event.target.value && void editor.setText(clip.id, { preset: event.target.value })}
        >
          <option value="">Apply a preset</option>
          {own.length > 0 && (
            <optgroup label="This project">
              {own.map((o) => (
                <option key={o.name} value={o.name} title={o.description}>
                  {o.name}
                </option>
              ))}
            </optgroup>
          )}
          {builtIn.length > 0 && (
            <optgroup label="Built in">
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
          <button title="Keep this style in the project to reuse it" onClick={() => setNaming("")}>
            Save style as preset
          </button>
        </div>
      ) : (
        <div className="buttons naming">
          <input
            autoFocus
            placeholder="Preset name"
            aria-label="Preset name"
            value={naming}
            onChange={(event) => setNaming(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") save();
              if (event.key === "Escape") setNaming(null);
            }}
          />
          <button disabled={!naming.trim()} onClick={save}>
            Save
          </button>
          <button onClick={() => setNaming(null)}>Cancel</button>
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
        <span>{LOOK_NAMES[look]}</span>
      </label>
      {on && (
        <div className="fields">
          <Swatch
            label="Color"
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
  const motions = catalog?.text.reveal.motions ?? REVEAL_MOTIONS;
  const change = (draft: Partial<RevealDraft>) => void editor.setText(clip.id, revealChange(clip, draft));
  return (
    <>
      <label className="choice">
        <span>Reveal</span>
        <select value={reveal.unit} onChange={(event) => change({ unit: event.target.value as RevealDraft["unit"] })}>
          <option value="">None</option>
          {units.map((unit) => (
            <option key={unit} value={unit}>
              {unit === "letter" ? "Letter by letter" : unit === "word" ? "Word by word" : "Line by line"}
            </option>
          ))}
        </select>
      </label>
      {reveal.unit && (
        <>
          <label className="choice">
            <span>Motion</span>
            <select value={reveal.motion} onChange={(event) => change({ motion: event.target.value as RevealDraft["motion"] })}>
              {motions.map((motion) => (
                <option key={motion} value={motion}>
                  {motion}
                </option>
              ))}
            </select>
          </label>
          <RevealSeconds seconds={reveal.seconds} onCommit={(seconds) => change({ seconds })} />
          <label className="toggle" title="Leave the same way, in reverse, before the clip ends">
            <input type="checkbox" checked={reveal.exit} onChange={(event) => change({ exit: event.target.checked })} />
            <span>Also on exit</span>
          </label>
        </>
      )}
    </>
  );
}

function RevealSeconds({ seconds, onCommit }: { seconds: number; onCommit: (seconds: number) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null && draft.trim() !== "") onCommit(Number(draft));
    setDraft(null);
  };
  return (
    <label className="field" title="Time until the whole text shows">
      <span />
      <span>Duration</span>
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
      <span className="unit">s</span>
    </label>
  );
}

/**
 * The selected text clip's words and look: its text, presets, typography, spacing,
 * outline, box, shadow and glow, and how it is revealed. Every edit is one `text.set`.
 */
export function Text({ editor, clip, catalog }: { editor: Editor; clip: ClipView; catalog: Catalog | null }) {
  const style = styleOf(clip);
  const listed = editor.getState().opened?.info.fonts ?? catalog?.text.fonts ?? ["Inter"];
  const fonts = listed.includes(style.font) ? listed : [...listed, style.font];
  const patch = (path: string, shown: unknown) => {
    const change = stylePatch(clip, path, shown, listed);
    if (change) void editor.setText(clip.id, { style: change });
  };
  const pressed = (path: "bold" | "italic" | "underline", label: string) => (
    <button className={style[path] ? "active" : ""} aria-pressed={style[path]} onClick={() => patch(path, !style[path])}>
      {label}
    </button>
  );
  return (
    <section className="effects text-panel">
      <h2>Text</h2>
      <TextArea
        key={clip.text}
        text={clip.text ?? ""}
        onCommit={(text) => void editor.setText(clip.id, textChange(clip, text))}
      />
      <Presets editor={editor} clip={clip} catalog={catalog} />
      <h3>Typography</h3>
      <label className="choice">
        <span>Font</span>
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
      <Swatch label="Color" value={style.color} onCommit={(rgb) => patch("color", picked(style.color, rgb))} />
      <div className="buttons">
        {pressed("bold", "Bold")}
        {pressed("italic", "Italic")}
        {pressed("underline", "Underline")}
      </div>
      <label className="choice">
        <span>Case</span>
        <select value={style.case} onChange={(event) => patch("case", event.target.value)}>
          {(catalog?.text.cases ?? CASES).map((c) => (
            <option key={c} value={c}>
              {CASE_NAMES[c] ?? words(c)}
            </option>
          ))}
        </select>
      </label>
      <div className="buttons" role="group" aria-label="Alignment">
        {ALIGNS.map((align) => (
          <button
            key={align}
            className={style.align === align ? "active" : ""}
            aria-pressed={style.align === align}
            onClick={() => patch("align", align)}
          >
            {align === "center" ? "Centre" : align === "left" ? "Left" : "Right"}
          </button>
        ))}
      </div>
      <h3>Spacing</h3>
      <div className="fields">
        <Amount path="letter_spacing" value={style.letter_spacing} onCommit={(shown) => patch("letter_spacing", shown)} />
        <Amount path="line_height" value={style.line_height} onCommit={(shown) => patch("line_height", shown)} />
      </div>
      <h3>Appearance</h3>
      {(["stroke", "box", "shadow", "glow"] as const).map((look) => (
        <LookFields key={look} look={look} clip={clip} onPatch={patch} />
      ))}
      <h3>Reveal</h3>
      <Reveal editor={editor} clip={clip} catalog={catalog} />
    </section>
  );
}
