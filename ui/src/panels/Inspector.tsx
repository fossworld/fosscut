import { useState } from "react";

import { EASINGS, type EasingName, easingName, keyAt, keyTimes } from "../app/animation";
import type { Editor } from "../app/editor";
import type { Catalog, ClipView } from "../engine/types";
import { Effects } from "./Effects";
import { Masks } from "./Masks";
import {
  type Field,
  type Viewing,
  change,
  fieldsFor,
  hasPicture,
  hasSound,
  isPlaced,
  keyedPaths,
  stretched,
  toggleKey,
} from "./fields";

const KINDS: Record<string, string> = {
  video: "Video clip",
  audio: "Audio clip",
  image: "Image",
  text: "Title",
  caption: "Caption page",
  adjustment: "Adjustment layer",
};

const BLENDS = [
  "normal",
  "multiply",
  "screen",
  "overlay",
  "darken",
  "lighten",
  "color_dodge",
  "color_burn",
  "hard_light",
  "soft_light",
  "linear_burn",
  "add",
  "difference",
  "exclusion",
];

const CHANNELS = ["stereo", "left", "right", "mono"];

const words = (name: string) => name.replaceAll("_", " ");

/** A diamond that adds or removes the property's keyframe at the playhead. */
function KeyButton({ field, onToggle }: { field: Field; onToggle: () => void }) {
  if (!field.animatable) return <span />;
  const state = field.keyed ? "on" : field.animated ? "animated" : "off";
  const title = field.keyed
    ? "Remove the keyframe at the playhead"
    : field.animated
      ? "Add a keyframe at the playhead"
      : "Animate: add a keyframe at the playhead";
  return (
    <button className={`key ${state}`} title={title} aria-label={title} onClick={onToggle}>
      <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
        <path d="M5 0.8 9.2 5 5 9.2 0.8 5z" />
      </svg>
    </button>
  );
}

function NumberField({
  field,
  onCommit,
  onToggle,
}: {
  field: Field;
  onCommit: (value: number) => void;
  onToggle: () => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null) onCommit(Number(draft));
    setDraft(null);
  };
  return (
    <label className={`field${field.animated ? " animated" : ""}`}>
      <KeyButton field={field} onToggle={onToggle} />
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

function Choice({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: string[];
  onChange: (value: string) => void;
}) {
  return (
    <label className="choice">
      <span>{label}</span>
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        {options.map((option) => (
          <option key={option} value={option}>
            {words(option)}
          </option>
        ))}
      </select>
    </label>
  );
}

/** Previous and next keyframe, and the easing of the keyframes at the playhead. */
function Keyframes({ editor, clip, playhead }: { editor: Editor; clip: ClipView; playhead: number }) {
  const times = keyTimes(clip);
  if (times.length === 0) return null;
  const previous = [...times].reverse().find((t) => t < playhead);
  const next = times.find((t) => t > playhead);
  const here = keyedPaths(clip, playhead);
  const first = here[0] ? keyAt(clip, here[0], playhead) : undefined;
  const easing = first ? easingName(first.interpolation) : null;
  const setEasing = (name: string) => {
    void editor.edit(
      here.map((property) => ({
        command: "keyframe.set",
        args: { clip: clip.id, property, at: { flicks: playhead }, easing: name },
      })),
    );
  };
  return (
    <div className="keyframes">
      <button
        title="Previous keyframe"
        disabled={previous === undefined}
        onClick={() => previous !== undefined && void editor.seek(previous)}
      >
        ◂ Key
      </button>
      <button title="Next keyframe" disabled={next === undefined} onClick={() => next !== undefined && void editor.seek(next)}>
        Key ▸
      </button>
      {easing !== null && (
        <Choice
          label="Easing"
          value={easing}
          options={easing === "custom" ? ["custom", ...Object.keys(EASINGS)] : Object.keys(EASINGS)}
          onChange={(name) => name !== "custom" && setEasing(name as EasingName)}
        />
      )}
    </div>
  );
}

/**
 * The selected clip at the playhead: what and where it is, its placement, crop, opacity,
 * blend mode and sound, with keyframe diamonds beside the animatable values.
 */
export function Inspector({
  editor,
  clip,
  playhead,
  catalog = null,
  luts = [],
  others = [],
}: {
  editor: Editor;
  clip: ClipView;
  playhead: number;
  catalog?: Catalog | null;
  luts?: { id: string; name: string }[];
  others?: ClipView[];
}) {
  const [separate, setSeparate] = useState(false);
  const viewing: Viewing = { playhead, separate };
  const set = (properties: Record<string, unknown>) => void editor.setProperties(clip.id, properties);
  const t = clip.transform ?? {};
  const enabled = clip.enabled !== false;
  return (
    <section className={`inspector${enabled ? "" : " disabled"}`}>
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
      <label className="toggle">
        <input type="checkbox" checked={enabled} onChange={(event) => set({ enabled: event.target.checked })} />
        <span>Active (V)</span>
      </label>
      <Keyframes editor={editor} clip={clip} playhead={playhead} />
      <div className="fields" key={clip.id}>
        {fieldsFor(clip, viewing).map((field) => (
          <NumberField
            key={`${field.id}:${field.value}`}
            field={field}
            onCommit={(value) => {
              const command = change(clip, field.id, value, viewing);
              if (command) void editor.edit([command]);
            }}
            onToggle={() => void editor.edit([toggleKey(clip, field.path, playhead)])}
          />
        ))}
      </div>
      {isPlaced(clip) && (
        <div className="options">
          <label className="toggle">
            <input
              type="checkbox"
              checked={!(separate || stretched(clip))}
              disabled={stretched(clip)}
              onChange={(event) => setSeparate(!event.target.checked)}
            />
            <span>Uniform scale</span>
          </label>
          <div className="buttons">
            <button
              className={t.flip_horizontal ? "active" : ""}
              aria-pressed={t.flip_horizontal === true}
              onClick={() => set({ "transform.flip_horizontal": !t.flip_horizontal })}
            >
              Mirror
            </button>
            <button
              className={t.flip_vertical ? "active" : ""}
              aria-pressed={t.flip_vertical === true}
              onClick={() => set({ "transform.flip_vertical": !t.flip_vertical })}
            >
              Flip
            </button>
          </div>
          <Choice label="Blend" value={clip.blend ?? "normal"} options={BLENDS} onChange={(blend) => set({ blend })} />
        </div>
      )}
      {hasSound(clip) && (
        <div className="options">
          <Choice
            label="Channels"
            value={clip.audio?.channels ?? "stereo"}
            options={CHANNELS}
            onChange={(channels) => set({ "audio.channels": channels })}
          />
        </div>
      )}
      {hasPicture(clip) && (
        <Effects editor={editor} clip={clip} playhead={playhead} catalog={catalog} luts={luts} others={others} />
      )}
      {hasPicture(clip) && <Masks editor={editor} clip={clip} playhead={playhead} tool={editor.getState().maskTool} />}
    </section>
  );
}
