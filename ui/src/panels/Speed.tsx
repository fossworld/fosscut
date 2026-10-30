import { useState } from "react";

import type { Editor } from "../app/editor";
import type { Catalog, ClipView } from "../engine/types";

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
      <h2>Speed</h2>
      {frozen ? (
        <p className="hint">A freeze frame: it holds one picture, without sound.</p>
      ) : (
        <>
          <label className="choice">
            <span>Rate</span>
            <input
              type="number"
              className="seconds"
              step={0.1}
              min={0.05}
              max={100}
              disabled={curved}
              value={draft ?? (curved ? "" : String(rate))}
              placeholder={curved ? "curve" : undefined}
              onChange={(event) => setDraft(event.target.value)}
              onBlur={commit}
              onKeyDown={(event) => event.key === "Enter" && commit()}
            />
          </label>
          <label className="choice">
            <span>Curve</span>
            <select
              value=""
              onChange={(event) => event.target.value && set({ preset: event.target.value })}
            >
              <option value="">{curved ? "Custom" : "None"}</option>
              {catalog?.speed_presets.map((p) => (
                <option key={p.preset} value={p.preset} title={p.meaning}>
                  {words(p.preset)}
                </option>
              ))}
            </select>
          </label>
          <label className="toggle">
            <input type="checkbox" checked={speed.reverse === true} onChange={(e) => set({ reverse: e.target.checked })} />
            <span>Reverse</span>
          </label>
          <label className="toggle">
            <input
              type="checkbox"
              checked={speed.pitch !== "shift"}
              onChange={(e) => set({ pitch: e.target.checked ? "preserve" : "shift" })}
            />
            <span>Keep pitch</span>
          </label>
          {clip.kind === "video" && (
            <label className="toggle">
              <input
                type="checkbox"
                checked={speed.interpolate === "blend"}
                onChange={(e) => set({ interpolate: e.target.checked ? "blend" : "nearest" })}
              />
              <span>Smooth slow motion</span>
            </label>
          )}
        </>
      )}
      <div className="buttons">
        {clip.speed && !frozen && <button onClick={() => set({ reset: true })}>Normal speed</button>}
        {clip.kind === "video" && (
          <button onClick={() => void editor.freeze(clip.id)} title="Hold the frame at the playhead for two seconds">
            Freeze frame
          </button>
        )}
      </div>
    </section>
  );
}
