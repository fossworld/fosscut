import { Route } from "lucide-react";
import { useState } from "react";

import type { Editor } from "../app/editor";
import type { ClipView, CommandCall, TimelineView } from "../engine/types";
import { t } from "../i18n";
import { Glyph } from "../ui/controls";
import { KeyedFields } from "./Keyed";
import type { KeyedField } from "./keyedFields";

/**
 * The shape clips `clip` can follow (ADR-0036): shape clips of its timeline other than
 * itself that follow nothing themselves, in the order of their tracks.
 */
export function pathsFor(timeline: TimelineView, clip: ClipView): ClipView[] {
  return timeline.tracks
    .flatMap((track) => track.clips)
    .filter((candidate) => candidate.kind === "shape" && candidate.id !== clip.id && !candidate.follow);
}

/** True when the clip may follow a path: it has a picture of its own and nothing follows it. */
export function canFollow(clip: ClipView, timeline: TimelineView): boolean {
  if (["audio", "adjustment", "camera"].includes(clip.kind)) return false;
  const followed = timeline.tracks.some((track) => track.clips.some((c) => c.follow?.clip === clip.id));
  return !followed;
}

/** The `clip.follow` call that changes how `clip` follows. */
export function followCall(clip: string, args: Record<string, unknown>): CommandCall {
  return { command: "clip.follow", args: { clip, ...args } };
}

/** A pair of plain numbers in % of the canvas, committed on blur or Enter. */
function Offset({ value, onCommit }: { value: [number, number]; onCommit: (value: [number, number]) => void }) {
  const [draft, setDraft] = useState<[string, string] | null>(null);
  const shown = (v: number) => String(Math.round(v * 10000) / 100);
  const commit = () => {
    if (draft) {
      const next: [number, number] = [Number(draft[0]) / 100, Number(draft[1]) / 100];
      if (next.every(Number.isFinite) && (next[0] !== value[0] || next[1] !== value[1])) onCommit(next);
    }
    setDraft(null);
  };
  const input = (index: 0 | 1, label: string) => (
    <label className="field" title={t("vector.follow.offset.help")}>
      <span />
      <span>{label}</span>
      <input
        type="number"
        step={1}
        value={draft?.[index] ?? shown(value[index])}
        onChange={(event) => {
          const next: [string, string] = draft ?? [shown(value[0]), shown(value[1])];
          next[index] = event.target.value;
          setDraft([...next]);
        }}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
          if (event.key === "Escape") setDraft(null);
        }}
      />
      <span className="unit">%</span>
    </label>
  );
  return (
    <div className="fields">
      {input(0, t("vector.follow.offset.x"))}
      {input(1, t("vector.follow.offset.y"))}
    </div>
  );
}

/**
 * Moving a clip along a shape clip's path (ADR-0036): which path, how far along it (with
 * keyframe diamonds, so it travels), whether it turns with the path, and an offset from it.
 */
export function Follow({ editor, clip, timeline, playhead }: { editor: Editor; clip: ClipView; timeline: TimelineView; playhead: number }) {
  const paths = pathsFor(timeline, clip);
  const follow = clip.follow;
  if (!follow && paths.length === 0) return null;
  const name = (path: ClipView) => path.text ?? t(`shape.kind.${path.shape ?? "path"}`);
  const progress: KeyedField = {
    path: "follow.progress",
    plain: follow?.progress ?? 0,
    label: t("vector.follow.progress"),
    unit: "%",
    scale: 100,
    range: [0, 100],
    help: t("vector.follow.progress.help"),
  };
  return (
    <section className="effects" data-part="follow">
      <h2>
        <span>{t("vector.follow")}</span>
        <Glyph icon={Route} size="sm" />
      </h2>
      <label className="choice">
        <span>{t("vector.follow.path")}</span>
        <select
          value={follow?.clip ?? ""}
          onChange={(event) =>
            void editor.edit([followCall(clip.id, event.target.value ? { path: event.target.value } : { clear: true })])
          }
        >
          <option value="">{t("vector.follow.none")}</option>
          {follow && !paths.some((p) => p.id === follow.clip) && <option value={follow.clip}>{follow.clip}</option>}
          {paths.map((path) => (
            <option key={path.id} value={path.id}>
              {name(path)}
            </option>
          ))}
        </select>
      </label>
      {follow ? (
        <>
          <KeyedFields
            editor={editor}
            clip={clip}
            playhead={playhead}
            fields={[progress]}
            plain={(_, value) => followCall(clip.id, { progress: value })}
          />
          <label className="toggle" title={t("vector.follow.orient.help")}>
            <input type="checkbox" checked={follow.orient === true} onChange={(event) => void editor.edit([followCall(clip.id, { orient: event.target.checked })])} />
            <span>{t("vector.follow.orient")}</span>
          </label>
          <Offset key={`${clip.id}:${String(follow.offset)}`} value={follow.offset ?? [0, 0]} onCommit={(offset) => void editor.edit([followCall(clip.id, { offset })])} />
        </>
      ) : (
        <p className="hint">{t("vector.follow.help")}</p>
      )}
    </section>
  );
}
