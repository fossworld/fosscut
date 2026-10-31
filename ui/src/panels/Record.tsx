import { useEffect, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import { NO_MICROPHONE, type RecordForm, defaultForm, devicesOf, recordChoice, recordProblem, recordedTime } from "../app/recorder";
import type { DeviceKind } from "../engine/types";
import { t } from "../i18n";

/** How often a running recording's time is read while the panel shows, in milliseconds. */
const POLL = 250;

const KINDS: DeviceKind[] = ["microphone", "camera", "screen"];

/**
 * Recording a voiceover, a camera or the screen into the project: choose what and from
 * which device (and the microphone for a camera or screen), then Record and Stop. The
 * recording lands at the playhead where it started.
 */
export function Record({ editor, state }: { editor: Editor; state: EditorState }) {
  const { devices, job, error } = state.recorder;
  const [chosen, setChosen] = useState<RecordForm | null>(null);
  const form = chosen ?? defaultForm("microphone", devices ?? []);
  const running = job?.state === "running";
  const finished = job && !running ? job : null;

  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => void editor.pollRecording(), POLL);
    return () => window.clearInterval(timer);
  }, [editor, running]);

  const listed = devicesOf(devices ?? [], form.what);
  const microphones = devicesOf(devices ?? [], "microphone");
  return (
    <section className="record-panel" role="dialog" aria-label={t("record.title")}>
      <div className="record-head">
        <h2>{t("record.title")}</h2>
        <button className="link" onClick={() => editor.closeRecorder()} title={running ? t("record.close.help") : undefined}>
          {t("record.close")}
        </button>
      </div>
      <div className="buttons kinds" role="radiogroup" aria-label={t("record.what")}>
        {KINDS.map((kind) => (
          <button
            key={kind}
            role="radio"
            aria-checked={form.what === kind}
            className={form.what === kind ? "active" : ""}
            disabled={running}
            onClick={() => setChosen(defaultForm(kind, devices ?? []))}
          >
            {t(`record.kind.${kind}`)}
          </button>
        ))}
      </div>
      <label className="choice">
        <span>{t(`record.kind.${form.what}`)}</span>
        <select
          value={form.device}
          disabled={running || listed.length === 0}
          onChange={(event) => setChosen({ ...form, device: event.target.value })}
        >
          {listed.length === 0 && (
            <option value="">{devices === null ? t("record.devices.looking") : t("record.devices.none")}</option>
          )}
          {listed.map((d) => (
            <option key={`${d.format}:${d.input}`} value={d.name}>
              {d.name}
            </option>
          ))}
        </select>
      </label>
      {form.what !== "microphone" && (
        <label className="choice">
          <span>{t("record.sound")}</span>
          <select value={form.microphone} disabled={running} onChange={(event) => setChosen({ ...form, microphone: event.target.value })}>
            <option value={NO_MICROPHONE}>{t("record.sound.none")}</option>
            {microphones.map((d) => (
              <option key={`${d.format}:${d.input}`} value={d.name}>
                {d.name}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="record-go">
        <button
          className={running ? "record-toggle on" : "record-toggle"}
          aria-pressed={running}
          disabled={!running && listed.length === 0}
          onClick={() => void (running ? editor.stopRecording() : editor.startRecording(recordChoice(form)))}
        >
          {running ? t("record.stop") : t("record.start")}
        </button>
        <span className="record-time" aria-live="polite">
          {recordedTime(job)}
        </span>
      </div>
      {finished?.state === "succeeded" && finished.clip && (
        <p className="hint">{t("record.placed", { time: recordedTime(finished) })}</p>
      )}
      {finished?.state === "succeeded" && !finished.clip && finished.asset && (
        <p className="hint">{t("record.binned", { time: recordedTime(finished) })}</p>
      )}
      {error && (
        <p className="problem" role="alert">
          {recordProblem(error)}
        </p>
      )}
    </section>
  );
}
