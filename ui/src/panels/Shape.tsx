import { useState } from "react";

import type { Editor } from "../app/editor";
import type { ClipView } from "../engine/types";
import { t } from "../i18n";
import { SHAPE_NUMBERS, type ShapeField, shapeKinds, shapeOf, shapePatch, shownNumber } from "./shapeFields";
import { Swatch } from "./Text";
import { picked } from "./textFields";

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
      <span>{t(spec.label)}</span>
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
      <span className="unit">{spec.unit === "deg" ? t("shape.degrees") : spec.unit}</span>
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
      placeholder={t("shape.label.none")}
      aria-label={t("shape.label")}
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
      <h2>{t("shape.title")}</h2>
      <label className="choice">
        <span>{t("shape.kind")}</span>
        <select value={s.shape} onChange={(event) => set("shape", event.target.value)}>
          {shapeKinds(clip).map((kind) => (
            <option key={kind} value={kind}>
              {t(`shape.kind.${kind}`)}
            </option>
          ))}
        </select>
      </label>
      <div className="fields" key={`size:${clip.id}`}>
        <Amount field="width" value={s.width} title={t("shape.ofWidth")} onCommit={(v) => set("width", v)} />
        <Amount field="height" value={s.height} title={t("shape.ofHeight")} onCommit={(v) => set("height", v)} />
        {s.shape === "rectangle" && (
          <Amount field="roundness" value={s.roundness} title={t("shape.roundness.help")} onCommit={(v) => set("roundness", v)} />
        )}
      </div>
      <h3>{t("shape.fill")}</h3>
      {colour("color", s.gradient ? t("shape.from") : t("shape.color"))}
      <label className="toggle">
        <input type="checkbox" checked={s.gradient} onChange={(event) => set("gradient", event.target.checked)} />
        <span>{t("shape.gradient")}</span>
      </label>
      {s.gradient && (
        <div className="look">
          {colour("gradient_to", t("shape.to"))}
          <label className="toggle" title={t("shape.radial.help")}>
            <input type="checkbox" checked={s.radial} onChange={(event) => set("radial", event.target.checked)} />
            <span>{t("shape.radial")}</span>
          </label>
          {!s.radial && (
            <div className="fields">
              <Amount field="angle" value={s.angle} title={t("shape.angle.help")} onCommit={(v) => set("angle", v)} />
            </div>
          )}
        </div>
      )}
      <h3>{t("shape.outline")}</h3>
      <label className="toggle">
        <input type="checkbox" checked={s.outline} onChange={(event) => set("outline", event.target.checked)} />
        <span>{t("shape.outline")}</span>
      </label>
      {s.outline && (
        <div className="look">
          {colour("outline_color", t("shape.color"))}
          <div className="fields">
            <Amount
              field="outline_width"
              value={s.outline_width}
              title={t("shape.outline.help")}
              onCommit={(v) => set("outline_width", v)}
            />
          </div>
        </div>
      )}
      <h3>{t("shape.label")}</h3>
      <LabelText key={s.label} text={s.label} onCommit={(text) => set("label", text)} />
      {s.label && (
        <>
          {colour("label_color", t("shape.color"))}
          <div className="fields">
            <Amount field="label_size" value={s.label_size} title={t("shape.ofHeight")} onCommit={(v) => set("label_size", v)} />
            <Amount
              field="label_x"
              value={s.label_x}
              title={t("shape.label.x.help")}
              onCommit={(v) => set("label_x", v)}
            />
            <Amount
              field="label_y"
              value={s.label_y}
              title={t("shape.label.y.help")}
              onCommit={(v) => set("label_y", v)}
            />
          </div>
        </>
      )}
    </section>
  );
}
