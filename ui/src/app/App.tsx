import { useEffect, useState, useSyncExternalStore } from "react";

import { t, useLanguage } from "../i18n";
import { MAC, keystroke } from "../keyboard";
import { Activity } from "../panels/Activity";
import { Inspector } from "../panels/Inspector";
import { Library } from "../panels/Library";
import { Media } from "../panels/Media";
import { Record } from "../panels/Record";
import { Speech } from "../panels/Speech";
import { who } from "../panels/runs";
import { Guides } from "../preview/Guides";
import { MaskOverlay } from "../preview/MaskOverlay";
import { PreviewHole } from "../preview/PreviewHole";
import { RegionOverlay } from "../preview/RegionOverlay";
import { type ActionId, type ShortcutContext, actionFor, hint } from "../shortcuts";
import { findClip } from "../timeline/geometry";
import { Timeline } from "../timeline/Timeline";
import { TimelineTabs } from "../timeline/TimelineTabs";
import { Transport } from "../transport/Transport";
import { type Editor, mediaWorkLine } from "./editor";
import { Export } from "./Export";
import { ModelInstall } from "./ModelInstall";
import { isMulticam } from "./multicam";
import { recordedTime } from "./recorder";
import { Settings } from "./Settings";
import { Start } from "./Start";

/** How often the list of connected agents is read, in milliseconds. */
const CLIENTS_POLL = 2000;

/** The part of the window a keystroke happened in, for shortcuts that work only there. */
export function focusOf(target: EventTarget | null): ShortcutContext {
  const element = target as Element | null;
  if (!element || typeof element.closest !== "function") return "global";
  if (element.closest(".preview-hole")) return "canvas";
  if (element.closest(".timeline")) return "timeline";
  return "global";
}

/** What each shortcut does. */
export function run(editor: Editor, action: ActionId): void {
  const angle = /^multicam\.angle_(\d)$/.exec(action);
  if (angle) {
    const state = editor.getState();
    const selected = state.timeline && state.selection ? findClip(state.timeline, state.selection) : null;
    if (isMulticam(selected)) void editor.switchAngle(Number(angle[1]));
    return;
  }
  const actions: Partial<Record<ActionId, () => unknown>> = {
    "player.play": () => editor.togglePlay(),
    "player.frame_back": () => editor.step(-1),
    "player.frame_forward": () => editor.step(1),
    "player.step_back": () => editor.stepMany(-1),
    "player.step_forward": () => editor.stepMany(1),
    "player.start": () => editor.seek(0),
    "player.end": () => editor.seekEnd(),
    "player.previous_marker": () => editor.seekMarker(-1),
    "player.next_marker": () => editor.seekMarker(1),
    "player.shuttle_back": () => editor.shuttle(-1),
    "player.shuttle_stop": () => editor.shuttle(0),
    "player.shuttle_forward": () => editor.shuttle(1),
    "player.fullscreen": () => editor.toggleFullscreen(),
    "player.rulers": () => editor.toggleRulers(),
    "basic.undo": () => editor.undo(),
    "basic.redo": () => editor.redo(),
    "basic.copy": () => editor.copy(),
    "basic.cut": () => editor.cut(),
    "basic.paste": () => editor.paste(),
    "basic.copy_attributes": () => editor.copyAttributes(),
    "basic.paste_attributes": () => editor.pasteAttributes(),
    "basic.deselect": () => (editor.getState().fullscreen ? editor.toggleFullscreen() : editor.select(null)),
    "basic.settings": () => editor.openSettings(),
    "timeline.split": () => editor.splitAtPlayhead(),
    "timeline.split_all": () => editor.splitAll(),
    "timeline.delete": () => editor.deleteSelection(),
    "timeline.delete_left": () => editor.deleteSide("left"),
    "timeline.delete_right": () => editor.deleteSide("right"),
    "timeline.toggle_enabled": () => editor.toggleEnabled(),
    "timeline.group": () => editor.group(),
    "timeline.ungroup": () => editor.ungroup(),
    "timeline.compound": () => editor.compound(),
    "timeline.marker": () => editor.toggleMarker(),
    "timeline.magnet": () => editor.setMagnet(!editor.getState().magnet),
    "timeline.select_tool": () => editor.setTool("select"),
    "timeline.blade_tool": () => editor.setTool("blade"),
    "timeline.snapping": () => editor.setSnapping(!editor.getState().snapping),
    "timeline.skimming": () => editor.setSkimming(!editor.getState().skimming),
    "timeline.zoom_in": () => editor.zoom("in"),
    "timeline.zoom_out": () => editor.zoom("out"),
    "timeline.zoom_fit": () => editor.zoom("fit"),
    "canvas.nudge_left": () => editor.nudge(-1, 0),
    "canvas.nudge_right": () => editor.nudge(1, 0),
    "canvas.nudge_up": () => editor.nudge(0, -1),
    "canvas.nudge_down": () => editor.nudge(0, 1),
  };
  void actions[action]?.();
}

/** A gear: the Settings button's icon. */
function Gear() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <path
        fill="currentColor"
        d="M6.9 1h2.2l.4 1.9 1.2.5 1.6-1.1 1.6 1.6-1.1 1.6.5 1.2 1.9.4v2.2l-1.9.4-.5 1.2 1.1 1.6-1.6 1.6-1.6-1.1-1.2.5-.4 1.9H6.9l-.4-1.9-1.2-.5-1.6 1.1-1.6-1.6 1.1-1.6-.5-1.2L1 9.1V6.9l1.9-.4.5-1.2-1.1-1.6 1.6-1.6 1.6 1.1 1.2-.5zM8 5.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z"
      />
    </svg>
  );
}

/** What the sidebar's bin shows: the project's media, or FossCut's own library. */
type Bin = "media" | "library";

export function App({ editor }: { editor: Editor }) {
  const state = useSyncExternalStore(editor.subscribe, editor.getState);
  // Every text below follows the language the preferences choose.
  useLanguage();
  const open = state.opened !== null;
  const [bin, setBin] = useState<Bin>("media");

  useEffect(() => {
    void editor.start();
    return () => editor.stop();
  }, [editor]);

  useEffect(() => {
    if (!open) return;
    void editor.refreshClients();
    const timer = window.setInterval(() => {
      void editor.refreshClients();
      // The export dialog and the record panel read their jobs faster while they show.
      if (!editor.getState().exporting.open) void editor.pollExport();
      if (!editor.getState().recorder.open) void editor.pollRecording();
      // An analysis goes on, and adds its effect, while its clip is not selected.
      void editor.pollAnalysis();
    }, CLIENTS_POLL);
    return () => window.clearInterval(timer);
  }, [editor, open]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const now = editor.getState();
      // Settings and the export dialog are modal: Escape closes them, and editing keys wait.
      if (now.settings.open) {
        if (event.key === "Escape") editor.closeSettings();
        return;
      }
      if (now.exporting.open) {
        if (event.key === "Escape") editor.closeExport();
        return;
      }
      if (now.install) {
        if (event.key === "Escape") editor.cancelInstall();
        return;
      }
      const action = actionFor(keystroke(event), MAC, undefined, focusOf(event.target));
      if (!action || (!open && action !== "basic.settings")) return;
      event.preventDefault();
      run(editor, action);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editor, open]);

  const agents = state.clients.filter((c) => c.name.startsWith("agent:"));
  const selected = state.timeline && state.selection ? findClip(state.timeline, state.selection) : null;
  const working = mediaWorkLine(state.mediaWork);
  const settingsButton = (
    <button className="settings-button" onClick={() => editor.openSettings()} title={hint(t("app.settings"), "basic.settings")}>
      <Gear />
      <span>{t("app.settings")}</span>
    </button>
  );
  return (
    <div className={open ? (state.fullscreen ? "app fullscreen" : "app") : "app empty"}>
      {state.notice && (
        <div className={`notice ${state.notice.kind}`} role="status">
          <span>{state.notice.text}</span>
          <button className="link" onClick={() => editor.dismissNotice()}>
            {t("app.dismiss")}
          </button>
        </div>
      )}
      {state.update?.newer && (
        <div className="notice info update" role="status">
          <span>{t("app.update.available", { version: state.update.latest ?? "", current: state.update.current ?? "" })}</span>
          {state.update.url && (
            <button className="link" onClick={() => void editor.openRelease()}>
              {t("app.update.open")}
            </button>
          )}
          <button className="link" onClick={() => editor.dismissUpdate()}>
            {t("app.dismiss")}
          </button>
        </div>
      )}
      {!open && <Start editor={editor} state={state} settings={settingsButton} />}
      {open && state.opened && (
        <>
          <header>
            <strong>{state.opened.info.name}</strong>
            <span className="muted">{t("app.revision", { revision: state.opened.info.revision })}</span>
            {agents.length > 0 && (
              <span className="live" title={agents.map((a) => who(a.name)).join(", ")}>
                {agents.length === 1
                  ? t("app.agents.one", { name: who(agents[0]?.name ?? "") })
                  : t("app.agents.many", { count: agents.length })}
              </span>
            )}
            {working && (
              <span className="media-work muted" role="status" title={t("app.work.help")}>
                {working}
              </span>
            )}
            <span className="spacer" />
            <button onClick={() => void editor.undo()} disabled={!state.opened.info.can_undo} title={hint(t("app.undo"), "basic.undo")}>
              {t("app.undo")}
            </button>
            <button onClick={() => void editor.redo()} disabled={!state.opened.info.can_redo} title={hint(t("app.redo"), "basic.redo")}>
              {t("app.redo")}
            </button>
            <button onClick={() => editor.toggleRulers()} aria-pressed={state.rulers} title={hint(t("app.rulers.help"), "player.rulers")}>
              {t("app.rulers")}
            </button>
            <button onClick={() => void editor.toggleFullscreen()} title={hint(t("app.fullscreen.help"), "player.fullscreen")}>
              {t("app.fullscreen")}
            </button>
            <button
              className={state.recorder.job?.state === "running" ? "recording" : ""}
              aria-pressed={state.recorder.open}
              onClick={() => (state.recorder.open ? editor.closeRecorder() : void editor.openRecorder())}
              title={t("app.record.help")}
            >
              {state.recorder.job?.state === "running"
                ? t("app.recording", { time: recordedTime(state.recorder.job) })
                : t("app.record")}
            </button>
            <button
              aria-pressed={state.speech.open}
              onClick={() => (state.speech.open ? editor.closeSpeech() : void editor.openSpeech())}
              title={t("app.speech.help")}
            >
              {t("app.speech")}
            </button>
            {settingsButton}
            <button
              className="primary"
              onClick={() => void editor.openExport()}
              disabled={!state.timeline}
              title={state.exporting.job?.state === "running" ? t("app.export.running.help") : t("app.export.help")}
            >
              {state.exporting.job?.state === "running" ? t("app.exporting") : t("app.export")}
            </button>
            <button onClick={() => void editor.close()}>{t("app.close")}</button>
          </header>
          <main>
            {state.recorder.open && <Record editor={editor} state={state} />}
            {state.speech.open && <Speech editor={editor} state={state} />}
            <PreviewHole editor={editor}>
              <MaskOverlay editor={editor} state={state} />
              <Guides editor={editor} state={state} />
              <RegionOverlay editor={editor} state={state} />
            </PreviewHole>
            <aside className="activity">
              {selected && (
                <Inspector
                  editor={editor}
                  clip={selected}
                  playhead={state.playhead}
                  catalog={state.catalog}
                  luts={state.luts}
                  media={state.media}
                  others={(state.timeline?.tracks ?? [])
                    .flatMap((track) => track.clips)
                    .filter((c) => c.id !== selected.id && (c.kind === "video" || c.kind === "image"))}
                />
              )}
              <section className="bin">
                <div className="bin-tabs" role="tablist" aria-label={t("app.bin")}>
                  {(["media", "library"] as const).map((tab) => (
                    <button
                      key={tab}
                      role="tab"
                      aria-selected={bin === tab}
                      className={bin === tab ? "active" : ""}
                      onClick={() => setBin(tab)}
                      title={tab === "media" ? t("app.bin.media.help") : t("app.bin.library.help")}
                    >
                      {tab === "media" ? t("app.bin.media") : t("app.bin.library")}
                    </button>
                  ))}
                </div>
                {bin === "media" ? <Media editor={editor} state={state} /> : <Library editor={editor} state={state} />}
              </section>
              <Activity editor={editor} state={state} />
            </aside>
          </main>
          <Transport editor={editor} state={state} />
          <div className="timeline-area">
            <TimelineTabs editor={editor} state={state} />
            <Timeline editor={editor} state={state} />
          </div>
          {state.exporting.open && <Export editor={editor} state={state} />}
          {state.install && <ModelInstall editor={editor} prompt={state.install} />}
        </>
      )}
      {state.settings.open && <Settings editor={editor} state={state} />}
    </div>
  );
}
