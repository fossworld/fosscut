import { useState } from "react";

import type { Editor } from "../app/editor";
import { type MulticamForm as Form, NEW_MULTICAM, angleNumber, angleTitle, canSync, multicamRequest } from "../app/multicam";
import type { AssetView, ClipView } from "../engine/types";
import { t } from "../i18n";
import { type ActionId, hint, keysOf } from "../shortcuts";
import { Outcome } from "./Measured";

/** The shortcut that cuts to angle `number`, for the first nine. */
function angleAction(number: number): ActionId | null {
  return number >= 1 && number <= 9 ? (`multicam.angle_${number}` as ActionId) : null;
}

/**
 * A multicam clip's angles as numbered buttons: pressing one, or its shortcut (1 to 9
 * unless changed) while the timeline has focus, cuts to that angle at the playhead.
 */
export function Multicam({ editor, clip }: { editor: Editor; clip: ClipView }) {
  const shown = angleNumber(clip);
  const angles = clip.angles ?? [];
  const [first, last] = [keysOf("multicam.angle_1"), keysOf("multicam.angle_9")];
  const cutTo = (number: number) => {
    const label = t("motion.multicam.cutTo", { angle: angleTitle(clip, number) });
    const action = angleAction(number);
    return action ? hint(label, action) : label;
  };
  return (
    <section className="effects multicam">
      <h2>{t("motion.multicam.title")}</h2>
      <div className="buttons angles" role="group" aria-label={t("motion.multicam.cut")}>
        {angles.map((angle, i) => (
          <button
            key={angle.id}
            className={shown === i + 1 ? "active" : ""}
            aria-pressed={shown === i + 1}
            title={cutTo(i + 1)}
            onClick={() => void editor.switchAngle(i + 1)}
          >
            {i + 1}
          </button>
        ))}
      </div>
      <p className="hint">
        {shown ? t("motion.multicam.shows", { angle: angleTitle(clip, shown) }) : t("motion.multicam.showsAll")}
        {first && last ? ` ${t("motion.multicam.keys", { first, last })}` : ""}
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
      <h3>{t("motion.multicam.title")}</h3>
      <label className="toggle" title={syncable ? t("motion.multicam.sync.help") : t("motion.multicam.sync.unable")}>
        <input
          type="checkbox"
          checked={form.sync && syncable}
          disabled={!syncable || busy}
          onChange={(event) => setForm({ ...form, sync: event.target.checked })}
        />
        <span>{t("motion.multicam.sync")}</span>
      </label>
      <label className="choice">
        <span>{t("motion.multicam.sound")}</span>
        <select value={sound} disabled={busy} onChange={(event) => setForm({ ...form, sound: Number(event.target.value) })}>
          {angles.map((angle, i) => (
            <option key={angle.id} value={i + 1}>
              {t("motion.multicam.named", { number: i + 1, name: angle.name })}
            </option>
          ))}
        </select>
      </label>
      <div className="buttons">
        <input
          type="text"
          placeholder={t("motion.multicam.name")}
          value={form.name}
          disabled={busy}
          aria-label={t("motion.multicam.name.label")}
          onChange={(event) => setForm({ ...form, name: event.target.value })}
        />
        <button
          className="primary"
          disabled={busy}
          title={t("motion.multicam.create.help")}
          onClick={() => {
            void editor.createMulticam(multicamRequest(form, angles, playhead)).then((made) => made && setForm(NEW_MULTICAM));
          }}
        >
          {busy && measurement?.kind === "multicam"
            ? form.sync && syncable
              ? t("motion.multicam.syncing")
              : t("motion.multicam.making")
            : t("motion.multicam.create")}
        </button>
      </div>
      {!syncable && <p className="hint">{t("motion.multicam.silent")}</p>}
      <Outcome editor={editor} measurement={measurement} kind="multicam" clip={null} />
    </div>
  );
}
