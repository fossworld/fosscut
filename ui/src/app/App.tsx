import { useEffect, useSyncExternalStore } from "react";

import { type Action, actionFor, isTyping } from "../keyboard";
import { Activity } from "../panels/Activity";
import { Inspector } from "../panels/Inspector";
import { who } from "../panels/runs";
import { MaskOverlay } from "../preview/MaskOverlay";
import { PreviewHole } from "../preview/PreviewHole";
import { findClip } from "../timeline/geometry";
import { Timeline } from "../timeline/Timeline";
import { Transport } from "../transport/Transport";
import type { Editor } from "./editor";
import { Start } from "./Start";

/** How often the list of connected agents is read, in milliseconds. */
const CLIENTS_POLL = 2000;

const MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

function run(editor: Editor, action: Action): void {
  const actions: Record<Action, () => unknown> = {
    "toggle-play": () => editor.togglePlay(),
    "step-back": () => editor.step(-1),
    "step-forward": () => editor.step(1),
    "second-back": () => editor.stepSeconds(-1),
    "second-forward": () => editor.stepSeconds(1),
    start: () => editor.seek(0),
    end: () => editor.seekEnd(),
    undo: () => editor.undo(),
    redo: () => editor.redo(),
    delete: () => editor.deleteSelection(),
    split: () => editor.splitAtPlayhead(),
    deselect: () => editor.select(null),
    "toggle-enabled": () => editor.toggleEnabled(),
  };
  void actions[action]();
}

export function App({ editor }: { editor: Editor }) {
  const state = useSyncExternalStore(editor.subscribe, editor.getState);
  const open = state.opened !== null;

  useEffect(() => {
    void editor.start();
    return () => editor.stop();
  }, [editor]);

  useEffect(() => {
    if (!open) return;
    void editor.refreshClients();
    const timer = window.setInterval(() => void editor.refreshClients(), CLIENTS_POLL);
    return () => window.clearInterval(timer);
  }, [editor, open]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const action = actionFor(
        {
          key: event.key,
          metaKey: event.metaKey,
          ctrlKey: event.ctrlKey,
          shiftKey: event.shiftKey,
          altKey: event.altKey,
          typing: isTyping(event.target),
        },
        MAC,
      );
      if (!action || !open) return;
      event.preventDefault();
      run(editor, action);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editor, open]);

  const agents = state.clients.filter((c) => c.name.startsWith("agent:"));
  const selected = state.timeline && state.selection ? findClip(state.timeline, state.selection) : null;
  return (
    <div className={open ? "app" : "app empty"}>
      {state.notice && (
        <div className={`notice ${state.notice.kind}`} role="status">
          <span>{state.notice.text}</span>
          <button className="link" onClick={() => editor.dismissNotice()}>
            Dismiss
          </button>
        </div>
      )}
      {!open && <Start editor={editor} state={state} />}
      {open && state.opened && (
        <>
          <header>
            <strong>{state.opened.info.name}</strong>
            <span className="muted">revision {state.opened.info.revision}</span>
            {agents.length > 0 && (
              <span className="live" title={agents.map((a) => who(a.name)).join(", ")}>
                {agents.length === 1 ? `${who(agents[0]?.name ?? "")} is editing` : `${agents.length} agents editing`}
              </span>
            )}
            <span className="spacer" />
            <button onClick={() => void editor.undo()} disabled={!state.opened.info.can_undo} title="Undo (Mod+Z)">
              Undo
            </button>
            <button onClick={() => void editor.redo()} disabled={!state.opened.info.can_redo} title="Redo (Mod+Shift+Z)">
              Redo
            </button>
            <button onClick={() => void editor.close()}>Close</button>
          </header>
          <main>
            <PreviewHole editor={editor}>
              <MaskOverlay editor={editor} state={state} />
            </PreviewHole>
            <aside className="activity">
              {selected && (
                <Inspector
                  editor={editor}
                  clip={selected}
                  playhead={state.playhead}
                  catalog={state.catalog}
                  luts={state.luts}
                  others={(state.timeline?.tracks ?? [])
                    .flatMap((t) => t.clips)
                    .filter((c) => c.id !== selected.id && (c.kind === "video" || c.kind === "image"))}
                />
              )}
              <Activity editor={editor} state={state} />
            </aside>
          </main>
          <Transport editor={editor} state={state} />
          <Timeline editor={editor} state={state} />
        </>
      )}
    </div>
  );
}
