import { PenLine, Spline } from "lucide-react";
import { useState } from "react";

import type { Editor } from "../app/editor";
import type { ClipView } from "../engine/types";
import { t } from "../i18n";
import { Glyph, Segmented } from "../ui/controls";
import { KeyedFields } from "./Keyed";
import type { KeyedField } from "./keyedFields";
import { CAPS, JOINS, SHAPE_NUMBERS, type ShapeField, isOpen, shapeKinds, shapeOf, shapePatch, shownNumber } from "./shapeFields";
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

/** The trims and the stroke width as keyed numbers, with the shape field each one sets. */
function vectorFields(clip: ClipView): { fields: KeyedField[]; byPath: Record<string, ShapeField> } {
  const s = shapeOf(clip);
  return {
    fields: [
      { path: "shape.trim_start", plain: s.trim_start, label: t("vector.trim.start"), unit: "%", scale: 100, min: 0, max: 100, range: [0, 100], help: t("vector.trim.start.help") },
      { path: "shape.trim_end", plain: s.trim_end, label: t("vector.trim.end"), unit: "%", scale: 100, min: 0, max: 100, range: [0, 100], help: t("vector.trim.end.help") },
      { path: "shape.trim_offset", plain: s.trim_offset, label: t("vector.trim.offset"), unit: "%", scale: 100, range: [-100, 100], help: t("vector.trim.offset.help") },
    ],
    byPath: { "shape.trim_start": "trim_start", "shape.trim_end": "trim_end", "shape.trim_offset": "trim_offset", "shape.stroke_width": "stroke_width" },
  };
}

/**
 * A stroke along the shape's edge or its open path (ADR-0036): colour, width (keyed), caps,
 * joins and dashes.
 */
function Stroke({ editor, clip, playhead }: { editor: Editor; clip: ClipView; playhead: number }) {
  const s = shapeOf(clip);
  const set = (field: ShapeField, shown: unknown) => void editor.setShape(clip.id, shapePatch(clip, field, shown));
  const width: KeyedField = {
    path: "shape.stroke_width",
    plain: s.stroke_width,
    label: t("vector.stroke.width"),
    unit: "%",
    scale: 100,
    min: 0,
    max: 50,
    range: [0, 5],
    step: 0.05,
    help: t("vector.stroke.width.help"),
  };
  return (
    <>
      <h3>{t("vector.stroke")}</h3>
      <label className="toggle">
        <input type="checkbox" checked={s.stroke} onChange={(event) => set("stroke", event.target.checked)} />
        <span>{t("vector.stroke.on")}</span>
      </label>
      {s.stroke && (
        <div className="look">
          <Swatch label={t("shape.color")} value={s.stroke_color} onCommit={(rgb) => set("stroke_color", picked(s.stroke_color, rgb))} />
          <KeyedFields
            editor={editor}
            clip={clip}
            playhead={playhead}
            fields={[width]}
            plain={(_, value) => {
              const patch = shapePatch(clip, "stroke_width", value);
              return patch ? { command: "shape.set", args: { clip: clip.id, set: patch } } : null;
            }}
          />
          <div className="choice">
            <span>{t("vector.cap")}</span>
            <Segmented
              label={t("vector.cap")}
              value={s.cap}
              onChange={(cap) => set("cap", cap)}
              options={CAPS.map((cap) => ({ id: cap, label: t(`vector.cap.${cap}`), help: t(`vector.cap.${cap}.help`) }))}
            />
          </div>
          <div className="choice">
            <span>{t("vector.join")}</span>
            <Segmented
              label={t("vector.join")}
              value={s.join}
              onChange={(join) => set("join", join)}
              options={JOINS.map((join) => ({ id: join, label: t(`vector.join.${join}`) }))}
            />
          </div>
          <label className="toggle">
            <input type="checkbox" checked={s.dashed} onChange={(event) => set("dashed", event.target.checked)} />
            <span>{t("vector.dashed")}</span>
          </label>
          {s.dashed && (
            <div className="fields" key={`dashes:${s.dash}:${s.gap}`}>
              <Amount field="dash" value={s.dash} title={t("shape.ofHeight")} onCommit={(v) => set("dash", v)} />
              <Amount field="gap" value={s.gap} title={t("shape.ofHeight")} onCommit={(v) => set("gap", v)} />
            </div>
          )}
        </div>
      )}
    </>
  );
}

/**
 * The part of the outline that shows (ADR-0036): start, end and offset with keyframe
 * diamonds, and the draw-on and draw-off animations that move them.
 */
function Trims({ editor, clip, playhead }: { editor: Editor; clip: ClipView; playhead: number }) {
  const { fields, byPath } = vectorFields(clip);
  const drawn = (place: "in" | "out", preset: string) => clip.animations?.[place]?.preset === preset;
  return (
    <section className="effects" data-part="trims">
      <h2>
        <span>{t("vector.trim")}</span>
        <Glyph icon={Spline} size="sm" />
      </h2>
      <KeyedFields
        editor={editor}
        clip={clip}
        playhead={playhead}
        fields={fields}
        plain={(field, value) => {
          const patch = shapePatch(clip, byPath[field.path]!, value);
          return patch ? { command: "shape.set", args: { clip: clip.id, set: patch } } : null;
        }}
      />
      <div className="buttons">
        <button
          type="button"
          className="small"
          aria-pressed={drawn("in", "draw_on")}
          title={t("vector.drawOn.help")}
          onClick={() => void editor.setAnimation(clip.id, "in", drawn("in", "draw_on") ? "" : "draw_on", 1)}
        >
          <Glyph icon={PenLine} size="sm" />
          {t("vector.drawOn")}
        </button>
        <button
          type="button"
          className="small"
          aria-pressed={drawn("out", "draw_off")}
          title={t("vector.drawOff.help")}
          onClick={() => void editor.setAnimation(clip.id, "out", drawn("out", "draw_off") ? "" : "draw_off", 1)}
        >
          {t("vector.drawOff")}
        </button>
      </div>
      <p className="hint">{t("vector.trim.help")}</p>
    </section>
  );
}

/**
 * The selected shape clip's shape (ADR-0028): its outline, size and roundness, its fill
 * and gradient (or none), the outline along its edge, its stroke, its label, and the part
 * of its outline that shows. Every edit is one `shape.set` with a sparse merge patch, or a
 * keyframe at the playhead for trims and stroke widths with keyframes.
 */
export function Shape({ editor, clip, playhead }: { editor: Editor; clip: ClipView; playhead: number }) {
  const s = shapeOf(clip);
  const set = (field: ShapeField, shown: unknown) => void editor.setShape(clip.id, shapePatch(clip, field, shown));
  const colour = (field: "color" | "gradient_to" | "outline_color" | "label_color", label: string) => (
    <Swatch label={label} value={s[field]} onCommit={(rgb) => set(field, picked(s[field], rgb))} />
  );
  return (
    <>
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
      {s.shape === "path" && (
        <label className="toggle" title={t("vector.closed.help")}>
          <input type="checkbox" checked={s.closed} onChange={(event) => set("closed", event.target.checked)} />
          <span>{t("vector.closed")}</span>
        </label>
      )}
      <h3>{t("shape.fill")}</h3>
      <label className="toggle" title={isOpen(clip) ? t("vector.fill.open") : undefined}>
        <input type="checkbox" checked={!s.no_fill} onChange={(event) => set("no_fill", !event.target.checked)} />
        <span>{t("vector.fill.on")}</span>
      </label>
      {!s.no_fill && colour("color", s.gradient ? t("shape.from") : t("shape.color"))}
      {!s.no_fill && (
        <label className="toggle">
          <input type="checkbox" checked={s.gradient} onChange={(event) => set("gradient", event.target.checked)} />
          <span>{t("shape.gradient")}</span>
        </label>
      )}
      {!s.no_fill && s.gradient && (
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
      <Stroke editor={editor} clip={clip} playhead={playhead} />
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
    <Trims editor={editor} clip={clip} playhead={playhead} />
    </>
  );
}
