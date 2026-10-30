import type { Editor, EditorState } from "../app/editor";
import { frameAt, parseRate, timecode } from "./time";

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

/** Play, pause, frame steps and the playhead's timecode. */
export function Transport({ editor, state }: { editor: Editor; state: EditorState }) {
  const timeline = state.timeline;
  const rate = parseRate(timeline?.format.rate ?? "30");
  const now = timecode(frameAt(state.playhead, rate), rate);
  return (
    <div className="transport">
      <button title="Start (Home)" onClick={() => void editor.seek(0)} disabled={!timeline}>
        <Icon path={START} />
      </button>
      <button title="Previous frame (Left arrow)" onClick={() => void editor.step(-1)} disabled={!timeline}>
        <Icon path={BACK} />
      </button>
      <button
        className="play"
        title={state.playing ? "Pause (Space)" : "Play (Space)"}
        onClick={() => void editor.togglePlay()}
        disabled={!timeline}
      >
        <Icon path={state.playing ? PAUSE : PLAY} />
      </button>
      <button title="Next frame (Right arrow)" onClick={() => void editor.step(1)} disabled={!timeline}>
        <Icon path={FORWARD} />
      </button>
      <button title="End (End)" onClick={() => void editor.seekEnd()} disabled={!timeline}>
        <Icon path={END} />
      </button>
      <span className="timecode">
        {now}
        <span className="muted"> / {timeline?.duration.timecode ?? "00:00:00:00"}</span>
      </span>
      {state.shuttle !== 0 && state.shuttle !== 1 && <span className="rate">{state.shuttle}x</span>}
      <span className="spacer" />
      <span className="tools" role="group" aria-label="Tool">
        <button
          className={state.tool === "select" ? "active" : ""}
          aria-pressed={state.tool === "select"}
          title="Select and drag clips (A)"
          onClick={() => editor.setTool("select")}
        >
          Select
        </button>
        <button
          className={state.tool === "blade" ? "active" : ""}
          aria-pressed={state.tool === "blade"}
          title="Click a clip to split it there (B)"
          onClick={() => editor.setTool("blade")}
        >
          Blade
        </button>
      </span>
      <label className="toggle" title="Dragged edges snap to clip edges, the playhead and markers (N)">
        <input type="checkbox" checked={state.snapping} onChange={(e) => editor.setSnapping(e.target.checked)} />
        <span>Snap</span>
      </label>
      <label className="toggle" title="Hovering over the timeline shows that frame (S)">
        <input type="checkbox" checked={state.skimming} onChange={(e) => editor.setSkimming(e.target.checked)} />
        <span>Skim</span>
      </label>
      <label className="toggle" title="Edits of a grouped clip edit its whole group">
        <input type="checkbox" checked={state.linkage} onChange={(e) => editor.setLinkage(e.target.checked)} />
        <span>Linkage</span>
      </label>
      <label className="toggle" title="Keep the main track gapless: deleting from it closes the gap (P)">
        <input type="checkbox" checked={state.magnet} onChange={(e) => editor.setMagnet(e.target.checked)} />
        <span>Magnet</span>
      </label>
      <select
        className="titles"
        value=""
        aria-label="Add a title at the playhead"
        title="Add a title at the playhead"
        disabled={!timeline}
        onChange={(event) => {
          const choice = event.target.value;
          if (choice) void editor.addTitle(choice === PLAIN ? "" : choice);
        }}
      >
        <option value="">Title</option>
        <option value={PLAIN} title="Plain white text, to style in the inspector">
          Plain text
        </option>
        {(state.catalog?.text.templates ?? []).length > 0 && (
          <optgroup label="Templates">
            {state.catalog?.text.templates.map((t) => (
              <option key={t.name} value={t.name} title={t.description}>
                {t.name.replaceAll("-", " ")}
              </option>
            ))}
          </optgroup>
        )}
      </select>
      <button
        title="Add an adjustment layer at the playhead: effects on it change every picture below"
        onClick={() => void editor.addAdjustment()}
        disabled={!timeline}
      >
        Adjustment
      </button>
      <button
        title="Split at the playhead (Mod+B)"
        onClick={() => void editor.splitAtPlayhead()}
        disabled={!timeline}
      >
        Split
      </button>
      <button
        title="Split every clip under the playhead (Mod+Shift+B)"
        onClick={() => void editor.splitAll()}
        disabled={!timeline}
      >
        Split all
      </button>
      <button
        title="Turn the selected clips into one compound clip (Mod+Alt+G)"
        onClick={() => void editor.compound()}
        disabled={!state.selection}
      >
        Compound
      </button>
      <button
        title="Add a marker at the playhead, or remove the one there (M)"
        onClick={() => void editor.toggleMarker()}
        disabled={!timeline}
      >
        Marker
      </button>
      <button
        title="Delete the selected clip (Delete)"
        onClick={() => void editor.deleteSelection()}
        disabled={!state.selection}
      >
        Delete
      </button>
    </div>
  );
}
