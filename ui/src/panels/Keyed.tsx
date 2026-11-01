import { useState } from "react";

import type { Editor } from "../app/editor";
import type { ClipView, CommandCall } from "../engine/types";
import { t } from "../i18n";
import { cx } from "../ui/controls";
import { toggleKey } from "./fields";
import { type KeyedField, type KeyedSpec, allowed, engineValue, keyedChange, keyedSpec } from "./keyedFields";

/** A diamond that adds or removes the property's keyframe at the playhead. */
export function KeyDiamond({ spec, onToggle }: { spec: Pick<KeyedSpec, "animatable" | "animated" | "keyed">; onToggle: () => void }) {
  if (!spec.animatable) return <span className="key-space" />;
  const state = spec.keyed ? "on" : spec.animated ? "animated" : "off";
  const title = spec.keyed ? t("inspector.key.remove") : spec.animated ? t("inspector.key.add") : t("inspector.key.animate");
  return (
    <button type="button" className={`key ${state}`} title={title} aria-label={title} onClick={onToggle}>
      <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
        <path d="M5 0.8 9.2 5 5 9.2 0.8 5z" />
      </svg>
    </button>
  );
}

/**
 * One number: its name, a slider over the range people use, the value to type, its unit,
 * and the keyframe diamond. Dragging shows the value and commits once, on release.
 */
export function KeyedNumber({ spec, onCommit, onToggle }: { spec: KeyedSpec; onCommit: (value: number) => void; onToggle: () => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const [sliding, setSliding] = useState<number | null>(null);
  const commit = () => {
    if (draft !== null && draft.trim() !== "" && Number.isFinite(Number(draft))) onCommit(Number(draft));
    setDraft(null);
  };
  const release = () => {
    if (sliding !== null && sliding !== spec.value) onCommit(sliding);
    setSliding(null);
  };
  const [low, high] = spec.range;
  const shown = sliding ?? spec.value;
  return (
    <div className={cx("field-row", spec.animated && "animated")} title={spec.help}>
      <span className="field-label">{spec.label}</span>
      <input
        type="range"
        className="slider"
        min={low}
        max={high}
        step={spec.step}
        value={Math.min(high, Math.max(low, shown))}
        aria-label={spec.label}
        onChange={(event) => setSliding(Number(event.target.value))}
        onPointerUp={release}
        onKeyUp={release}
        onBlur={release}
      />
      <span className="number">
        <input
          type="number"
          step={spec.step}
          min={spec.min}
          max={spec.max}
          aria-label={spec.label}
          value={draft ?? String(sliding ?? spec.value)}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
            if (event.key === "Escape") setDraft(null);
          }}
        />
        <span className="unit">{spec.unit}</span>
      </span>
      <KeyDiamond spec={spec} onToggle={onToggle} />
    </div>
  );
}

/**
 * Number fields of one clip with sliders and keyframe diamonds: a new value is a keyframe
 * at the playhead when the property has keyframes, otherwise the command `plain` makes of
 * the engine value (null leaves it unchanged).
 */
export function KeyedFields({
  editor,
  clip,
  playhead,
  fields,
  plain,
}: {
  editor: Editor;
  clip: ClipView;
  playhead: number;
  fields: KeyedField[];
  plain: (field: KeyedField, value: number | [number, number]) => CommandCall | null;
}) {
  return (
    <div className="field-rows">
      {fields.map((field) => {
        const spec = keyedSpec(clip, field, playhead);
        return (
          <KeyedNumber
            key={`${field.path}:${field.part ?? ""}:${spec.value}`}
            spec={spec}
            onCommit={(shown) => {
              if (!allowed(field, shown) || shown === spec.value) return;
              const value = engineValue(clip, field, shown, playhead);
              const command = plain(field, value);
              if (command) void editor.edit([keyedChange(clip, field.path, value, playhead, command)]);
            }}
            onToggle={() => void editor.edit([toggleKey(clip, field.path, playhead)])}
          />
        );
      })}
    </div>
  );
}
