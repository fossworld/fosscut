import { useState } from "react";

import { EASINGS, type EasingName, easingName, keyAt, keyTimes } from "../app/animation";
import { type Editor, preferencesOf } from "../app/editor";
import { isMulticam } from "../app/multicam";
import type { AssetView, Catalog, ClipView, Moment } from "../engine/types";
import { type MessageKey, t } from "../i18n";
import { hint } from "../shortcuts";
import { findClip } from "../timeline/geometry";
import { formatTime, parseRate } from "../transport/time";
import { CaptionTools } from "./CaptionTools";
import { Effects } from "./Effects";
import { targetsOf } from "./effectFields";
import { Masks } from "./Masks";
import { Measured } from "./Measured";
import { Motion } from "./Motion";
import { Multicam } from "./Multicam";
import { Shape } from "./Shape";
import { Speed } from "./Speed";
import { Subject } from "./Subject";
import { Text } from "./Text";
import {
  type Field,
  type Viewing,
  change,
  fieldsFor,
  hasPicture,
  hasSound,
  isPlaced,
  keyedPaths,
  playsSound,
  stretched,
  toggleKey,
} from "./fields";
import { CanvasFill } from "./CanvasFill";
import { hasCanvas } from "./canvasRequest";

/** The kinds of clip the inspector names; others show the engine's kind. */
const KINDS = new Set(["video", "audio", "image", "text", "caption", "adjustment", "shape", "timeline"]);

/** What the clip is, in the interface's language. */
function kindName(clip: ClipView): string {
  if (isMulticam(clip)) return t("inspector.kind.multicam");
  return KINDS.has(clip.kind) ? t(`inspector.kind.${clip.kind}` as MessageKey) : clip.kind;
}

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

const EASING_NAMES = ["custom", ...Object.keys(EASINGS)];

/** A diamond that adds or removes the property's keyframe at the playhead. */
function KeyButton({ field, onToggle }: { field: Field; onToggle: () => void }) {
  if (!field.animatable) return <span />;
  const state = field.keyed ? "on" : field.animated ? "animated" : "off";
  const title = field.keyed
    ? t("inspector.key.remove")
    : field.animated
      ? t("inspector.key.add")
      : t("inspector.key.animate");
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
  name,
  onChange,
}: {
  label: string;
  value: string;
  options: string[];
  /** An option's name in the interface's language. */
  name: (option: string) => string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="choice">
      <span>{label}</span>
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        {options.map((option) => (
          <option key={option} value={option}>
            {name(option)}
          </option>
        ))}
      </select>
    </label>
  );
}

/** A name of the catalogue under `prefix`, or the option as written when there is none. */
function named(prefix: string, known: string[]): (option: string) => string {
  return (option) => (known.includes(option) ? t(`${prefix}.${option}` as MessageKey) : option.replaceAll("_", " "));
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
        title={t("inspector.key.previous")}
        disabled={previous === undefined}
        onClick={() => previous !== undefined && void editor.seek(previous)}
      >
        {t("inspector.key.back")}
      </button>
      <button
        title={t("inspector.key.next")}
        disabled={next === undefined}
        onClick={() => next !== undefined && void editor.seek(next)}
      >
        {t("inspector.key.forward")}
      </button>
      {easing !== null && (
        <Choice
          label={t("inspector.easing")}
          value={easing}
          options={easing === "custom" ? ["custom", ...Object.keys(EASINGS)] : Object.keys(EASINGS)}
          name={named("inspector.easing", EASING_NAMES)}
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
  media = [],
}: {
  editor: Editor;
  clip: ClipView;
  playhead: number;
  catalog?: Catalog | null;
  luts?: { id: string; name: string }[];
  others?: ClipView[];
  /** The media bin, to tell which video clips have sound. */
  media?: AssetView[];
}) {
  const [separate, setSeparate] = useState(false);
  const viewing: Viewing = { playhead, separate };
  const set = (properties: Record<string, unknown>) => void editor.setProperties(clip.id, properties);
  const transform = clip.transform ?? {};
  const enabled = clip.enabled !== false;
  const targets = targetsOf(clip, media);
  const state = editor.getState();
  const timeline = state.timeline;
  const preferences = preferencesOf(state);
  const rate = parseRate(timeline?.format.rate ?? "30");
  const time = (moment: Moment) => formatTime(moment.flicks, rate, preferences.timeline.time_display);
  // Every selected clip with sound levels together.
  const sounding = editor
    .selected()
    .map((id) => (timeline ? findClip(timeline, id) : null))
    .filter((c): c is ClipView => c !== null && playsSound(c, media))
    .map((c) => c.id);
  return (
    <section className={`inspector${enabled ? "" : " disabled"}`}>
      <h2>{kindName(clip)}</h2>
      <p className="name">{clip.text ?? clip.asset_name ?? clip.timeline_name ?? clip.id}</p>
      <dl className="times">
        <dt>{t("inspector.start")}</dt>
        <dd>{time(clip.start)}</dd>
        <dt>{t("inspector.end")}</dt>
        <dd>{time(clip.end)}</dd>
        <dt>{t("inspector.length")}</dt>
        <dd>{time(clip.duration)}</dd>
      </dl>
      <label className="toggle">
        <input type="checkbox" checked={enabled} onChange={(event) => set({ enabled: event.target.checked })} />
        <span>{hint(t("inspector.active"), "timeline.toggle_enabled")}</span>
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
            <span>{t("inspector.uniform")}</span>
          </label>
          <div className="buttons">
            <button
              className={transform.flip_horizontal ? "active" : ""}
              aria-pressed={transform.flip_horizontal === true}
              onClick={() => set({ "transform.flip_horizontal": !transform.flip_horizontal })}
            >
              {t("inspector.mirror")}
            </button>
            <button
              className={transform.flip_vertical ? "active" : ""}
              aria-pressed={transform.flip_vertical === true}
              onClick={() => set({ "transform.flip_vertical": !transform.flip_vertical })}
            >
              {t("inspector.flip")}
            </button>
          </div>
          <Choice
            label={t("inspector.blend")}
            value={clip.blend ?? "normal"}
            options={BLENDS}
            name={named("inspector.blend", BLENDS)}
            onChange={(blend) => set({ blend })}
          />
          <div className="buttons align" aria-label={t("inspector.align")}>
            {hasPicture(clip) && (
              <button
                title={t("inspector.align.left.title")}
                onClick={() => void editor.align(clip.id, { horizontal: "left" })}
              >
                {t("inspector.align.left")}
              </button>
            )}
            <button
              title={t("inspector.align.centre.title")}
              onClick={() => void editor.align(clip.id, { horizontal: "center", vertical: "center" })}
            >
              {t("inspector.align.centre")}
            </button>
            {hasPicture(clip) && (
              <>
                <button
                  title={t("inspector.align.right.title")}
                  onClick={() => void editor.align(clip.id, { horizontal: "right" })}
                >
                  {t("inspector.align.right")}
                </button>
                <button title={t("inspector.align.top.title")} onClick={() => void editor.align(clip.id, { vertical: "top" })}>
                  {t("inspector.align.top")}
                </button>
                <button
                  title={t("inspector.align.bottom.title")}
                  onClick={() => void editor.align(clip.id, { vertical: "bottom" })}
                >
                  {t("inspector.align.bottom")}
                </button>
              </>
            )}
          </div>
        </div>
      )}
      {hasCanvas(clip) && <CanvasFill key={`canvas:${clip.id}`} editor={editor} clip={clip} />}
      {hasSound(clip) && (
        <div className="options">
          <Choice
            label={t("inspector.channels")}
            value={clip.audio?.channels ?? "stereo"}
            options={CHANNELS}
            name={named("inspector.channel", CHANNELS)}
            onChange={(channels) => set({ "audio.channels": channels })}
          />
          {clip.kind === "video" && (
            <div className="buttons">
              <button
                title={clip.audio_detached ? t("inspector.audio.restore.title") : t("inspector.audio.extract.title")}
                onClick={() => void editor.toggleExtractedAudio(clip.id)}
              >
                {clip.audio_detached ? t("inspector.audio.restore") : t("inspector.audio.extract")}
              </button>
            </div>
          )}
        </div>
      )}
      {targets.sound && (
        <div className="options">
          <div className="buttons">
            <button
              title={t("inspector.normalize.title", { target: preferences.editing.loudness_target })}
              onClick={() => void editor.normalizeLoudness(sounding)}
            >
              {sounding.length > 1
                ? t("inspector.normalize.several", { count: sounding.length })
                : t("inspector.normalize")}
            </button>
          </div>
        </div>
      )}
      {clip.kind === "timeline" && clip.timeline && (
        <div className="options">
          <div className="buttons">
            <span className="muted">{t("inspector.compound")}</span>
            <button title={t("inspector.compound.open.title")} onClick={() => void editor.showTimeline(clip.timeline!)}>
              {t("inspector.compound.open")}
            </button>
            <button title={t("inspector.compound.decompose.title")} onClick={() => void editor.decompose(clip.id)}>
              {t("inspector.compound.decompose")}
            </button>
          </div>
        </div>
      )}
      {isMulticam(clip) && <Multicam editor={editor} clip={clip} />}
      {clip.group && (
        <div className="options">
          <div className="buttons">
            <span className="muted">{t("inspector.grouped")}</span>
            <button title={hint(t("inspector.ungroup.title"), "timeline.ungroup")} onClick={() => void editor.ungroup()}>
              {t("inspector.ungroup")}
            </button>
          </div>
        </div>
      )}
      {clip.kind === "text" && <Text key={`text:${clip.id}`} editor={editor} clip={clip} catalog={catalog} />}
      {clip.kind === "shape" && <Shape key={`shape:${clip.id}`} editor={editor} clip={clip} />}
      {(targets.picture || targets.sound) && (
        <Effects
          editor={editor}
          clip={clip}
          playhead={playhead}
          catalog={catalog}
          luts={luts}
          others={others}
          targets={targets}
        />
      )}
      {clip.kind === "video" && clip.asset && <Subject key={`subject:${clip.id}`} editor={editor} clip={clip} />}
      {clip.kind === "caption" && <CaptionTools key={`captions:${clip.id}`} editor={editor} clip={clip} />}
      {hasPicture(clip) && <Masks editor={editor} clip={clip} playhead={playhead} tool={editor.getState().maskTool} />}
      {(clip.kind === "video" || clip.kind === "audio") && <Speed editor={editor} clip={clip} catalog={catalog} />}
      {clip.kind === "video" && timeline && <Measured key={`measured:${clip.id}`} editor={editor} clip={clip} timeline={timeline} />}
      {isPlaced(clip) && editor.getState().timeline && (
        <Motion editor={editor} clip={clip} timeline={editor.getState().timeline!} catalog={catalog} />
      )}
    </section>
  );
}
