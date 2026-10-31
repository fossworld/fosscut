import { useState } from "react";

import type { Editor, Measurement, RegionPick } from "../app/editor";
import { Problem } from "../app/Export";
import { trackTargets } from "../app/measure";
import type { ClipView, TimelineView } from "../engine/types";
import { t } from "../i18n";
import { frameAt, parseRate, timecode } from "../transport/time";

/** What a measurement of `kind` on `clip` found, or why it failed, while it is the last. */
export function Outcome({ editor, measurement, kind, clip }: { editor: Editor; measurement: Measurement | null; kind: Measurement["kind"]; clip: string | null }) {
  if (!measurement || measurement.kind !== kind || measurement.clip !== clip || measurement.busy) return null;
  return (
    <div className="outcome">
      {measurement.error ? <Problem error={measurement.error} /> : <p className="hint">{measurement.report}</p>}
      <button className="link" onClick={() => editor.dismissMeasurement()}>
        {t("motion.dismiss")}
      </button>
    </div>
  );
}

/** The region drawn, in words: its size in the picture and the frame it was drawn on. */
function drawnAt(pick: RegionPick, timeline: TimelineView): string {
  const region = pick.region;
  if (!region) return "";
  const rate = parseRate(timeline.format.rate);
  const percent = (fraction: number) => `${Math.round(fraction * 100)}%`;
  return t("motion.region", {
    width: percent(region.width),
    height: percent(region.height),
    time: timecode(frameAt(pick.at, rate), rate),
  });
}

/**
 * Edits a video clip measured from its media: stabilizing it, splitting it at its scene
 * changes, and tracking a region drawn on the preview with a mask or another clip.
 */
export function Measured({ editor, clip, timeline }: { editor: Editor; clip: ClipView; timeline: TimelineView }) {
  const [strength, setStrength] = useState(0.5);
  const [target, setTarget] = useState("");
  const { measurement, regionPick } = editor.getState();
  const busy = measurement?.busy === true;
  const running = (kind: Measurement["kind"]) => busy && measurement?.kind === kind && measurement.clip === clip.id;
  const pick = regionPick?.clip === clip.id ? regionPick : null;
  const targets = trackTargets(clip, timeline);
  const chosen = targets.some((option) => option.value === target) ? target : "";
  return (
    <section className="effects measured">
      <h2>{t("motion.stabilize")}</h2>
      <label className="param">
        <span />
        <span>{t("motion.stabilize.strength")}</span>
        <span className="control">
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={strength}
            disabled={busy}
            aria-label={t("motion.stabilize.strength.label")}
            onChange={(event) => setStrength(Number(event.target.value))}
          />
          <span className="value">{strength.toFixed(2)}</span>
        </span>
      </label>
      <div className="buttons">
        <button
          disabled={busy}
          title={t("motion.stabilize.help")}
          onClick={() => void editor.stabilize(clip.id, strength)}
        >
          {running("stabilize") ? t("motion.stabilize.busy") : t("motion.stabilize")}
        </button>
      </div>
      <Outcome editor={editor} measurement={measurement} kind="stabilize" clip={clip.id} />

      <h2>{t("motion.scenes")}</h2>
      <div className="buttons">
        <button
          disabled={busy}
          title={t("motion.scenes.help")}
          onClick={() => void editor.splitAtScenes(clip.id)}
        >
          {running("scenes") ? t("motion.scenes.busy") : t("motion.scenes.split")}
        </button>
      </div>
      <Outcome editor={editor} measurement={measurement} kind="scenes" clip={clip.id} />

      <h2>{t("motion.track")}</h2>
      <div className="buttons">
        <button
          className={pick?.drawing ? "active" : ""}
          aria-pressed={pick?.drawing === true}
          disabled={busy}
          title={t("motion.track.draw.help")}
          onClick={() => (pick?.drawing ? editor.cancelRegion() : void editor.pickRegion(clip.id))}
        >
          {pick?.drawing ? t("motion.track.drawing") : pick?.region ? t("motion.track.redraw") : t("motion.track.draw")}
        </button>
        {pick?.region && (
          <button className="link" onClick={() => editor.cancelRegion()}>
            {t("motion.track.clear")}
          </button>
        )}
      </div>
      {pick?.region && <p className="hint">{drawnAt(pick, timeline)}</p>}
      <label className="choice">
        <span>{t("motion.track.follower")}</span>
        <select value={chosen} aria-label={t("motion.track.follower.label")} onChange={(event) => setTarget(event.target.value)}>
          <option value="">{targets.length === 0 ? t("motion.track.follower.none") : t("motion.track.follower.choose")}</option>
          {targets.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </label>
      <div className="buttons">
        <button
          disabled={busy || !pick?.region || !chosen}
          title={t("motion.track.help")}
          onClick={() => void editor.track(chosen)}
        >
          {running("track") ? t("motion.track.busy") : t("motion.track")}
        </button>
      </div>
      <Outcome editor={editor} measurement={measurement} kind="track" clip={clip.id} />
    </section>
  );
}
