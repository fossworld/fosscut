import { CircleAlert, Info, X } from "lucide-react";
import { Suspense, lazy, useEffect, useState, useSyncExternalStore } from "react";

import { t, useLanguage } from "../i18n";
import { MAC, keystroke } from "../keyboard";
import { Browser } from "../panels/Browser";
import { InspectorPanel } from "../panels/Inspector";
import { SmartOutcome } from "../panels/Smart";
import { Speech } from "../panels/Speech";
import { type ActionId, type ShortcutContext, actionFor } from "../shortcuts";
import { findClip } from "../timeline/geometry";
import { Timeline } from "../timeline/Timeline";
import { Glyph, IconButton, cx } from "../ui/controls";
import { Splitter } from "../ui/Splitter";
import type { Editor } from "./editor";
import { LIMITS, setLayout, useLayout } from "./layout";
import { isMulticam } from "./multicam";
import { Player } from "./Player";
import { Start } from "./Start";
import { TopBar } from "./TopBar";

// What is not on screen when the window opens loads when first wanted.
const Export = lazy(() => import("./Export").then((m) => ({ default: m.Export })));
const Settings = lazy(() => import("./Settings").then((m) => ({ default: m.Settings })));
const ModelInstall = lazy(() => import("./ModelInstall").then((m) => ({ default: m.ModelInstall })));
const Record = lazy(() => import("../panels/Record").then((m) => ({ default: m.Record })));
const WebImport = lazy(() => import("./Compositions").then((m) => ({ default: m.WebImport })));
const RemotionRender = lazy(() => import("./Compositions").then((m) => ({ default: m.RemotionRender })));

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

/** The window's size, followed as it changes, for panel limits. */
function useWindowSize(): { width: number; height: number } {
  const [size, setSize] = useState({ width: window.innerWidth, height: window.innerHeight });
  useEffect(() => {
    const onResize = () => setSize({ width: window.innerWidth, height: window.innerHeight });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return size;
}

export function App({ editor }: { editor: Editor }) {
  const state = useSyncExternalStore(editor.subscribe, editor.getState);
  // Every text below follows the language the preferences choose.
  useLanguage();
  const layout = useLayout();
  const windowSize = useWindowSize();
  const open = state.opened !== null;

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
      // A Remotion render goes on, and imports its file, while its dialog is hidden.
      if (!editor.getState().remotion.open) void editor.pollRemotion();
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
      if (now.webImport) {
        if (event.key === "Escape") editor.closeWebImport();
        return;
      }
      if (now.remotion.open) {
        if (event.key === "Escape") editor.closeRemotion();
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

  // The panels as dragged, kept inside the window so the preview always has room.
  const max = (limit: readonly [number, number], room: number) => Math.max(limit[0], Math.min(limit[1], room));
  const browserMax = max(LIMITS.browser, windowSize.width / 3);
  const inspectorMax = max(LIMITS.inspector, windowSize.width / 3);
  const timelineMax = max(LIMITS.timeline, windowSize.height - 320);
  const browser = Math.min(layout.browser, browserMax);
  const inspector = Math.min(layout.inspector, inspectorMax);
  const timeline = Math.min(layout.timeline, timelineMax);
  const columns = [
    layout.showBrowser ? `${browser}px var(--gap)` : "",
    "minmax(0, 1fr)",
    layout.showInspector ? `var(--gap) ${inspector}px` : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={cx("app", !open && "empty", state.fullscreen && "fullscreen")}>
      {(state.notice || state.update?.newer) && (
        <div className="notices">
          {state.notice && (
            <div className={`notice ${state.notice.kind}`} role="status">
              <Glyph icon={state.notice.kind === "error" ? CircleAlert : Info} size="sm" />
              <span>{state.notice.text}</span>
              <IconButton icon={X} size="sm" label={t("app.dismiss")} onClick={() => editor.dismissNotice()} />
            </div>
          )}
          {state.update?.newer && (
            <div className="notice info update" role="status">
              <Glyph icon={Info} size="sm" />
              <span>{t("app.update.available", { version: state.update.latest ?? "", current: state.update.current ?? "" })}</span>
              {state.update.url && (
                <button className="link" onClick={() => void editor.openRelease()}>
                  {t("app.update.open")}
                </button>
              )}
              <IconButton icon={X} size="sm" label={t("app.dismiss")} onClick={() => editor.dismissUpdate()} />
            </div>
          )}
        </div>
      )}
      {!open && <Start editor={editor} state={state} />}
      {open && state.opened && (
        <>
          <TopBar editor={editor} state={state} />
          <div className="workspace" style={{ gridTemplateColumns: columns, gridTemplateRows: `minmax(0, 1fr) var(--gap) ${timeline}px` }}>
            {layout.showBrowser && (
              <>
                <Browser editor={editor} state={state} />
                <Splitter
                  orientation="vertical"
                  label={t("app.layout.resize.browser")}
                  value={browser}
                  min={LIMITS.browser[0]}
                  max={browserMax}
                  grows="right"
                  onChange={(value) => setLayout({ browser: value })}
                />
              </>
            )}
            <Player editor={editor} state={state}>
              {(state.measurement?.kind === "vertical" || (state.analysisFor === "vertical" && state.assist?.kind === "analysis")) && (
                <section className="record-panel smart-card" aria-live="polite">
                  <div className="record-head">
                    <h2>{t("timeline.vertical.title")}</h2>
                  </div>
                  <SmartOutcome editor={editor} state={state} kind="vertical" clip={null} />
                </section>
              )}
              <Suspense fallback={null}>
                {state.recorder.open && <Record editor={editor} state={state} />}
                {state.speech.open && <Speech editor={editor} state={state} />}
              </Suspense>
            </Player>
            {layout.showInspector && (
              <>
                <Splitter
                  orientation="vertical"
                  label={t("app.layout.resize.inspector")}
                  value={inspector}
                  min={LIMITS.inspector[0]}
                  max={inspectorMax}
                  grows="left"
                  onChange={(value) => setLayout({ inspector: value })}
                />
                <InspectorPanel editor={editor} state={state} />
              </>
            )}
            <Splitter
              orientation="horizontal"
              label={t("app.layout.resize.timeline")}
              value={timeline}
              min={LIMITS.timeline[0]}
              max={timelineMax}
              grows="up"
              onChange={(value) => setLayout({ timeline: value })}
            />
            <Timeline editor={editor} state={state} />
          </div>
          <Suspense fallback={null}>
            {state.exporting.open && <Export editor={editor} state={state} />}
            {state.install && <ModelInstall editor={editor} prompt={state.install} />}
            {state.webImport && <WebImport key={state.webImport.page.path} editor={editor} state={state} dialog={state.webImport} />}
            {state.remotion.open && <RemotionRender editor={editor} dialog={state.remotion} />}
          </Suspense>
        </>
      )}
      <Suspense fallback={null}>{state.settings.open && <Settings editor={editor} state={state} />}</Suspense>
    </div>
  );
}
