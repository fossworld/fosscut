import { useState } from "react";

import type { Editor } from "../app/editor";
import type { ClipView } from "../engine/types";
import { type Field, change, fieldsFor } from "./fields";

const KINDS: Record<string, string> = {
  video: "Video clip",
  audio: "Audio clip",
  image: "Image",
  text: "Title",
  caption: "Caption page",
};

function NumberField({ field, onCommit }: { field: Field; onCommit: (value: number) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null) onCommit(Number(draft));
    setDraft(null);
  };
  return (
    <label className="field">
      <span>{field.label}</span>
      <input
        type="number"
        step={field.step}
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

/** The selected clip: what and where it is, and its placement, opacity and volume. */
export function Inspector({ editor, clip }: { editor: Editor; clip: ClipView }) {
  return (
    <section className="inspector">
      <h2>{KINDS[clip.kind] ?? clip.kind}</h2>
      <p className="name">{clip.text ?? clip.asset_name ?? clip.id}</p>
      <dl className="times">
        <dt>Start</dt>
        <dd>{clip.start.timecode}</dd>
        <dt>End</dt>
        <dd>{clip.end.timecode}</dd>
        <dt>Length</dt>
        <dd>{clip.duration.timecode}</dd>
      </dl>
      <div className="fields" key={clip.id}>
        {fieldsFor(clip).map((field) => (
          <NumberField
            key={`${field.id}:${field.value}`}
            field={field}
            onCommit={(value) => {
              const set = change(clip, field.id, value);
              if (set) void editor.setProperties(clip.id, set);
            }}
          />
        ))}
      </div>
    </section>
  );
}
