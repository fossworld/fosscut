import { MonitorPlay } from "lucide-react";
import { useRef, useState, useSyncExternalStore } from "react";

import type { ClipView } from "../engine/types";
import { t } from "../i18n";
import { Guides } from "../preview/Guides";
import { MaskOverlay } from "../preview/MaskOverlay";
import { PreviewHole, ZOOMS, clampPan } from "../preview/PreviewHole";
import { RegionOverlay } from "../preview/RegionOverlay";
import { SafeZones } from "../preview/SafeZones";
import { fitCanvas } from "../preview/placement";
import { rows } from "../timeline/geometry";
import { Transport } from "../transport/Transport";
import { Glyph } from "../ui/controls";
import { useSize } from "../ui/useSize";
import type { Editor, EditorState } from "./editor";
import { Pictures } from "./pictures";

/** True in a plain browser (`pnpm dev`), where no engine draws behind the page. */
function standingIn(): boolean {
  return typeof document !== "undefined" && document.body.classList.contains("browser");
}

/** The topmost clip with a picture under the playhead, for the stand-in stage. */
function shownClip(state: EditorState): ClipView | null {
  const timeline = state.timeline;
  if (!timeline) return null;
  for (const track of rows(timeline)) {
    if (track.kind !== "video") continue;
    const clip = track.clips.find((c) => c.asset && c.start.flicks <= state.playhead && state.playhead < c.end.flicks);
    if (clip) return clip;
  }
  return null;
}

/**
 * Outside the desktop app nothing draws behind the page: a frame of the clip under the
 * playhead stands in for the preview, from the stand-in engine's pictures.
 */
function StandIn({ editor, state }: { editor: Editor; state: EditorState }) {
  useSyncExternalStore(editor.pictures.subscribe, editor.pictures.version, editor.pictures.version);
  const stage = useRef<HTMLDivElement>(null);
  const size = useSize(stage);
  const format = state.timeline?.format;
  const clip = shownClip(state);
  const asset = clip ? state.media.find((a) => a.id === clip.asset) : undefined;
  const at = clip ? (clip.source_in?.flicks ?? 0) + (state.playhead - clip.start.flicks) : 0;
  const image = asset && Pictures.pictured(asset) ? editor.pictures.frame(asset, Math.floor(at / 352_800_000) * 352_800_000) : null;
  const box = fitCanvas({ x: 0, y: 0, ...size }, format ? format.width / format.height : 16 / 9);
  return (
    <div className="stand-in" ref={stage}>
      <div className="stand-in-canvas" style={{ left: box.x, top: box.y, width: box.width, height: box.height }}>
        {image ? <img src={image} alt="" /> : <Glyph icon={MonitorPlay} size="lg" />}
        <span className="stand-in-note">{t("preview.standIn")}</span>
      </div>
    </div>
  );
}

/**
 * The player: the timeline's name and canvas over the preview, which the engine draws
 * into the hole the page leaves (with the overlays for masks, guides, regions and safe
 * zones over it), and the control bar under it. Floating panels sit over its corner.
 */
export function Player({ editor, state, children }: { editor: Editor; state: EditorState; children?: React.ReactNode }) {
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState<[number, number]>([0, 0]);
  const [safeZones, setSafeZones] = useState(false);
  const format = state.timeline?.format;
  const aspect = format ? format.width / format.height : 16 / 9;
  const name = state.opened?.info.timelines.find((tl) => tl.id === state.opened?.info.primary_timeline)?.name ?? state.timeline?.name;
  return (
    <section className="panel player" aria-label={t("preview.player")}>
      <header className="panel-head player-head">
        <Glyph icon={MonitorPlay} size="sm" />
        <h2>{name ?? t("preview.player")}</h2>
        {format && (
          <span className="faint">
            {t("preview.format", { width: format.width, height: format.height, rate: format.rate })}
          </span>
        )}
      </header>
      <div className="player-stage">
        {standingIn() && <StandIn editor={editor} state={state} />}
        <PreviewHole
          editor={editor}
          zoom={zoom}
          pan={pan}
          onWheel={(event, size) => {
            if (event.ctrlKey || event.metaKey) {
              event.preventDefault();
              const index = ZOOMS.indexOf(zoom as (typeof ZOOMS)[number]);
              const next = ZOOMS[Math.min(ZOOMS.length - 1, Math.max(0, index + (event.deltaY < 0 ? 1 : -1)))] ?? 1;
              setZoom(next);
              setPan(clampPan(pan, next, size));
            } else if (zoom > 1) {
              event.preventDefault();
              setPan(clampPan([pan[0] - event.deltaX, pan[1] - event.deltaY], zoom, size));
            }
          }}
        >
          {safeZones && <SafeZones aspect={aspect} />}
          <MaskOverlay editor={editor} state={state} />
          <Guides editor={editor} state={state} />
          <RegionOverlay editor={editor} state={state} />
        </PreviewHole>
        {children}
      </div>
      <Transport
        editor={editor}
        state={state}
        zoom={zoom}
        onZoom={(z) => {
          setZoom(z);
          if (z === 1) setPan([0, 0]);
        }}
        safeZones={safeZones}
        onSafeZones={setSafeZones}
      />
    </section>
  );
}
