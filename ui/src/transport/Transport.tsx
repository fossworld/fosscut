import { type Editor, type EditorState, preferencesOf } from "../app/editor";
import { t } from "../i18n";
import { hint } from "../shortcuts";
import { formatTime, parseRate } from "./time";

function Icon({ path }: { path: string }) {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <path d={path} fill="currentColor" />
    </svg>
  );
}

const PLAY = "M4 2.5v11l9-5.5z";
const PAUSE = "M4 2.5h3v11H4zM9 2.5h3v11H9z";
const START = "M3 2.5h2v11H3zM13 2.5v11L6 8z";
const BACK = "M11 2.5v11L4 8z";
const FORWARD = "M5 2.5v11l7-5.5z";
const END = "M11 2.5h2v11h-2zM3 2.5v11L10 8z";

/** The title picker's choice for plain text rather than a template. */
const PLAIN = "*plain";

/** Play, pause, frame steps and the playhead's time, shown as the preferences say. */
export function Transport({ editor, state }: { editor: Editor; state: EditorState }) {
  const timeline = state.timeline;
  const rate = parseRate(timeline?.format.rate ?? "30");
  const display = preferencesOf(state).timeline.time_display;
  const now = formatTime(state.playhead, rate, display);
  const length = formatTime(timeline?.duration.flicks ?? 0, rate, display);
  return (
    <div className="transport">
      <button title={hint(t("transport.start"), "player.start")} onClick={() => void editor.seek(0)} disabled={!timeline}>
        <Icon path={START} />
      </button>
      <button title={hint(t("transport.previous"), "player.frame_back")} onClick={() => void editor.step(-1)} disabled={!timeline}>
        <Icon path={BACK} />
      </button>
      <button
        className="play"
        title={hint(state.playing ? t("transport.pause") : t("transport.play"), "player.play")}
        onClick={() => void editor.togglePlay()}
        disabled={!timeline}
      >
        <Icon path={state.playing ? PAUSE : PLAY} />
      </button>
      <button title={hint(t("transport.next"), "player.frame_forward")} onClick={() => void editor.step(1)} disabled={!timeline}>
        <Icon path={FORWARD} />
      </button>
      <button title={hint(t("transport.end"), "player.end")} onClick={() => void editor.seekEnd()} disabled={!timeline}>
        <Icon path={END} />
      </button>
      <span className="timecode">
        {now}
        <span className="muted"> / {length}</span>
      </span>
      {state.shuttle !== 0 && state.shuttle !== 1 && <span className="rate">{t("transport.rate", { rate: state.shuttle })}</span>}
      <span className="spacer" />
      <span className="tools" role="group" aria-label={t("transport.tool")}>
        <button
          className={state.tool === "select" ? "active" : ""}
          aria-pressed={state.tool === "select"}
          title={hint(t("transport.select.help"), "timeline.select_tool")}
          onClick={() => editor.setTool("select")}
        >
          {t("transport.select")}
        </button>
        <button
          className={state.tool === "blade" ? "active" : ""}
          aria-pressed={state.tool === "blade"}
          title={hint(t("transport.blade.help"), "timeline.blade_tool")}
          onClick={() => editor.setTool("blade")}
        >
          {t("transport.blade")}
        </button>
      </span>
      <label className="toggle" title={hint(t("transport.snap.help"), "timeline.snapping")}>
        <input type="checkbox" checked={state.snapping} onChange={(e) => editor.setSnapping(e.target.checked)} />
        <span>{t("transport.snap")}</span>
      </label>
      <label className="toggle" title={hint(t("transport.skim.help"), "timeline.skimming")}>
        <input type="checkbox" checked={state.skimming} onChange={(e) => editor.setSkimming(e.target.checked)} />
        <span>{t("transport.skim")}</span>
      </label>
      <label className="toggle" title={t("transport.linked.help")}>
        <input type="checkbox" checked={state.linkage} onChange={(e) => editor.setLinkage(e.target.checked)} />
        <span>{t("transport.linked")}</span>
      </label>
      <label className="toggle" title={hint(t("transport.magnet.help"), "timeline.magnet")}>
        <input type="checkbox" checked={state.magnet} onChange={(e) => editor.setMagnet(e.target.checked)} />
        <span>{t("transport.magnet")}</span>
      </label>
      <select
        className="titles"
        value=""
        aria-label={t("transport.title.add")}
        title={t("transport.title.add")}
        disabled={!timeline}
        onChange={(event) => {
          const choice = event.target.value;
          if (choice) void editor.addTitle(choice === PLAIN ? "" : choice);
        }}
      >
        <option value="">{t("transport.title")}</option>
        <option value={PLAIN} title={t("transport.title.plain.help")}>
          {t("transport.title.plain")}
        </option>
        {(state.catalog?.text.templates ?? []).length > 0 && (
          <optgroup label={t("transport.title.templates")}>
            {state.catalog?.text.templates.map((template) => (
              <option key={template.name} value={template.name} title={template.description}>
                {template.name.replaceAll("-", " ")}
              </option>
            ))}
          </optgroup>
        )}
      </select>
      <button title={t("transport.adjustment.help")} onClick={() => void editor.addAdjustment()} disabled={!timeline}>
        {t("transport.adjustment")}
      </button>
      <button
        title={hint(t("transport.split.help"), "timeline.split")}
        onClick={() => void editor.splitAtPlayhead()}
        disabled={!timeline}
      >
        {t("transport.split")}
      </button>
      <button
        title={hint(t("transport.splitAll.help"), "timeline.split_all")}
        onClick={() => void editor.splitAll()}
        disabled={!timeline}
      >
        {t("transport.splitAll")}
      </button>
      <button
        title={hint(t("transport.compound.help"), "timeline.compound")}
        onClick={() => void editor.compound()}
        disabled={!state.selection}
      >
        {t("transport.compound")}
      </button>
      <button
        title={hint(t("transport.marker.help"), "timeline.marker")}
        onClick={() => void editor.toggleMarker()}
        disabled={!timeline}
      >
        {t("transport.marker")}
      </button>
      <button
        title={hint(t("transport.delete.help"), "timeline.delete")}
        onClick={() => void editor.deleteSelection()}
        disabled={!state.selection}
      >
        {t("transport.delete")}
      </button>
    </div>
  );
}
