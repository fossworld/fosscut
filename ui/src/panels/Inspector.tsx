import {
  AlignCenter,
  ArrowDownToLine,
  ArrowLeftToLine,
  ArrowRightToLine,
  ArrowUpToLine,
  AudioLines,
  Blend,
  ChevronLeft,
  ChevronRight,
  FlipHorizontal2,
  FlipVertical2,
  Gauge,
  History,
  Info,
  Languages,
  type LucideIcon,
  Palette,
  PersonStanding,
  Shapes,
  Sparkles,
  SquareDashedMousePointer,
  Subtitles,
  Type,
  Wand2,
  Waves,
} from "lucide-react";
import { useEffect, useState } from "react";

import { EASINGS, type EasingName, easingName, keyAt, keyTimes } from "../app/animation";
import { type Editor, type EditorState, preferencesOf } from "../app/editor";
import { aspectName, useInspectorRequest } from "../app/layout";
import { isMulticam } from "../app/multicam";
import { readableSize } from "../app/Start";
import type { ClipView, Moment } from "../engine/types";
import { type MessageKey, formatNumber, t } from "../i18n";
import { hint } from "../shortcuts";
import { findClip } from "../timeline/geometry";
import { formatTime, parseRate } from "../transport/time";
import { Glyph, IconButton, Tabs, cx } from "../ui/controls";
import { kindIcon } from "../ui/kinds";
import { Activity } from "./Activity";
import { CanvasFill } from "./CanvasFill";
import { hasCanvas } from "./canvasRequest";
import { CaptionTools } from "./CaptionTools";
import { Effects } from "./Effects";
import { targetsOf } from "./effectFields";
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
import { type InspectorTab, basicTabName, fieldsIn, inspectorTabs, shownTab, sliderRange } from "./inspectorTabs";
import { LibraryItems } from "./Library";
import { Masks } from "./Masks";
import { Measured } from "./Measured";
import { Motion } from "./Motion";
import { Multicam } from "./Multicam";
import { Shape } from "./Shape";
import { Beats, Reframe, Shorts, VoicePresets } from "./Smart";
import { Speed } from "./Speed";
import { Subject } from "./Subject";
import { TextContent, TextReveal, TextStyleFields } from "./Text";

/** The kinds of clip the inspector names; others show the engine's kind. */
const KINDS = new Set(["video", "audio", "image", "text", "caption", "adjustment", "shape", "timeline"]);

/** What the clip is, in the interface's language. */
function kindName(clip: ClipView): string {
  if (isMulticam(clip)) return t("inspector.kind.multicam");
  return KINDS.has(clip.kind) ? t(`inspector.kind.${clip.kind}` as MessageKey) : clip.kind;
}

const TAB_ICONS: Record<InspectorTab, LucideIcon> = {
  basic: SquareDashedMousePointer,
  text: Type,
  style: Palette,
  shape: Shapes,
  captions: Subtitles,
  translate: Languages,
  audio: AudioLines,
  speed: Gauge,
  animation: Waves,
  adjust: Palette,
  masks: SquareDashedMousePointer,
  effects: Sparkles,
  subject: PersonStanding,
  tools: Wand2,
  shorts: Sparkles,
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

const EASING_NAMES = ["custom", ...Object.keys(EASINGS)];

/** A diamond that adds or removes the property's keyframe at the playhead. */
function KeyButton({ field, onToggle }: { field: Field; onToggle: () => void }) {
  if (!field.animatable) return <span className="key-space" />;
  const state = field.keyed ? "on" : field.animated ? "animated" : "off";
  const title = field.keyed
    ? t("inspector.key.remove")
    : field.animated
      ? t("inspector.key.add")
      : t("inspector.key.animate");
  return (
    <button type="button" className={`key ${state}`} title={title} aria-label={title} onClick={onToggle}>
      <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
        <path d="M5 0.8 9.2 5 5 9.2 0.8 5z" />
      </svg>
    </button>
  );
}

/**
 * One numeric property: its name, a slider over the range people use, the value to type,
 * its unit, and the keyframe diamond. Dragging shows the value and commits once, on release.
 */
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
  const [sliding, setSliding] = useState<number | null>(null);
  const commit = () => {
    if (draft !== null && draft.trim() !== "" && Number.isFinite(Number(draft))) onCommit(Number(draft));
    setDraft(null);
  };
  const release = () => {
    if (sliding !== null && sliding !== field.value) onCommit(sliding);
    setSliding(null);
  };
  const [low, high] = sliderRange(field);
  const shown = sliding ?? field.value;
  return (
    <div className={cx("field-row", field.animated && "animated")}>
      <span className="field-label">{field.label}</span>
      <input
        type="range"
        className="slider"
        min={low}
        max={high}
        step={field.step}
        value={Math.min(high, Math.max(low, shown))}
        aria-label={field.label}
        onChange={(event) => setSliding(Number(event.target.value))}
        onPointerUp={release}
        onKeyUp={release}
        onBlur={release}
      />
      <span className="number">
        <input
          type="number"
          step={field.step}
          min={field.min}
          max={field.max}
          aria-label={field.label}
          value={draft ?? String(sliding ?? field.value)}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
            if (event.key === "Escape") setDraft(null);
          }}
        />
        <span className="unit">{field.unit}</span>
      </span>
      <KeyButton field={field} onToggle={onToggle} />
    </div>
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
      <span className="keyframes-label">{t("inspector.keyframes")}</span>
      <IconButton
        icon={ChevronLeft}
        size="sm"
        label={t("inspector.key.previous")}
        disabled={previous === undefined}
        onClick={() => previous !== undefined && void editor.seek(previous)}
      />
      <IconButton
        icon={ChevronRight}
        size="sm"
        label={t("inspector.key.next")}
        disabled={next === undefined}
        onClick={() => next !== undefined && void editor.seek(next)}
      />
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

/** Numeric fields with their sliders, committing through the clip's keyframes or values. */
function FieldList({ editor, clip, fields, viewing }: { editor: Editor; clip: ClipView; fields: Field[]; viewing: Viewing }) {
  if (fields.length === 0) return null;
  return (
    <div className="field-rows" key={clip.id}>
      {fields.map((field) => (
        <NumberField
          key={`${field.id}:${field.value}`}
          field={field}
          onCommit={(value) => {
            const command = change(clip, field.id, value, viewing);
            if (command) void editor.edit([command]);
          }}
          onToggle={() => void editor.edit([toggleKey(clip, field.path, viewing.playhead)])}
        />
      ))}
    </div>
  );
}

/** Where the clip sits on the canvas: position, scale, rotation, opacity, blend, mirroring, alignment and crop. */
function Placement({
  editor,
  clip,
  viewing,
  separate,
  onSeparate,
}: {
  editor: Editor;
  clip: ClipView;
  viewing: Viewing;
  separate: boolean;
  onSeparate: (on: boolean) => void;
}) {
  const set = (properties: Record<string, unknown>) => void editor.setProperties(clip.id, properties);
  const transform = clip.transform ?? {};
  const fields = fieldsFor(clip, viewing);
  const placement = fieldsIn(fields, "placement");
  const crop = fieldsIn(fields, "crop");
  const align = (title: MessageKey, icon: LucideIcon, place: { horizontal?: string; vertical?: string }) => (
    <IconButton icon={icon} size="sm" label={t(title)} onClick={() => void editor.align(clip.id, place)} />
  );
  return (
    <>
      {placement.length > 0 && (
        <section className="effects">
          <h2>{t("inspector.group.transform")}</h2>
          <FieldList editor={editor} clip={clip} fields={placement} viewing={viewing} />
          <div className="options">
            <label className="toggle">
              <input
                type="checkbox"
                checked={!(separate || stretched(clip))}
                disabled={stretched(clip)}
                onChange={(event) => onSeparate(!event.target.checked)}
              />
              <span>{t("inspector.uniform")}</span>
            </label>
            <div className="buttons tool-row">
              <span className="segmented-icons" role="group" aria-label={t("inspector.group.mirror")}>
                <IconButton
                  icon={FlipHorizontal2}
                  size="sm"
                  label={t("inspector.mirror")}
                  pressed={transform.flip_horizontal === true}
                  onClick={() => set({ "transform.flip_horizontal": !transform.flip_horizontal })}
                />
                <IconButton
                  icon={FlipVertical2}
                  size="sm"
                  label={t("inspector.flip")}
                  pressed={transform.flip_vertical === true}
                  onClick={() => set({ "transform.flip_vertical": !transform.flip_vertical })}
                />
              </span>
              <span className="segmented-icons" role="group" aria-label={t("inspector.align")}>
                {hasPicture(clip) && align("inspector.align.left.title", ArrowLeftToLine, { horizontal: "left" })}
                {align("inspector.align.centre.title", AlignCenter, { horizontal: "center", vertical: "center" })}
                {hasPicture(clip) && align("inspector.align.right.title", ArrowRightToLine, { horizontal: "right" })}
                {hasPicture(clip) && align("inspector.align.top.title", ArrowUpToLine, { vertical: "top" })}
                {hasPicture(clip) && align("inspector.align.bottom.title", ArrowDownToLine, { vertical: "bottom" })}
              </span>
            </div>
          </div>
        </section>
      )}
      <section className="effects">
        <h2>
          <span>{t("inspector.group.blend")}</span>
          <Glyph icon={Blend} size="sm" />
        </h2>
        <FieldList editor={editor} clip={clip} fields={fieldsIn(fields, "opacity")} viewing={viewing} />
        {isPlaced(clip) && (
          <Choice
            label={t("inspector.blend")}
            value={clip.blend ?? "normal"}
            options={BLENDS}
            name={named("inspector.blend", BLENDS)}
            onChange={(blend) => set({ blend })}
          />
        )}
      </section>
      {crop.length > 0 && (
        <section className="effects" data-part="crop">
          <h2>{t("inspector.group.crop")}</h2>
          <FieldList editor={editor} clip={clip} fields={crop} viewing={viewing} />
        </section>
      )}
      {hasCanvas(clip) && <CanvasFill key={`canvas:${clip.id}`} editor={editor} clip={clip} />}
    </>
  );
}

/** Groups, compound clips and multicam angles: what the clip is part of. */
function Structure({ editor, clip }: { editor: Editor; clip: ClipView }) {
  return (
    <>
      {clip.kind === "timeline" && clip.timeline && (
        <div className="options">
          <div className="buttons">
            <span className="muted">{t("inspector.compound")}</span>
            <span className="spacer" />
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
            <span className="spacer" />
            <button title={hint(t("inspector.ungroup.title"), "timeline.ungroup")} onClick={() => void editor.ungroup()}>
              {t("inspector.ungroup")}
            </button>
          </div>
        </div>
      )}
    </>
  );
}

/** The clip's sound: volume, pan, fades, channels, its own audio clip, loudness and sound effects. */
function Sound({ editor, state, clip, viewing }: { editor: Editor; state: EditorState; clip: ClipView; viewing: Viewing }) {
  const set = (properties: Record<string, unknown>) => void editor.setProperties(clip.id, properties);
  const preferences = preferencesOf(state);
  const timeline = state.timeline;
  const targets = targetsOf(clip, state.media);
  // Every selected clip with sound levels together.
  const sounding = editor
    .selected()
    .map((id) => (timeline ? findClip(timeline, id) : null))
    .filter((c): c is ClipView => c !== null && playsSound(c, state.media))
    .map((c) => c.id);
  return (
    <>
      <section className="effects">
        <h2>{t("inspector.group.sound")}</h2>
        <FieldList editor={editor} clip={clip} fields={fieldsIn(fieldsFor(clip, viewing), "sound")} viewing={viewing} />
        {hasSound(clip) && (
          <Choice
            label={t("inspector.channels")}
            value={clip.audio?.channels ?? "stereo"}
            options={CHANNELS}
            name={named("inspector.channel", CHANNELS)}
            onChange={(channels) => set({ "audio.channels": channels })}
          />
        )}
        <div className="buttons">
          {clip.kind === "video" && (
            <button
              title={clip.audio_detached ? t("inspector.audio.restore.title") : t("inspector.audio.extract.title")}
              onClick={() => void editor.toggleExtractedAudio(clip.id)}
            >
              {clip.audio_detached ? t("inspector.audio.restore") : t("inspector.audio.extract")}
            </button>
          )}
          {targets.sound && (
            <button
              title={t("inspector.normalize.title", { target: preferences.editing.loudness_target })}
              onClick={() => void editor.normalizeLoudness(sounding)}
            >
              {sounding.length > 1 ? t("inspector.normalize.several", { count: sounding.length }) : t("inspector.normalize")}
            </button>
          )}
        </div>
      </section>
      {targets.sound && <VoicePresets editor={editor} state={state} clip={clip} />}
      {targets.sound && (
        <Effects
          editor={editor}
          clip={clip}
          playhead={state.playhead}
          catalog={state.catalog}
          luts={state.luts}
          others={[]}
          targets={{ picture: false, sound: true }}
          scope="sound"
        />
      )}
      {clip.kind === "audio" && <Beats key={`beats:${clip.id}`} editor={editor} state={state} clip={clip} />}
    </>
  );
}

/** The name of a tab, by the clip it is for. */
function tabName(tab: InspectorTab, clip: ClipView): string {
  if (tab === "basic") return t(`inspector.tab.basic.${basicTabName(clip)}`);
  return t(`inspector.tab.${tab}`);
}

/**
 * The selected clip, in tabs by what it is: its own controls first (placement for pictures,
 * the words for titles, the shape for shapes), then its sound, speed, animation, colour,
 * masks, effects, subject and measured edits. A header over the tabs says what and where it
 * is, and whether it is active.
 */
export function ClipInspector({ editor, state, clip }: { editor: Editor; state: EditorState; clip: ClipView }) {
  const [wanted, setWanted] = useState<InspectorTab | null>(null);
  // A tab asked for from elsewhere (the timeline's crop button), once per request.
  const asked = useInspectorRequest();
  const [answered, setAnswered] = useState(asked?.seq ?? 0);
  if (asked && asked.seq !== answered) {
    setAnswered(asked.seq);
    setWanted(asked.tab as InspectorTab);
  }
  useEffect(() => {
    if (!asked?.part) return;
    document.querySelector(`.inspector-body [data-part="${asked.part}"]`)?.scrollIntoView({ block: "start", behavior: "smooth" });
  }, [asked]);
  const [separate, setSeparate] = useState(false);
  const playhead = state.playhead;
  const viewing: Viewing = { playhead, separate };
  const timeline = state.timeline;
  const rate = parseRate(timeline?.format.rate ?? "30");
  const time = (moment: Moment) => formatTime(moment.flicks, rate, preferencesOf(state).timeline.time_display);
  const enabled = clip.enabled !== false;
  const tabs = inspectorTabs(clip, state.media);
  const tab = shownTab(tabs, wanted);
  const targets = targetsOf(clip, state.media);
  const others = (timeline?.tracks ?? [])
    .flatMap((track) => track.clips)
    .filter((c) => c.id !== clip.id && (c.kind === "video" || c.kind === "image"));
  const effects = (scope: "color" | "picture") => (
    <Effects
      editor={editor}
      clip={clip}
      playhead={playhead}
      catalog={state.catalog}
      luts={state.luts}
      others={others}
      targets={{ picture: targets.picture, sound: false }}
      scope={scope}
    />
  );
  let body: React.ReactNode = null;
  switch (tab) {
    case "basic":
      body = (
        <>
          <Keyframes editor={editor} clip={clip} playhead={playhead} />
          <Placement editor={editor} clip={clip} viewing={viewing} separate={separate} onSeparate={setSeparate} />
          <Structure editor={editor} clip={clip} />
        </>
      );
      break;
    case "text":
      body = <TextContent key={`text:${clip.id}`} editor={editor} clip={clip} catalog={state.catalog} />;
      break;
    case "style":
      body = (
        <>
          {clip.kind === "text" && <TextStyleFields key={`style:${clip.id}`} editor={editor} clip={clip} catalog={state.catalog} />}
          <Keyframes editor={editor} clip={clip} playhead={playhead} />
          <Placement editor={editor} clip={clip} viewing={viewing} separate={separate} onSeparate={setSeparate} />
          <Structure editor={editor} clip={clip} />
        </>
      );
      break;
    case "shape":
      body = <Shape key={`shape:${clip.id}`} editor={editor} clip={clip} />;
      break;
    case "captions":
      body = (
        <>
          <section className="effects">
            <h2>{t("inspector.caption.page")}</h2>
            <p className="caption-words">{clip.text}</p>
            <p className="hint">{t("inspector.caption.help")}</p>
          </section>
          <section className="effects">
            <h2>{t("media.library.plural.caption_style")}</h2>
            <LibraryItems editor={editor} state={state} kinds={["caption_style"]} compact />
          </section>
          <Structure editor={editor} clip={clip} />
        </>
      );
      break;
    case "translate":
      body = <CaptionTools key={`captions:${clip.id}`} editor={editor} clip={clip} />;
      break;
    case "audio":
      body = <Sound editor={editor} state={state} clip={clip} viewing={viewing} />;
      break;
    case "speed":
      body = <Speed editor={editor} clip={clip} catalog={state.catalog} />;
      break;
    case "animation":
      body = (
        <>
          <Keyframes editor={editor} clip={clip} playhead={playhead} />
          {clip.kind === "text" && <TextReveal key={`reveal:${clip.id}`} editor={editor} clip={clip} catalog={state.catalog} />}
          {isPlaced(clip) && timeline && <Motion editor={editor} clip={clip} timeline={timeline} catalog={state.catalog} />}
        </>
      );
      break;
    case "adjust":
      body = (
        <>
          {clip.kind === "adjustment" && <FieldList editor={editor} clip={clip} fields={fieldsIn(fieldsFor(clip, viewing), "opacity")} viewing={viewing} />}
          {effects("color")}
        </>
      );
      break;
    case "masks":
      body = <Masks editor={editor} clip={clip} playhead={playhead} tool={state.maskTool} />;
      break;
    case "effects":
      body = effects("picture");
      break;
    case "subject":
      body = <Subject key={`subject:${clip.id}`} editor={editor} clip={clip} />;
      break;
    case "tools":
      body = (
        <>
          {clip.asset && <Reframe key={`reframe:${clip.id}`} editor={editor} state={state} clip={clip} />}
          {timeline && <Measured key={`measured:${clip.id}`} editor={editor} clip={clip} timeline={timeline} />}
        </>
      );
      break;
    case "shorts":
      body = <Shorts key={`shorts:${clip.id}`} editor={editor} state={state} clip={clip} />;
      break;
  }
  return (
    <div className={cx("inspector", !enabled && "disabled")}>
      <header className="inspector-head">
        <span className={`kind-badge kind-${isMulticam(clip) ? "timeline" : clip.kind}`}>
          <Glyph icon={kindIcon(clip.kind)} size="sm" />
        </span>
        <div className="inspector-title">
          <p className="name" title={clip.text ?? clip.asset_name ?? clip.timeline_name ?? clip.id}>
            {clip.text ?? clip.asset_name ?? clip.timeline_name ?? clip.id}
          </p>
          <p className="kind">
            {kindName(clip)}
            <span className="times" title={`${t("inspector.start")} ${time(clip.start)}, ${t("inspector.end")} ${time(clip.end)}`}>
              {time(clip.start)} · {time(clip.duration)}
            </span>
          </p>
        </div>
        <label className="toggle compact" title={hint(t("inspector.active"), "timeline.toggle_enabled")}>
          <span className="sr-only">{t("inspector.active")}</span>
          <input type="checkbox" checked={enabled} onChange={(event) => void editor.setProperties(clip.id, { enabled: event.target.checked })} />
        </label>
      </header>
      {tabs.length > 1 && (
        <Tabs
          scroll={{ earlier: t("app.scroll.earlier"), later: t("app.scroll.later") }}
          label={t("inspector.tabs")}
          className="inspector-tabs"
          value={tab}
          onChange={setWanted}
          tabs={tabs.map((id) => ({ id, label: tabName(id, clip), icon: TAB_ICONS[id] }))}
        />
      )}
      <div className="inspector-body" role="tabpanel" aria-labelledby={`tab-${tab}`} key={`${clip.id}:${tab}`}>
        {body}
      </div>
    </div>
  );
}

/** One line of the project's details. */
function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="kv-row">
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

/** With nothing selected: the project and the shown timeline, and who changed what. */
export function ProjectInspector({ editor, state }: { editor: Editor; state: EditorState }) {
  const [tab, setTab] = useState<"project" | "activity">("project");
  const info = state.opened?.info;
  const timeline = state.timeline;
  if (!info) return null;
  const rate = parseRate(timeline?.format.rate ?? "30");
  const clips = timeline?.tracks.reduce((sum, track) => sum + track.clips.length, 0) ?? 0;
  const summary = state.projects.find((p) => p.path === state.opened?.bundle);
  const format = timeline?.format;
  return (
    <div className="inspector">
      <header className="inspector-head">
        <span className="kind-badge kind-project">
          <Glyph icon={Info} size="sm" />
        </span>
        <div className="inspector-title">
          <p className="name">{info.name}</p>
          <p className="kind">{t("inspector.project.nothing")}</p>
        </div>
      </header>
      <Tabs
        scroll={{ earlier: t("app.scroll.earlier"), later: t("app.scroll.later") }}
        label={t("inspector.tabs")}
        className="inspector-tabs"
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "project", label: t("inspector.tab.project"), icon: Info },
          { id: "activity", label: t("inspector.tab.activity"), icon: History },
        ]}
      />
      <div className="inspector-body" role="tabpanel" aria-labelledby={`tab-${tab}`}>
        {tab === "project" ? (
          <>
            <section className="effects">
              <h2>{t("inspector.project.timeline")}</h2>
              <dl className="kv">
                <Row label={t("inspector.project.name")} value={timeline?.name ?? ""} />
                {format && <Row label={t("inspector.project.canvas")} value={`${format.width} × ${format.height} (${aspectName(format.width, format.height)})`} />}
                {format && <Row label={t("inspector.project.rate")} value={t("inspector.project.fps", { rate: formatNumber(rate.num / rate.den) })} />}
                <Row label={t("inspector.length")} value={formatTime(timeline?.duration.flicks ?? 0, rate, preferencesOf(state).timeline.time_display)} />
                <Row label={t("inspector.project.tracks")} value={String(timeline?.tracks.length ?? 0)} />
                <Row label={t("inspector.project.clips")} value={String(clips)} />
                <Row label={t("inspector.project.markers")} value={String(timeline?.markers?.length ?? 0)} />
              </dl>
            </section>
            <section className="effects">
              <h2>{t("inspector.project.title")}</h2>
              <dl className="kv">
                <Row label={t("inspector.project.timelines")} value={String(info.timelines.filter((tl) => tl.role !== "compound").length)} />
                <Row label={t("inspector.project.media")} value={String(info.assets)} />
                <Row label={t("inspector.project.revision")} value={String(info.revision)} />
                {summary && <Row label={t("inspector.project.size")} value={readableSize(summary.bytes)} />}
                <Row label={t("inspector.project.where")} value={<span className="path" title={state.opened?.bundle}>{state.opened?.bundle}</span>} />
              </dl>
            </section>
            <p className="hint">{t("inspector.project.hint")}</p>
          </>
        ) : (
          <Activity editor={editor} state={state} />
        )}
      </div>
    </div>
  );
}

/** The right-hand panel: the selected clip's inspector, or the project's details. */
export function InspectorPanel({ editor, state }: { editor: Editor; state: EditorState }) {
  const selected = state.timeline && state.selection ? findClip(state.timeline, state.selection) : null;
  return (
    <aside className="panel inspector-panel" aria-label={t("inspector.label")}>
      {selected ? <ClipInspector key={selected.kind} editor={editor} state={state} clip={selected} /> : <ProjectInspector editor={editor} state={state} />}
    </aside>
  );
}
