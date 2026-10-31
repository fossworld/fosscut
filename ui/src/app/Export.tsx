import { useEffect, useState } from "react";

import type { ApiError, ExportPresets, JobStatus, Opened, TimelineView } from "../engine/types";
import { t } from "../i18n";
import { parseRate } from "../transport/time";
import type { Editor, EditorState } from "./editor";
import {
  type ExportForm,
  FRAME_RATES,
  audioOnly,
  baseName,
  choiceOf,
  coverStart,
  defaultPreset,
  fileName,
  folderOf,
  fpsNumber,
  markerRanges,
  progressOf,
  reportRows,
  showTime,
  sizeFor,
  withExtension,
} from "./exportForm";

/** How often a running export's progress is read while the dialog shows, in milliseconds. */
const POLL = 500;

/** A FossCut error with its hint, as panels show one. */
export function Problem({ error }: { error: ApiError }) {
  return (
    <p className="problem" role="alert">
      {error.message}
      {error.hint ? `. ${error.hint}` : ""}
    </p>
  );
}

/** The form a new export starts from: the project's name beside it, the whole timeline. */
function initial(presets: ExportPresets, opened: Opened, timeline: TimelineView, playhead: number): ExportForm {
  const preset = defaultPreset(presets.presets);
  const extension = preset?.extension ?? "mp4";
  const folder = folderOf(opened.bundle);
  const name = fileName(opened.info.name, extension);
  const rate = parseRate(timeline.format.rate);
  return {
    preset: preset?.name ?? "",
    resolution: "",
    fps: "",
    bitrate: "",
    part: false,
    from: showTime(0, rate),
    to: showTime(timeline.duration.flicks, rate),
    cover: false,
    coverAt: showTime(playhead, rate),
    subtitles: "",
    output: folder ? `${folder}/${name}` : name,
    overwrite: false,
  };
}

/** The choices of a new export, and starting it. */
function Choices({
  editor,
  state,
  opened,
  presets,
  timeline,
}: {
  editor: Editor;
  state: EditorState;
  opened: Opened;
  presets: ExportPresets;
  timeline: TimelineView;
}) {
  const [form, setForm] = useState<ExportForm>(() => initial(presets, opened, timeline, state.playhead));
  const [starting, setStarting] = useState(false);
  const update = (change: Partial<ExportForm>) => setForm({ ...form, ...change });
  const rate = parseRate(timeline.format.rate);
  const preset = presets.presets.find((p) => p.name === form.preset);
  const sound = audioOnly(preset);
  const made = choiceOf(form, { format: timeline.format, duration: timeline.duration.flicks, presets: presets.presets });
  const marked = markerRanges(timeline.markers ?? [], timeline.duration.flicks);
  const captions = timeline.tracks.some((track) => track.kind === "caption");
  const [width, height] = [timeline.format.width, timeline.format.height];
  const here = showTime(state.playhead, rate);

  const choosePreset = (name: string) => {
    const next = presets.presets.find((p) => p.name === name);
    update({ preset: name, output: next ? withExtension(form.output, next.extension) : form.output });
  };
  const pick = async () => {
    const path = await editor.pickExportPath(baseName(form.output), preset?.extension);
    // The system's save dialog has asked before choosing a file that exists.
    if (path) update({ output: path, overwrite: true });
  };
  const start = async () => {
    if (!("choice" in made)) return;
    setStarting(true);
    await editor.startExport(made.choice);
    setStarting(false);
  };

  return (
    <form
      className="export-form"
      onSubmit={(event) => {
        event.preventDefault();
        void start();
      }}
    >
      <label className="choice">
        <span>{t("export.preset")}</span>
        <select value={form.preset} onChange={(e) => choosePreset(e.target.value)}>
          {presets.presets.map((p) => (
            <option key={p.name} value={p.name} disabled={!p.available} title={p.description}>
              {p.available ? p.name : t("export.preset.unavailable", { name: p.name })}
            </option>
          ))}
        </select>
      </label>
      {preset && <p className="muted hint">{preset.description}</p>}
      <label className="choice">
        <span>{t("export.resolution")}</span>
        <select value={form.resolution} disabled={sound} onChange={(e) => update({ resolution: e.target.value })}>
          <option value="">{t("export.resolution.timeline", { width, height })}</option>
          {presets.resolutions.map((r) => {
            const [w, h] = sizeFor(timeline.format, r.pixels);
            return (
              <option key={r.name} value={r.name}>
                {r.name} ({w} x {h})
              </option>
            );
          })}
        </select>
      </label>
      <label className="choice">
        <span>{t("export.frameRate")}</span>
        <select value={form.fps} disabled={sound} onChange={(e) => update({ fps: e.target.value })}>
          <option value="">{t("export.frameRate.timeline", { fps: fpsNumber(timeline.format.rate) })}</option>
          {FRAME_RATES.map((fps) => (
            <option key={fps} value={fps}>
              {t("export.fps", { fps: Number(fps) })}
            </option>
          ))}
        </select>
      </label>
      <label className="choice">
        <span>{t("export.bitrate")}</span>
        <input
          className="short"
          placeholder={t("export.bitrate.automatic")}
          title={t("export.bitrate.help")}
          disabled={sound}
          value={form.bitrate}
          onChange={(e) => update({ bitrate: e.target.value })}
        />
      </label>
      <div className="part" role="radiogroup" aria-label={t("export.what")}>
        <label className="toggle">
          <input type="radio" checked={!form.part} onChange={() => update({ part: false })} />
          <span>{t("export.whole", { length: timeline.duration.timecode })}</span>
        </label>
        <label className="toggle">
          <input type="radio" checked={form.part} onChange={() => update({ part: true })} />
          <span>{t("export.part")}</span>
        </label>
        {form.part && (
          <div className="times">
            <label className="choice">
              <span>{t("export.from")}</span>
              <input className="time" value={form.from} onChange={(e) => update({ from: e.target.value })} />
              <button type="button" title={t("export.playhead.help")} onClick={() => update({ from: here })}>
                {t("export.playhead")}
              </button>
            </label>
            <label className="choice">
              <span>{t("export.to")}</span>
              <input className="time" value={form.to} onChange={(e) => update({ to: e.target.value })} />
              <button type="button" title={t("export.playhead.help")} onClick={() => update({ to: here })}>
                {t("export.playhead")}
              </button>
            </label>
            {marked.length > 0 && (
              <label className="choice">
                <span>{t("export.markers")}</span>
                <select
                  value=""
                  onChange={(e) => {
                    const range = marked[Number(e.target.value)];
                    if (range) update({ from: showTime(range.from, rate), to: showTime(range.to, rate) });
                  }}
                >
                  <option value="">{t("export.markers.choose")}</option>
                  {marked.map((range, index) => (
                    <option key={index} value={index}>
                      {range.label}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>
        )}
      </div>
      <label className="toggle" title={t("export.cover.help")}>
        <input
          type="checkbox"
          checked={form.cover}
          disabled={sound}
          onChange={(e) => update({ cover: e.target.checked, coverAt: coverStart(form, rate) })}
        />
        <span>{t("export.cover")}</span>
      </label>
      {form.cover && !sound && (
        <div className="times">
          <label className="choice">
            <span>{t("export.cover.at")}</span>
            <input className="time" value={form.coverAt} onChange={(e) => update({ coverAt: e.target.value })} />
            <button type="button" title={t("export.cover.playhead.help")} onClick={() => update({ coverAt: here })}>
              {t("export.playhead")}
            </button>
          </label>
        </div>
      )}
      <label className="choice">
        <span>{t("export.subtitles")}</span>
        <select value={form.subtitles} onChange={(e) => update({ subtitles: e.target.value as ExportForm["subtitles"] })}>
          <option value="">{t("export.subtitles.none")}</option>
          <option value="srt">{t("export.subtitles.srt")}</option>
          <option value="vtt">{t("export.subtitles.vtt")}</option>
        </select>
      </label>
      {form.subtitles && (
        <p className="muted hint">
          {captions ? t("export.subtitles.from") : t("export.subtitles.noCaptions")}
        </p>
      )}
      <label className="choice output">
        <span>{t("export.saveTo")}</span>
        <input value={form.output} title={form.output} onChange={(e) => update({ output: e.target.value, overwrite: false })} />
        <button type="button" onClick={() => void pick()}>
          {t("export.saveTo.choose")}
        </button>
      </label>
      <label className="toggle">
        <input type="checkbox" checked={form.overwrite} onChange={(e) => update({ overwrite: e.target.checked })} />
        <span>{t("export.overwrite")}</span>
      </label>
      {"problem" in made && <p className="muted hint">{made.problem}</p>}
      {state.exporting.error && <Problem error={state.exporting.error} />}
      <div className="buttons">
        <span className="spacer" />
        <button type="button" onClick={() => editor.closeExport()}>
          {t("export.cancel")}
        </button>
        <button type="submit" className="primary" disabled={!("choice" in made) || starting}>
          {t("export.start")}
        </button>
      </div>
    </form>
  );
}

/** A running export's progress, and a finished one's report or error. */
function Progress({ editor, job }: { editor: Editor; job: JobStatus }) {
  const progress = progressOf(job);
  const report = job.report;
  return (
    <div className="export-job">
      <p className="name" title={job.output}>
        {job.output ? baseName(job.output) : job.job}
      </p>
      <div className="bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round((progress.fraction ?? 0) * 100)}>
        <span
          className={progress.fraction === null && job.state === "running" ? "busy" : ""}
          style={{ width: `${(progress.fraction ?? (job.state === "running" ? 1 : 0)) * 100}%` }}
        />
      </div>
      <p className="muted">{progress.text}</p>
      {job.state === "failed" && job.error && <Problem error={job.error} />}
      {report && (
        <>
          <dl className="report">
            {reportRows(report).map(([label, value]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd title={value}>{value}</dd>
              </div>
            ))}
          </dl>
          {(report.rights ?? []).length > 0 && (
            <>
              <h3>{t("export.rights")}</h3>
              <ul className="rights">
                {report.rights?.map((note, index) => (
                  <li key={index} className={note.origin}>
                    <strong>{note.name}</strong> <span className="muted">{note.note}</span>
                  </li>
                ))}
              </ul>
            </>
          )}
        </>
      )}
      <div className="buttons">
        <span className="spacer" />
        {job.state === "running" ? (
          <>
            <button title={t("export.hide.help")} onClick={() => editor.closeExport()}>
              {t("export.hide")}
            </button>
            <button onClick={() => void editor.cancelExport()}>{t("export.cancelExport")}</button>
          </>
        ) : (
          <>
            <button onClick={() => editor.clearExport()}>
              {job.state === "succeeded" ? t("export.another") : t("export.back")}
            </button>
            <button className="primary" onClick={() => editor.closeExport()}>
              {t("export.done")}
            </button>
          </>
        )}
      </div>
    </div>
  );
}

/**
 * The export dialog: preset, size, frame rate and bit rate, the whole timeline or a part,
 * a cover image, subtitles and where to save. The export runs in the background; its
 * progress is read twice a second while the dialog shows, then its verified report.
 */
export function Export({ editor, state }: { editor: Editor; state: EditorState }) {
  const { presets, job, error } = state.exporting;
  const running = job?.state === "running";
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => void editor.pollExport(), POLL);
    return () => window.clearInterval(timer);
  }, [editor, running]);
  const timeline = state.timeline;
  return (
    <div className="dialog-backdrop">
      <div
        className="dialog export"
        role="dialog"
        aria-modal="true"
        aria-labelledby="export-title"
      >
        <h2 id="export-title">
          {t("export.title")} {timeline ? <span className="muted">{timeline.name}</span> : null}
        </h2>
        {job ? (
          <Progress editor={editor} job={job} />
        ) : presets && timeline && state.opened ? (
          <Choices editor={editor} state={state} opened={state.opened} presets={presets} timeline={timeline} />
        ) : error ? (
          <>
            <Problem error={error} />
            <div className="buttons">
              <span className="spacer" />
              <button onClick={() => editor.closeExport()}>{t("export.close")}</button>
            </div>
          </>
        ) : (
          <p className="muted">{t("export.reading")}</p>
        )}
      </div>
    </div>
  );
}
