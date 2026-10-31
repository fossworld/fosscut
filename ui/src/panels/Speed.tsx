import { useState } from "react";

import type { Editor } from "../app/editor";
import type { Catalog, ClipView, FrameBlend } from "../engine/types";
import { t } from "../i18n";
import { Segmented } from "../ui/controls";

/** The ways slowed footage fills the frames between its own, plainest first. */
export const INTERPOLATIONS: FrameBlend[] = ["nearest", "blend", "flow"];

const words = (name: string) => name.replaceAll("_", " ");

/** The selected clip's speed: rate or curve, reverse, pitch, smooth slow motion, freeze. */
export function Speed({ editor, clip, catalog }: { editor: Editor; clip: ClipView; catalog: Catalog | null }) {
  const speed = clip.speed ?? {};
  const frozen = !speed.curve?.length && speed.rate === 0;
  const curved = Boolean(speed.curve?.length);
  const rate = speed.rate ?? 1;
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    const value = Number(draft);
    if (draft !== null && Number.isFinite(value) && value >= 0.05 && value <= 100 && value !== rate) {
      void editor.setSpeed(clip.id, { rate: value });
    }
    setDraft(null);
  };
  const set = (change: Record<string, unknown>) => void editor.setSpeed(clip.id, change);
  return (
    <section className="effects speed">
      <h2>{t("motion.speed.title")}</h2>
      {frozen ? (
        <p className="hint">{t("motion.speed.frozen")}</p>
      ) : (
        <>
          <label className="choice">
            <span>{t("motion.speed.rate")}</span>
            <input
              type="number"
              className="seconds"
              step={0.1}
              min={0.05}
              max={100}
              disabled={curved}
              value={draft ?? (curved ? "" : String(rate))}
              placeholder={curved ? t("motion.speed.label.curve") : undefined}
              onChange={(event) => setDraft(event.target.value)}
              onBlur={commit}
              onKeyDown={(event) => event.key === "Enter" && commit()}
            />
          </label>
          <label className="choice">
            <span>{t("motion.speed.curve")}</span>
            <select
              value=""
              onChange={(event) => event.target.value && set({ preset: event.target.value })}
            >
              <option value="">{curved ? t("motion.speed.curve.custom") : t("motion.none")}</option>
              {catalog?.speed_presets.map((p) => (
                <option key={p.preset} value={p.preset} title={p.meaning}>
                  {words(p.preset)}
                </option>
              ))}
            </select>
          </label>
          <label className="toggle">
            <input type="checkbox" checked={speed.reverse === true} onChange={(e) => set({ reverse: e.target.checked })} />
            <span>{t("motion.speed.reverse")}</span>
          </label>
          <label className="toggle">
            <input
              type="checkbox"
              checked={speed.pitch !== "shift"}
              onChange={(e) => set({ pitch: e.target.checked ? "preserve" : "shift" })}
            />
            <span>{t("motion.speed.pitch")}</span>
          </label>
          {clip.kind === "video" && (
            <div className="choice" title={t("motion.speed.smooth.help")}>
              <span>{t("motion.speed.smooth")}</span>
              <Segmented
                label={t("motion.speed.smooth")}
                value={speed.interpolate ?? "nearest"}
                onChange={(interpolate: FrameBlend) => set({ interpolate })}
                options={INTERPOLATIONS.map((id) => ({ id, label: t(`motion.speed.interpolate.${id}`), help: t(`motion.speed.interpolate.${id}.help`) }))}
              />
            </div>
          )}
        </>
      )}
      <div className="buttons">
        {clip.speed && !frozen && <button onClick={() => set({ reset: true })}>{t("motion.speed.normal")}</button>}
        {clip.kind === "video" && (
          <button onClick={() => void editor.freeze(clip.id)} title={t("motion.speed.freeze.help")}>
            {t("motion.speed.freeze")}
          </button>
        )}
      </div>
    </section>
  );
}
