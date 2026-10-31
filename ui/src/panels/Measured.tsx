import { useState } from "react";

import type { Editor, Measurement, RegionPick } from "../app/editor";
import { Problem } from "../app/Export";
import { trackTargets } from "../app/measure";
import type { ClipView, TimelineView } from "../engine/types";
import { frameAt, parseRate, timecode } from "../transport/time";

/** What a measurement of `kind` on `clip` found, or why it failed, while it is the last. */
export function Outcome({ editor, measurement, kind, clip }: { editor: Editor; measurement: Measurement | null; kind: Measurement["kind"]; clip: string | null }) {
  if (!measurement || measurement.kind !== kind || measurement.clip !== clip || measurement.busy) return null;
  return (
    <div className="outcome">
      {measurement.error ? <Problem error={measurement.error} /> : <p className="hint">{measurement.report}</p>}
      <button className="link" onClick={() => editor.dismissMeasurement()}>
        Dismiss
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
  return `${percent(region.width)} by ${percent(region.height)} of the picture, at ${timecode(frameAt(pick.at, rate), rate)}`;
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
  const chosen = targets.some((t) => t.value === target) ? target : "";
  return (
    <section className="effects measured">
      <h2>Stabilize</h2>
      <label className="param">
        <span />
        <span>Strength</span>
        <span className="control">
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={strength}
            disabled={busy}
            aria-label="Stabilization strength"
            onChange={(event) => setStrength(Number(event.target.value))}
          />
          <span className="value">{strength.toFixed(2)}</span>
        </span>
      </label>
      <div className="buttons">
        <button
          disabled={busy}
          title="Measure how the camera moved and key the clip's position, rotation and scale to steady it, zooming in to hide the moving edges"
          onClick={() => void editor.stabilize(clip.id, strength)}
        >
          {running("stabilize") ? "Stabilizing..." : "Stabilize"}
        </button>
      </div>
      <Outcome editor={editor} measurement={measurement} kind="stabilize" clip={clip.id} />

      <h2>Scenes</h2>
      <div className="buttons">
        <button
          disabled={busy}
          title="Find the shot cuts in the clip and split it there, so each shot is a clip of its own"
          onClick={() => void editor.splitAtScenes(clip.id)}
        >
          {running("scenes") ? "Finding scenes..." : "Split at scenes"}
        </button>
      </div>
      <Outcome editor={editor} measurement={measurement} kind="scenes" clip={clip.id} />

      <h2>Track</h2>
      <div className="buttons">
        <button
          className={pick?.drawing ? "active" : ""}
          aria-pressed={pick?.drawing === true}
          disabled={busy}
          title="Drag a rectangle on the preview over what to follow, on the frame at the playhead"
          onClick={() => (pick?.drawing ? editor.cancelRegion() : void editor.pickRegion(clip.id))}
        >
          {pick?.drawing ? "Drawing on the preview" : pick?.region ? "Redraw region" : "Draw region"}
        </button>
        {pick?.region && (
          <button className="link" onClick={() => editor.cancelRegion()}>
            Clear
          </button>
        )}
      </div>
      {pick?.region && <p className="hint">{drawnAt(pick, timeline)}</p>}
      <label className="choice">
        <span>Follower</span>
        <select value={chosen} aria-label="What follows the region" onChange={(event) => setTarget(event.target.value)}>
          <option value="">{targets.length === 0 ? "Add a mask or a title first" : "Choose"}</option>
          {targets.map((t) => (
            <option key={t.value} value={t.value}>
              {t.label}
            </option>
          ))}
        </select>
      </label>
      <div className="buttons">
        <button
          disabled={busy || !pick?.region || !chosen}
          title="Follow the region from where it was drawn to the clip's end and key the follower to it"
          onClick={() => void editor.track(chosen)}
        >
          {running("track") ? "Tracking..." : "Track"}
        </button>
      </div>
      <Outcome editor={editor} measurement={measurement} kind="track" clip={clip.id} />
    </section>
  );
}
