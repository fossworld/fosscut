import { useState } from "react";

import type { Editor } from "../app/editor";
import type { Catalog, ClipView, TimelineView } from "../engine/types";
import { t } from "../i18n";
import { FLICKS_PER_SECOND } from "../transport/time";

const words = (name: string) => name.replaceAll("_", " ");

/** The clip that starts where `clip` ends on its track, if any. */
export function nextClip(timeline: TimelineView, clip: ClipView): ClipView | null {
  for (const track of timeline.tracks) {
    const index = track.clips.findIndex((c) => c.id === clip.id);
    if (index === -1) continue;
    const next = track.clips[index + 1];
    return next && next.start.flicks === clip.end.flicks ? next : null;
  }
  return null;
}

function Seconds({ value, onCommit }: { value: number; onCommit: (seconds: number) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    const seconds = Number(draft);
    if (draft !== null && Number.isFinite(seconds) && seconds > 0 && seconds !== value) onCommit(seconds);
    setDraft(null);
  };
  return (
    <input
      type="number"
      step={0.1}
      min={0.05}
      className="seconds"
      value={draft ?? String(Math.round(value * 100) / 100)}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => event.key === "Enter" && commit()}
    />
  );
}

/** The transition into the next clip and the clip's animations. */
export function Motion({
  editor,
  clip,
  timeline,
  catalog,
}: {
  editor: Editor;
  clip: ClipView;
  timeline: TimelineView;
  catalog: Catalog | null;
}) {
  const next = nextClip(timeline, clip);
  const transition = clip.transition;
  const seconds = (flicks: number) => flicks / FLICKS_PER_SECOND;
  const spec = catalog?.transitions.find((candidate) => candidate.type === transition?.type);
  const direction = spec?.params.find((p) => p.name === "direction");
  const presets = (place: "in" | "combo") =>
    (catalog?.animations ?? []).filter((a) => (place === "combo" ? a.applies === "combo" : a.applies !== "combo"));
  const animation = (place: "in" | "out" | "combo") => clip.animations?.[place];
  return (
    <section className="effects motion">
      <h2>{t("motion.title")}</h2>
      <label className="choice" title={next ? t("motion.transition.over") : t("motion.transition.touch")}>
        <span>{t("motion.transition")}</span>
        <select
          value={transition?.type ?? ""}
          disabled={!next}
          onChange={(event) =>
            void editor.setTransition(clip.id, event.target.value, transition ? seconds(transition.duration.flicks) : 0.5)
          }
        >
          <option value="">{t("motion.none")}</option>
          {catalog?.transitions.map((type) => (
            <option key={type.type} value={type.type} title={type.summary}>
              {type.title}
            </option>
          ))}
        </select>
        {transition && (
          <Seconds
            value={seconds(transition.duration.flicks)}
            onCommit={(s) => void editor.setTransition(clip.id, transition.type, s, transition.params ?? {})}
          />
        )}
      </label>
      {transition && direction && (
        <label className="choice">
          <span>{t("motion.direction")}</span>
          <select
            value={String(transition.params?.direction ?? direction.default)}
            onChange={(event) =>
              void editor.setTransition(clip.id, transition.type, seconds(transition.duration.flicks), {
                ...transition.params,
                direction: event.target.value,
              })
            }
          >
            {direction.choices?.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
        </label>
      )}
      {(["in", "out", "combo"] as const).map((place) => {
        const current = animation(place);
        return (
          <label key={place} className="choice">
            <span>{t(`motion.animate.${place}`)}</span>
            <select
              value={current?.preset ?? ""}
              onChange={(event) => void editor.setAnimation(clip.id, place, event.target.value)}
            >
              <option value="">{t("motion.none")}</option>
              {presets(place === "combo" ? "combo" : "in").map((a) => (
                <option key={a.preset} value={a.preset} title={a.meaning}>
                  {words(a.preset)}
                </option>
              ))}
            </select>
            {current && (
              <Seconds
                value={seconds(current.duration)}
                onCommit={(s) => void editor.setAnimation(clip.id, place, current.preset, s)}
              />
            )}
          </label>
        );
      })}
    </section>
  );
}
