import { useState } from "react";

import { sampleCurve } from "../app/animation";
import type { Editor } from "../app/editor";
import type { Catalog, ClipView, EffectSpec, EffectView, ParamSpec } from "../engine/types";
import { t } from "../i18n";
import {
  type Point,
  type Targets,
  addPoint,
  animated,
  isSoundEffect,
  keyed,
  keyframeable,
  movePoint,
  offeredEffects,
  paramAt,
  paramChange,
  removePoint,
  specOf,
  toggleParamKey,
} from "./effectFields";

const words = (name: string) => name.replaceAll("_", " ");

function Diamond({ on, animated: moving, onToggle }: { on: boolean; animated: boolean; onToggle: () => void }) {
  const title = on ? t("effects.key.remove") : t("effects.key.add");
  return (
    <button className={`key ${on ? "on" : moving ? "animated" : "off"}`} title={title} aria-label={title} onClick={onToggle}>
      <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
        <path d="M5 0.8 9.2 5 5 9.2 0.8 5z" />
      </svg>
    </button>
  );
}

/** A number parameter as a slider with its value. */
function NumberParam({
  spec,
  value,
  onCommit,
}: {
  spec: ParamSpec;
  value: number;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState<number | null>(null);
  const min = spec.min ?? -1;
  const max = spec.max ?? 1;
  const step = (max - min) / 200;
  const shown = draft ?? value;
  return (
    <>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={shown}
        aria-label={words(spec.name)}
        onChange={(event) => setDraft(Number(event.target.value))}
        onPointerUp={() => {
          if (draft !== null) onCommit(draft);
          setDraft(null);
        }}
        onKeyUp={() => {
          if (draft !== null) onCommit(draft);
          setDraft(null);
        }}
      />
      <span className="value">{shown.toFixed(2)}</span>
    </>
  );
}

const CHANNELS = ["master", "red", "green", "blue"] as const;
const SIZE = 120;

/** An editable tone curve: click to add a point, drag to move, double-click to remove. */
function CurveEditor({
  curves,
  onCommit,
}: {
  curves: Record<string, Point[]>;
  onCommit: (channel: string, points: Point[]) => void;
}) {
  const [channel, setChannel] = useState<(typeof CHANNELS)[number]>("master");
  const [drag, setDrag] = useState<{ index: number; points: Point[] } | null>(null);
  const points = drag?.points ?? curves[channel] ?? [[0, 0], [1, 1]];
  const toSvg = ([x, y]: Point) => [x * SIZE, (1 - y) * SIZE] as const;
  const fromEvent = (event: React.PointerEvent<SVGSVGElement>): Point => {
    const box = event.currentTarget.getBoundingClientRect();
    return [(event.clientX - box.left) / box.width, 1 - (event.clientY - box.top) / box.height];
  };
  const line = sampleCurve(points, 64)
    .map((v, i) => `${(i / 63) * SIZE},${(1 - v) * SIZE}`)
    .join(" ");
  return (
    <div className="curves">
      <div className="buttons">
        {CHANNELS.map((c) => (
          <button key={c} className={c === channel ? "active" : ""} onClick={() => setChannel(c)}>
            {t(`effects.channel.${c}`)}
          </button>
        ))}
      </div>
      <svg
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        width={SIZE}
        height={SIZE}
        className={`curve ${channel}`}
        role="img"
        aria-label={t("effects.curve", { channel: t(`effects.channel.${channel}`) })}
        onPointerDown={(event) => {
          if (event.target !== event.currentTarget) return;
          const added = addPoint(points, fromEvent(event));
          if (added) onCommit(channel, added);
        }}
        onPointerMove={(event) => {
          if (drag) setDrag({ ...drag, points: movePoint(drag.points, drag.index, fromEvent(event)) });
        }}
        onPointerUp={() => {
          if (drag) onCommit(channel, drag.points);
          setDrag(null);
        }}
      >
        <line x1="0" y1={SIZE} x2={SIZE} y2="0" className="diagonal" />
        <polyline points={line} />
        {points.map((p, index) => {
          const [cx, cy] = toSvg(p);
          return (
            <circle
              key={index}
              cx={cx}
              cy={cy}
              r="4"
              onPointerDown={(event) => {
                event.currentTarget.ownerSVGElement?.setPointerCapture(event.pointerId);
                setDrag({ index, points });
              }}
              onDoubleClick={() => onCommit(channel, removePoint(points, index))}
            />
          );
        })}
      </svg>
    </div>
  );
}

function Param({
  editor,
  clip,
  effect,
  spec,
  playhead,
  luts,
}: {
  editor: Editor;
  clip: ClipView;
  effect: EffectView;
  spec: ParamSpec;
  playhead: number;
  luts: { id: string; name: string }[];
}) {
  const value = paramAt(clip, effect, spec, playhead);
  const commit = (next: unknown) => {
    const command = paramChange(clip, effect, spec, next, playhead);
    if (command) void editor.edit([command]);
  };
  const diamond = keyframeable(spec) ? (
    <Diamond
      on={keyed(effect, spec.name, playhead)}
      animated={animated(effect, spec.name)}
      onToggle={() => void editor.edit([toggleParamKey(clip, effect, spec, playhead)])}
    />
  ) : (
    <span />
  );
  let control: React.ReactNode;
  if (spec.kind === "number") {
    control = <NumberParam key={String(value)} spec={spec} value={Number(value)} onCommit={commit} />;
  } else if (spec.kind === "pair") {
    const [x, y] = (value as [number, number]) ?? [0, 0];
    control = (
      <>
        <NumberParam key={`x${x}`} spec={spec} value={x} onCommit={(v) => commit([v, y])} />
        <NumberParam key={`y${y}`} spec={spec} value={y} onCommit={(v) => commit([x, v])} />
      </>
    );
  } else if (spec.kind === "asset") {
    const current = (value as { asset?: string } | undefined)?.asset ?? "";
    control = (
      <select value={current} onChange={(event) => commit({ asset: event.target.value })}>
        {current === "" && <option value="">{t("effects.lut.choose")}</option>}
        {luts.map((lut) => (
          <option key={lut.id} value={lut.id}>
            {lut.name}
          </option>
        ))}
      </select>
    );
  } else if (spec.kind === "bool") {
    control = <input type="checkbox" checked={value === true} onChange={(event) => commit(event.target.checked)} />;
  } else if (spec.kind === "choice") {
    control = (
      <select value={String(value)} onChange={(event) => commit(event.target.value)}>
        {(spec.choices ?? []).map((choice) => (
          <option key={choice} value={choice}>
            {words(choice)}
          </option>
        ))}
      </select>
    );
  } else {
    return null;
  }
  return (
    <label className="param" title={spec.meaning}>
      {diamond}
      <span>{words(spec.name)}</span>
      <span className="control">{control}</span>
    </label>
  );
}

function EffectCard({
  editor,
  clip,
  effect,
  spec,
  index,
  count,
  playhead,
  luts,
}: {
  editor: Editor;
  clip: ClipView;
  effect: EffectView;
  spec: EffectSpec | undefined;
  index: number;
  count: number;
  playhead: number;
  luts: { id: string; name: string }[];
}) {
  const [open, setOpen] = useState(true);
  const on = effect.enabled !== false;
  const set = (args: Record<string, unknown>) =>
    void editor.edit([{ command: "effect.set", args: { clip: clip.id, effect: effect.id, ...args } }]);
  const curves = spec?.params.filter((p) => p.kind === "curve") ?? [];
  return (
    <div className={`effect${on ? "" : " off"}`}>
      <div className="effect-head">
        <button className="disclose" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? "▾" : "▸"}
        </button>
        <input
          type="checkbox"
          checked={on}
          title={on ? t("effects.bypass") : t("effects.apply")}
          onChange={() => set({ enabled: !on })}
        />
        <span className="title">{spec?.title ?? effect.type}</span>
        <button
          title={t("effects.earlier")}
          disabled={index === 0}
          onClick={() => void editor.edit([{ command: "effect.move", args: { clip: clip.id, effect: effect.id, index: index - 1 } }])}
        >
          ↑
        </button>
        <button
          title={t("effects.later")}
          disabled={index === count - 1}
          onClick={() => void editor.edit([{ command: "effect.move", args: { clip: clip.id, effect: effect.id, index: index + 1 } }])}
        >
          ↓
        </button>
        <button
          title={t("effects.remove")}
          onClick={() => void editor.edit([{ command: "effect.remove", args: { clip: clip.id, effect: effect.id } }])}
        >
          ×
        </button>
      </div>
      {open && spec && (
        <div className="params">
          {curves.length > 0 && (
            <CurveEditor
              curves={Object.fromEntries(curves.map((p) => [p.name, paramAt(clip, effect, p, playhead) as Point[]]))}
              onCommit={(channel, points) => set({ params: { [channel]: points } })}
            />
          )}
          {spec.params
            .filter((p) => p.kind !== "curve")
            .map((p) => (
              <Param key={p.name} editor={editor} clip={clip} effect={effect} spec={p} playhead={playhead} luts={luts} />
            ))}
        </div>
      )}
    </div>
  );
}

/** The effect types to add, as one list, or grouped when there are both kinds. */
function EffectOptions({ offered }: { offered: EffectSpec[] }) {
  const option = (e: EffectSpec) => (
    <option key={e.type} value={e.type} title={e.summary}>
      {e.title}
    </option>
  );
  const sound = offered.filter(isSoundEffect);
  const picture = offered.filter((e) => !isSoundEffect(e));
  if (sound.length === 0 || picture.length === 0) return <>{offered.map(option)}</>;
  return (
    <>
      <optgroup label={t("effects.group.picture")}>{picture.map(option)}</optgroup>
      <optgroup label={t("effects.group.sound")}>{sound.map(option)}</optgroup>
    </>
  );
}

/**
 * The selected clip's effects in the order they apply, each with its parameters at the
 * playhead, plus adding effects and looks and automatic color. Sound effects show for
 * clips with sound and picture effects for clips with a picture.
 */
export function Effects({
  editor,
  clip,
  playhead,
  catalog,
  luts,
  others,
  targets = { picture: true, sound: false },
}: {
  editor: Editor;
  clip: ClipView;
  playhead: number;
  catalog: Catalog | null;
  luts: { id: string; name: string }[];
  /** Other video clips, to match the color of. */
  others: ClipView[];
  /** What effects can change on this clip. */
  targets?: Targets;
}) {
  const effects = clip.effects ?? [];
  const pictures = clip.kind === "video" || clip.kind === "image";
  const offered = catalog ? offeredEffects(catalog.effects, targets) : [];
  return (
    <section className="effects">
      <h2>{t("effects.title")}</h2>
      {effects.map((effect, index) => (
        <EffectCard
          key={effect.id}
          editor={editor}
          clip={clip}
          effect={effect}
          spec={catalog ? specOf(catalog.effects, effect) : undefined}
          index={index}
          count={effects.length}
          playhead={playhead}
          luts={luts}
        />
      ))}
      <div className="add">
        <select
          value=""
          aria-label={targets.picture ? t("effects.add") : t("effects.add.sound")}
          onChange={(event) => event.target.value && void editor.addEffect(clip.id, event.target.value)}
        >
          <option value="">{targets.picture ? t("effects.add") : t("effects.add.sound")}</option>
          <EffectOptions offered={offered} />
        </select>
        {targets.picture && (
          <select
            value=""
            aria-label={t("effects.look")}
            onChange={(event) => event.target.value && void editor.applyLook(clip.id, event.target.value)}
          >
            <option value="">{t("effects.look")}</option>
            {catalog?.looks.map((l) => (
              <option key={l.name} value={l.name} title={l.description}>
                {l.title}
              </option>
            ))}
          </select>
        )}
      </div>
      {pictures && (
        <div className="add">
          <button title={t("effects.auto.title")} onClick={() => void editor.autoColor(clip.id)}>
            {t("effects.auto")}
          </button>
          <select
            value=""
            aria-label={t("effects.match.title")}
            disabled={others.length === 0}
            onChange={(event) => event.target.value && void editor.autoColor(clip.id, event.target.value)}
          >
            <option value="">{t("effects.match")}</option>
            {others.map((c) => (
              <option key={c.id} value={c.id}>
                {t("effects.match.clip", { name: c.asset_name ?? c.id, time: c.start.timecode })}
              </option>
            ))}
          </select>
        </div>
      )}
    </section>
  );
}
