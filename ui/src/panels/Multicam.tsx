import { useState } from "react";

import type { Editor } from "../app/editor";
import { type MulticamForm as Form, NEW_MULTICAM, angleNumber, angleTitle, canSync, multicamRequest } from "../app/multicam";
import type { AssetView, ClipView } from "../engine/types";
import { Outcome } from "./Measured";

/**
 * A multicam clip's angles as numbered buttons: pressing one, or its number key while
 * the timeline has focus, cuts to that angle at the playhead.
 */
export function Multicam({ editor, clip }: { editor: Editor; clip: ClipView }) {
  const shown = angleNumber(clip);
  const angles = clip.angles ?? [];
  return (
    <section className="effects multicam">
      <h2>Multicam</h2>
      <div className="buttons angles" role="group" aria-label="Cut to an angle at the playhead">
        {angles.map((angle, i) => (
          <button
            key={angle.id}
            className={shown === i + 1 ? "active" : ""}
            aria-pressed={shown === i + 1}
            title={`${angleTitle(clip, i + 1)}: cut to it at the playhead${i < 9 ? ` (${i + 1})` : ""}`}
            onClick={() => void editor.switchAngle(i + 1)}
          >
            {i + 1}
          </button>
        ))}
      </div>
      <p className="hint">
        {shown ? `This part shows ${angleTitle(clip, shown)}.` : "This part shows every angle."} Press 1 to 9 on the
        timeline to cut, even while it plays.
      </p>
    </section>
  );
}

/**
 * Making a multicam clip of the video assets chosen in the bin: synced by their sound
 * unless told not to, with one angle's sound and a name, placed at the playhead.
 */
export function MulticamForm({ editor, angles, playhead }: { editor: Editor; angles: AssetView[]; playhead: number }) {
  const [form, setForm] = useState<Form>(NEW_MULTICAM);
  const { measurement } = editor.getState();
  const busy = measurement?.busy === true;
  const syncable = canSync(angles);
  const sound = Math.min(form.sound, angles.length);
  return (
    <div className="multicam-form">
      <h3>Multicam</h3>
      <label className="toggle" title={syncable ? "Line the angles up by comparing their sound" : "Every angle needs sound to sync by it"}>
        <input
          type="checkbox"
          checked={form.sync && syncable}
          disabled={!syncable || busy}
          onChange={(event) => setForm({ ...form, sync: event.target.checked })}
        />
        <span>Sync by sound</span>
      </label>
      <label className="choice">
        <span>Sound from</span>
        <select value={sound} disabled={busy} onChange={(event) => setForm({ ...form, sound: Number(event.target.value) })}>
          {angles.map((angle, i) => (
            <option key={angle.id} value={i + 1}>
              {`Angle ${i + 1}: ${angle.name}`}
            </option>
          ))}
        </select>
      </label>
      <div className="buttons">
        <input
          type="text"
          placeholder="Name, such as Interview"
          value={form.name}
          disabled={busy}
          aria-label="Multicam name"
          onChange={(event) => setForm({ ...form, name: event.target.value })}
        />
        <button
          className="primary"
          disabled={busy}
          title="Make a multicam clip of the chosen video at the playhead, one angle per track"
          onClick={() => {
            void editor.createMulticam(multicamRequest(form, angles, playhead)).then((made) => made && setForm(NEW_MULTICAM));
          }}
        >
          {busy && measurement?.kind === "multicam" ? (form.sync && syncable ? "Syncing..." : "Making...") : "Create multicam"}
        </button>
      </div>
      {!syncable && <p className="hint">Not every angle has sound, so they start together.</p>}
      <Outcome editor={editor} measurement={measurement} kind="multicam" clip={null} />
    </div>
  );
}
