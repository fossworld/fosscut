import { ListOrdered } from "lucide-react";
import { useState } from "react";

import type { Editor } from "../app/editor";
import type { AnimationPreset, Catalog, ClipView, TimelineView } from "../engine/types";
import { type MessageKey, t } from "../i18n";
import { Glyph, Segmented } from "../ui/controls";
import { FLICKS_PER_SECOND } from "../transport/time";

const words = (name: string) => name.replaceAll("_", " ");

/** Where an animation plays. */
export type Place = "in" | "out" | "combo";

/** The orders `clips.stagger` moves clips in. */
export const STAGGER_ORDERS = [
  "listed",
  "reverse",
  "timeline",
  "left_to_right",
  "right_to_left",
  "top_to_bottom",
  "bottom_to_top",
  "center_out",
  "random",
] as const;

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

/** The presets that play at `place`: the engine says where each applies ("in or out", "in", "combo"). */
export function presetsFor(presets: AnimationPreset[], place: Place): AnimationPreset[] {
  return presets.filter((preset) => preset.applies.split(/\s+or\s+|\s*,\s*/).includes(place));
}

/** A number of seconds committed on blur or Enter; `zero` allows 0 (a delay may be none). */
function Seconds({ value, zero = false, label, onCommit }: { value: number; zero?: boolean; label: string; onCommit: (seconds: number) => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    const seconds = Number(draft);
    if (draft !== null && draft.trim() !== "" && Number.isFinite(seconds) && (zero ? seconds >= 0 : seconds > 0) && seconds !== value) onCommit(seconds);
    setDraft(null);
  };
  return (
    <input
      type="number"
      step={0.1}
      min={zero ? 0 : 0.05}
      className="seconds"
      aria-label={label}
      title={label}
      value={draft ?? String(Math.round(value * 100) / 100)}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => event.key === "Enter" && commit()}
    />
  );
}

/**
 * Animating the selected clips one after another (`clips.stagger`): a preset for where it
 * plays, the step between clips, how long each takes and the order they move in.
 */
function Stagger({ editor, catalog, count }: { editor: Editor; catalog: Catalog | null; count: number }) {
  const [place, setPlace] = useState<Place>("in");
  const presets = presetsFor(catalog?.animations ?? [], place);
  const [preset, setPreset] = useState("");
  const [step, setStep] = useState(0.1);
  const [duration, setDuration] = useState(0.5);
  const [order, setOrder] = useState<(typeof STAGGER_ORDERS)[number]>("listed");
  const chosen = presets.some((p) => p.preset === preset) ? preset : (presets[0]?.preset ?? "");
  return (
    <section className="effects stagger" data-part="stagger">
      <h2>
        <span>{t("motion.stagger")}</span>
        <Glyph icon={ListOrdered} size="sm" />
      </h2>
      <p className="hint">{t("motion.stagger.help", { count })}</p>
      <Segmented
        label={t("motion.stagger.place")}
        value={place}
        onChange={setPlace}
        options={(["in", "out", "combo"] as const).map((id) => ({ id, label: t(`motion.animate.${id}`) }))}
      />
      <label className="choice">
        <span>{t("motion.stagger.preset")}</span>
        <select value={chosen} onChange={(event) => setPreset(event.target.value)}>
          {presets.map((a) => (
            <option key={a.preset} value={a.preset} title={a.meaning}>
              {words(a.preset)}
            </option>
          ))}
        </select>
      </label>
      <label className="choice">
        <span>{t("motion.stagger.step")}</span>
        <Seconds value={step} label={t("motion.stagger.step")} onCommit={setStep} />
      </label>
      <label className="choice">
        <span>{t("motion.stagger.duration")}</span>
        <Seconds value={duration} label={t("motion.stagger.duration")} onCommit={setDuration} />
      </label>
      <label className="choice">
        <span>{t("motion.stagger.order")}</span>
        <select value={order} onChange={(event) => setOrder(event.target.value as (typeof STAGGER_ORDERS)[number])}>
          {STAGGER_ORDERS.map((id) => (
            <option key={id} value={id}>
              {t(`motion.stagger.order.${id}` as MessageKey)}
            </option>
          ))}
        </select>
      </label>
      <div className="buttons">
        <span className="spacer" />
        <button type="button" className="primary small" disabled={!chosen} onClick={() => void editor.stagger({ preset: chosen, place, step, duration, order })}>
          {t("motion.stagger.apply", { count })}
        </button>
      </div>
    </section>
  );
}

/** The transition into the next clip, the clip's animations with their delays, and staggering a selection. */
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
  const animation = (place: Place) => clip.animations?.[place];
  const selected = editor.selected().length;
  return (
    <>
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
              label={t("motion.duration")}
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
          const delay = seconds(current?.delay ?? 0);
          return (
            <div key={place} className="animation-row">
              <label className="choice">
                <span>{t(`motion.animate.${place}`)}</span>
                <select
                  value={current?.preset ?? ""}
                  onChange={(event) =>
                    void editor.setAnimation(clip.id, place, event.target.value, current ? seconds(current.duration) : undefined, current ? delay : undefined)
                  }
                >
                  <option value="">{t("motion.none")}</option>
                  {current && !presetsFor(catalog?.animations ?? [], place).some((a) => a.preset === current.preset) && (
                    <option value={current.preset}>{words(current.preset)}</option>
                  )}
                  {presetsFor(catalog?.animations ?? [], place).map((a) => (
                    <option key={a.preset} value={a.preset} title={a.meaning}>
                      {words(a.preset)}
                    </option>
                  ))}
                </select>
                {current && (
                  <Seconds
                    value={seconds(current.duration)}
                    label={t("motion.duration")}
                    onCommit={(s) => void editor.setAnimation(clip.id, place, current.preset, s, delay)}
                  />
                )}
              </label>
              {current && (
                <label className="choice delay" title={t(`motion.delay.${place}.help`)}>
                  <span>{t("motion.delay")}</span>
                  <Seconds
                    value={delay}
                    zero
                    label={t("motion.delay")}
                    onCommit={(d) => void editor.setAnimation(clip.id, place, current.preset, seconds(current.duration), d)}
                  />
                </label>
              )}
            </div>
          );
        })}
      </section>
      {selected > 1 && <Stagger editor={editor} catalog={catalog} count={selected} />}
    </>
  );
}
