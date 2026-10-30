import { useState } from "react";

import type { Editor } from "../app/editor";
import type { ClipView } from "../engine/types";
import { SHAPE_NUMBERS, type ShapeField, shapeKinds, shapeOf, shapePatch, shownNumber } from "./shapeFields";
import { Swatch } from "./Text";
import { picked } from "./textFields";

const SHAPE_NAMES: Record<string, string> = {
  rectangle: "Rectangle",
  ellipse: "Ellipse",
  star: "Star",
  heart: "Heart",
  path: "Pen path",
};

/** A number in its shown unit, committed on blur or Enter. */
function Amount({
  field,
  value,
  title,
  onCommit,
}: {
  field: ShapeField;
  value: number;
  title?: string;
  onCommit: (shown: number) => void;
}) {
  const spec = SHAPE_NUMBERS[field]!;
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null && draft.trim() !== "") onCommit(Number(draft));
    setDraft(null);
  };
  return (
    <label className="field" title={title}>
      <span />
      <span>{spec.label}</span>
      <input
        type="number"
        step={spec.step}
        min={spec.min}
        max={spec.max}
        value={draft ?? String(shownNumber(field, value))}
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

/** The label's words, committed on blur or Enter; empty removes the label. */
function LabelText({ text, onCommit }: { text: string; onCommit: (text: string) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null) onCommit(draft);
    setDraft(null);
  };
  return (
    <input
      className="label-input"
      placeholder="No label: type one to add it"
      aria-label="Label"
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
 * The selected shape clip's shape (ADR-0028): its outline, size and roundness, its fill
 * and gradient, the outline along its edge, and its label. Every edit is one `shape.set`
 * with a sparse merge patch.
 */
export function Shape({ editor, clip }: { editor: Editor; clip: ClipView }) {
  const s = shapeOf(clip);
  const set = (field: ShapeField, shown: unknown) => void editor.setShape(clip.id, shapePatch(clip, field, shown));
  const colour = (field: "color" | "gradient_to" | "outline_color" | "label_color", label: string) => (
    <Swatch label={label} value={s[field]} onCommit={(rgb) => set(field, picked(s[field], rgb))} />
  );
  return (
    <section className="effects text-panel shape-panel">
      <h2>Shape</h2>
      <label className="choice">
        <span>Kind</span>
        <select value={s.shape} onChange={(event) => set("shape", event.target.value)}>
          {shapeKinds(clip).map((kind) => (
            <option key={kind} value={kind}>
              {SHAPE_NAMES[kind] ?? kind}
            </option>
          ))}
        </select>
      </label>
      <div className="fields" key={`size:${clip.id}`}>
        <Amount field="width" value={s.width} title="Of the canvas's width" onCommit={(v) => set("width", v)} />
        <Amount field="height" value={s.height} title="Of the canvas's height" onCommit={(v) => set("height", v)} />
        {s.shape === "rectangle" && (
          <Amount field="roundness" value={s.roundness} title="How round the corners are" onCommit={(v) => set("roundness", v)} />
        )}
      </div>
      <h3>Fill</h3>
      {colour("color", s.gradient ? "From" : "Color")}
      <label className="toggle">
        <input type="checkbox" checked={s.gradient} onChange={(event) => set("gradient", event.target.checked)} />
        <span>Gradient</span>
      </label>
      {s.gradient && (
        <div className="look">
          {colour("gradient_to", "To")}
          <label className="toggle" title="From the centre outwards instead of along a line">
            <input type="checkbox" checked={s.radial} onChange={(event) => set("radial", event.target.checked)} />
            <span>Radial</span>
          </label>
          {!s.radial && (
            <div className="fields">
              <Amount field="angle" value={s.angle} title="0 runs left to right, 90 top to bottom" onCommit={(v) => set("angle", v)} />
            </div>
          )}
        </div>
      )}
      <h3>Outline</h3>
      <label className="toggle">
        <input type="checkbox" checked={s.outline} onChange={(event) => set("outline", event.target.checked)} />
        <span>Outline</span>
      </label>
      {s.outline && (
        <div className="look">
          {colour("outline_color", "Color")}
          <div className="fields">
            <Amount
              field="outline_width"
              value={s.outline_width}
              title="Of the shape's shorter side"
              onCommit={(v) => set("outline_width", v)}
            />
          </div>
        </div>
      )}
      <h3>Label</h3>
      <LabelText key={s.label} text={s.label} onCommit={(text) => set("label", text)} />
      {s.label && (
        <>
          {colour("label_color", "Color")}
          <div className="fields">
            <Amount field="label_size" value={s.label_size} title="Of the canvas's height" onCommit={(v) => set("label_size", v)} />
            <Amount
              field="label_x"
              value={s.label_x}
              title="From the shape's left edge (-100) to its right edge (100)"
              onCommit={(v) => set("label_x", v)}
            />
            <Amount
              field="label_y"
              value={s.label_y}
              title="From the shape's top edge (-100) to its bottom edge (100)"
              onCommit={(v) => set("label_y", v)}
            />
          </div>
        </>
      )}
    </section>
  );
}
