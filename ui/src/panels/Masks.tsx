import { useState } from "react";

import type { Editor, MaskTool } from "../app/editor";
import type { ClipView, MaskView } from "../engine/types";
import { type MaskField, maskChange, maskFields, toggleMaskKey } from "./maskFields";

const SHAPES = ["rectangle", "ellipse", "linear", "mirror", "star", "heart", "path", "text", "brush"];
const MODES = ["add", "subtract", "intersect"];

function Field({
  field,
  onCommit,
  onToggle,
}: {
  field: MaskField;
  onCommit: (value: number) => void;
  onToggle: () => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null) onCommit(Number(draft));
    setDraft(null);
  };
  const state = field.keyed ? "on" : field.animated ? "animated" : "off";
  return (
    <label className={`field${field.animated ? " animated" : ""}`}>
      <button className={`key ${state}`} title="Add or remove a keyframe at the playhead" onClick={onToggle}>
        <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
          <path d="M5 0.8 9.2 5 5 9.2 0.8 5z" />
        </svg>
      </button>
      <span>{field.label}</span>
      <input
        type="number"
        min={field.min}
        max={field.max}
        value={draft ?? String(field.value)}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
          if (event.key === "Escape") setDraft(null);
        }}
      />
      <span className="unit">{field.unit}</span>
    </label>
  );
}

function MaskCard({
  editor,
  clip,
  mask,
  playhead,
  tool,
}: {
  editor: Editor;
  clip: ClipView;
  mask: MaskView;
  playhead: number;
  tool: MaskTool | null;
}) {
  const set = (args: Record<string, unknown>) =>
    void editor.edit([{ command: "mask.set", args: { clip: clip.id, mask: mask.id, ...args } }]);
  const shape = mask.shape ?? "rectangle";
  const drawing = tool?.mask === mask.id;
  return (
    <div className={`effect${mask.enabled === false ? " off" : ""}`}>
      <div className="effect-head">
        <input
          type="checkbox"
          checked={mask.enabled !== false}
          title="Use this mask"
          onChange={(event) => set({ enabled: event.target.checked })}
        />
        <select value={shape} aria-label="Shape" onChange={(event) => set({ shape: event.target.value })}>
          {SHAPES.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        <select value={mask.mode ?? "add"} aria-label="Combine" onChange={(event) => set({ mode: event.target.value })}>
          {MODES.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
        <label className="toggle" title="Keep the outside">
          <input type="checkbox" checked={mask.invert === true} onChange={(event) => set({ invert: event.target.checked })} />
          <span>Invert</span>
        </label>
        <button
          title="Remove the mask"
          onClick={() => void editor.edit([{ command: "mask.remove", args: { clip: clip.id, mask: mask.id } }])}
        >
          ×
        </button>
      </div>
      {shape === "text" && (
        <label className="choice">
          <span>Text</span>
          <input
            type="text"
            defaultValue={mask.text ?? ""}
            onBlur={(event) => event.target.value.trim() && event.target.value !== mask.text && set({ text: event.target.value })}
          />
        </label>
      )}
      {(shape === "path" || shape === "brush") && (
        <div className="buttons">
          <button
            className={drawing ? "active" : ""}
            onClick={() =>
              editor.setMaskTool(
                drawing
                  ? null
                  : { clip: clip.id, mask: mask.id, mode: shape === "path" ? "pen" : "brush", radius: 0.04, erase: false },
              )
            }
          >
            {drawing ? "Stop drawing" : shape === "path" ? "Redraw on the preview" : "Paint on the preview"}
          </button>
        </div>
      )}
      <div className="fields">
        {maskFields(clip, mask, playhead).map((field) => (
          <Field
            key={`${field.id}:${field.value}`}
            field={field}
            onCommit={(value) => {
              const command = maskChange(clip, mask, field.id, value, playhead);
              if (command) void editor.edit([command]);
            }}
            onToggle={() => void editor.edit([toggleMaskKey(clip, mask, field.property, playhead)])}
          />
        ))}
      </div>
    </div>
  );
}

/** The selected clip's masks, combined in order, and adding new ones. */
export function Masks({
  editor,
  clip,
  playhead,
  tool,
}: {
  editor: Editor;
  clip: ClipView;
  playhead: number;
  tool: MaskTool | null;
}) {
  const masks = clip.masks ?? [];
  const brush = tool && tool.clip === clip.id && tool.mode === "brush" ? tool : null;
  return (
    <section className="effects masks">
      <h2>Masks</h2>
      {masks.map((mask) => (
        <MaskCard key={mask.id} editor={editor} clip={clip} mask={mask} playhead={playhead} tool={tool} />
      ))}
      {brush && (
        <div className="params">
          <label className="param">
            <span />
            <span>Brush size</span>
            <input
              type="range"
              min={0.005}
              max={0.2}
              step={0.005}
              value={brush.radius}
              onChange={(event) => editor.setMaskTool({ ...brush, radius: Number(event.target.value) })}
            />
          </label>
          <label className="toggle">
            <input type="checkbox" checked={brush.erase} onChange={(event) => editor.setMaskTool({ ...brush, erase: event.target.checked })} />
            <span>Erase</span>
          </label>
        </div>
      )}
      <div className="add">
        <select
          value=""
          aria-label="Add a mask"
          onChange={(event) => event.target.value && void editor.addMask(clip.id, event.target.value)}
        >
          <option value="">Add a mask</option>
          {SHAPES.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
      </div>
    </section>
  );
}
