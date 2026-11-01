import {
  Combine,
  Crop,
  EyeOff,
  Flag,
  Link2,
  Link2Off,
  ListOrdered,
  Lock,
  type LucideIcon,
  Magnet,
  Maximize2,
  Mic,
  MousePointer2,
  MoveHorizontal,
  Redo2,
  Scissors,
  ScanLine,
  ScissorsLineDashed,
  Snowflake,
  SplitSquareHorizontal,
  Trash2,
  Undo2,
  Upload,
  Video,
  VolumeX,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";

import { ASSET_DRAG, draggedAsset } from "../app/dragging";
import type { Editor, EditorState } from "../app/editor";
import { requestInspector } from "../app/layout";
import type { PictureAsset } from "../app/pictures";
import type { TrackView } from "../engine/types";
import { type MessageKey, t, useLanguage } from "../i18n";
import { useTheme } from "../theme/theme";
import { parseRate } from "../transport/time";
import { Glyph, IconButton, cx } from "../ui/controls";
import { kindIcon } from "../ui/kinds";
import { drawTimeline } from "./draw";
import {
  type Drag,
  type DragTarget,
  HEADER,
  MAX_ZOOM,
  MIN_ZOOM,
  ROW,
  RULER,
  type View,
  dragTarget,
  grabAt,
  hitTest,
  maxTop,
  onRuler,
  rowAt,
  rowY,
  rows,
  scrollBy,
  snapPoints,
  unchanged,
  xToTime,
  zoomAround,
  zoomToFit,
} from "./geometry";
import { decoded } from "./images";
import { currentPalette } from "./palette";
import { TimelineTabs } from "./TimelineTabs";

interface Props {
  editor: Editor;
  state: EditorState;
}

/** The keys of track names by kind, looked up when shown so the language can change. */
const KIND_NAMES = new Map<string, MessageKey>([
  ["video", "timeline.track.video"],
  ["audio", "timeline.track.audio"],
  ["text", "timeline.track.text"],
  ["caption", "timeline.track.caption"],
]);

/** A track's name: its own, or its kind numbered from the bottom of its kind ("Video 2"). */
export function trackName(track: TrackView, tracks: TrackView[]): string {
  if (track.name) return track.name;
  const key = KIND_NAMES.get(track.kind);
  const kind = key ? t(key) : track.kind;
  const same = tracks.filter((other) => other.kind === track.kind);
  return same.length > 1 ? t("timeline.track.numbered", { kind, number: same.indexOf(track) + 1 }) : kind;
}

/** The zoom slider's position, 0 to 100, for a zoom in pixels per second: even steps by ratio. */
export function sliderOf(zoom: number): number {
  return (Math.log(zoom / MIN_ZOOM) / Math.log(MAX_ZOOM / MIN_ZOOM)) * 100;
}

/** The zoom at a position of the slider. */
export function zoomOf(position: number): number {
  return MIN_ZOOM * Math.pow(MAX_ZOOM / MIN_ZOOM, Math.min(100, Math.max(0, position)) / 100);
}

/** The flags a track has, as the engine reports them. */
type Flagged = TrackView & { muted?: boolean; hidden?: boolean; locked?: boolean };

/** A track's header: its kind, name, and whether it is muted, hidden or locked. */
function TrackHeader({ track, tracks, y }: { track: Flagged; tracks: TrackView[]; y: number }) {
  const sound = track.kind === "audio" || track.kind === "video";
  const picture = track.kind !== "audio";
  // Only what is set shows: a muted, hidden or locked track says so at a glance.
  const flag = (on: boolean | undefined, icon: LucideIcon, label: MessageKey) =>
    on ? (
      <span className="track-flag on" role="img" aria-label={t(label)} title={t(label)}>
        <Glyph icon={icon} size="sm" />
      </span>
    ) : null;
  return (
    <div className={cx("track-header", `track-${track.kind}`)} style={{ top: y, height: ROW }}>
      <span className={`kind-badge kind-${track.kind === "text" ? "text" : track.kind}`}>
        <Glyph icon={kindIcon(track.kind === "caption" ? "caption" : track.kind)} size="sm" />
      </span>
      <span className="track-name" title={trackName(track, tracks)}>
        {trackName(track, tracks)}
      </span>
      <span className="track-flags">
        {sound && flag(track.muted, VolumeX, "timeline.track.muted")}
        {picture && flag(track.hidden, EyeOff, "timeline.track.hidden")}
        {flag(track.locked, Lock, "timeline.track.locked")}
      </span>
    </div>
  );
}

/** The timeline's tools: editing on the left, how the timeline behaves and its zoom on the right. */
function Toolbar({ editor, state, view, onZoom }: { editor: Editor; state: EditorState; view: View; onZoom: (zoom: number | "fit") => void }) {
  const timeline = state.timeline;
  const info = state.opened?.info;
  const selected = state.selection !== null;
  const recording = state.recorder.job?.state === "running";
  return (
    <div className="timeline-toolbar" role="toolbar" aria-label={t("timeline.tools")}>
      <div className="tool-group">
        <span className="segmented-icons" role="group" aria-label={t("transport.tool")}>
          <IconButton icon={MousePointer2} label={t("transport.select")} tooltip={t("transport.select.help")} action="timeline.select_tool" pressed={state.tool === "select"} onClick={() => editor.setTool("select")} />
          <IconButton icon={ScissorsLineDashed} label={t("transport.blade")} tooltip={t("transport.blade.help")} action="timeline.blade_tool" pressed={state.tool === "blade"} onClick={() => editor.setTool("blade")} />
        </span>
        <span className="divider" aria-hidden="true" />
        <IconButton icon={Undo2} label={t("app.undo")} action="basic.undo" disabled={!info?.can_undo} onClick={() => void editor.undo()} />
        <IconButton icon={Redo2} label={t("app.redo")} action="basic.redo" disabled={!info?.can_redo} onClick={() => void editor.redo()} />
        <span className="divider" aria-hidden="true" />
        <IconButton icon={Scissors} label={t("transport.split")} tooltip={t("transport.split.help")} action="timeline.split" disabled={!timeline} onClick={() => void editor.splitAtPlayhead()} />
        <IconButton icon={SplitSquareHorizontal} label={t("transport.splitAll")} tooltip={t("transport.splitAll.help")} action="timeline.split_all" disabled={!timeline} onClick={() => void editor.splitAll()} />
        <IconButton icon={Trash2} label={t("transport.delete")} tooltip={t("transport.delete.help")} action="timeline.delete" disabled={!selected} onClick={() => void editor.deleteSelection()} />
        <IconButton icon={Flag} label={t("transport.marker")} tooltip={t("transport.marker.help")} action="timeline.marker" disabled={!timeline} onClick={() => void editor.toggleMarker()} />
        <IconButton
          icon={Snowflake}
          label={t("timeline.freeze")}
          tooltip={t("timeline.freeze.help")}
          disabled={!selected}
          onClick={() => state.selection && void editor.freeze(state.selection)}
        />
        <IconButton icon={Crop} label={t("timeline.crop")} tooltip={t("timeline.crop.help")} disabled={!selected} onClick={() => requestInspector("basic", "crop")} />
        <IconButton icon={Combine} label={t("transport.compound")} tooltip={t("transport.compound.help")} action="timeline.compound" disabled={!selected} onClick={() => void editor.compound()} />
        <IconButton
          icon={ListOrdered}
          label={t("timeline.stagger")}
          tooltip={t("timeline.stagger.help")}
          disabled={state.also.length === 0}
          onClick={() => requestInspector("animation", "stagger")}
        />
        <span className="divider" aria-hidden="true" />
        <IconButton icon={Video} label={t("timeline.camera")} tooltip={t("timeline.camera.help")} disabled={!timeline} onClick={() => void editor.addCamera()} />
      </div>
      <div className="tool-group">
        <IconButton
          icon={Mic}
          label={t("timeline.voiceover")}
          tooltip={t("app.record.help")}
          className={recording ? "recording" : undefined}
          pressed={state.recorder.open}
          onClick={() => (state.recorder.open ? editor.closeRecorder() : void editor.openRecorder())}
        />
        <span className="divider" aria-hidden="true" />
        <IconButton icon={Magnet} label={t("transport.magnet")} tooltip={t("transport.magnet.help")} action="timeline.magnet" pressed={state.magnet} onClick={() => editor.setMagnet(!state.magnet)} />
        <IconButton icon={MoveHorizontal} label={t("transport.snap")} tooltip={t("transport.snap.help")} action="timeline.snapping" pressed={state.snapping} onClick={() => editor.setSnapping(!state.snapping)} />
        <IconButton icon={state.linkage ? Link2 : Link2Off} label={t("transport.linked")} tooltip={t("transport.linked.help")} pressed={state.linkage} onClick={() => editor.setLinkage(!state.linkage)} />
        <IconButton
          icon={ScanLine}
          label={t("transport.skim")}
          tooltip={t("transport.skim.help")}
          action="timeline.skimming"
          pressed={state.skimming}
          onClick={() => editor.setSkimming(!state.skimming)}
        />
        <span className="divider" aria-hidden="true" />
        <IconButton icon={ZoomOut} label={t("timeline.zoomOut")} action="timeline.zoom_out" disabled={!timeline} onClick={() => onZoom(view.zoom / 1.5)} />
        <input
          type="range"
          className="slider zoom-slider"
          min={0}
          max={100}
          step={0.5}
          value={sliderOf(view.zoom)}
          aria-label={t("timeline.zoom")}
          title={t("timeline.zoom")}
          disabled={!timeline}
          onChange={(event) => onZoom(zoomOf(Number(event.target.value)))}
        />
        <IconButton icon={ZoomIn} label={t("timeline.zoomIn")} action="timeline.zoom_in" disabled={!timeline} onClick={() => onZoom(view.zoom * 1.5)} />
        <IconButton icon={Maximize2} label={t("timeline.fit")} action="timeline.zoom_fit" disabled={!timeline} onClick={() => onZoom("fit")} />
      </div>
    </div>
  );
}

/** The timeline: click a clip to select it (or split it, with the blade), drag it to move
 * it, drag its edges to trim it, drag on the ruler to move the playhead, scroll to move
 * along it (with Shift, up and down the tracks), and pinch or Mod+scroll to zoom. Dragged
 * edges snap when snapping is on, and hovering shows the frame there when skimming is on.
 * Media dragged from the browser lands where it is dropped. With a multicam clip selected
 * and the timeline focused, the angle shortcuts (1 to 9) cut to that angle at the playhead;
 * the app's keyboard handler finds them (`multicam.angle_*` in ../shortcuts). */
export function Timeline({ editor, state }: Props) {
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  // Zero until the box is measured: the timeline is fitted to the real width.
  const [size, setSize] = useState({ width: 0, height: 240 });
  const [view, setView] = useState<View>({ zoom: 60, scroll: 0, width: 800, top: 0 });
  const fitted = useRef<string | null>(null);
  const scrubbing = useRef(false);
  const dragging = useRef<Drag | null>(null);
  // The latest drop target: state draws it, the ref is what a release commits, since a
  // release can arrive before the last move has rendered.
  const [target, setTarget] = useState<DragTarget | null>(null);
  const latest = useRef<DragTarget | null>(null);
  const [drop, setDrop] = useState<{ start: number; end: number; row: number; track?: string } | null>(null);
  const snaps = useRef<number[]>([]);
  // Track names and clip labels are drawn in the interface's language and the theme's colours.
  const lang = useLanguage();
  const theme = useTheme();
  const pictures = useSyncExternalStore(editor.pictures.subscribe, editor.pictures.version, editor.pictures.version);
  const [decodedCount, setDecoded] = useState(0);
  // Frames decode one by one: the canvas draws again once per animation frame at most.
  const scheduled = useRef(false);
  const redraw = useRef(() => {
    if (scheduled.current) return;
    scheduled.current = true;
    requestAnimationFrame(() => {
      scheduled.current = false;
      setDecoded((n) => n + 1);
    });
  });
  // The theme's colours, read again only when the theme changes.
  const palette = useMemo(() => currentPalette(theme), [theme]);

  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      const { width, height } = entry.contentRect;
      setSize({ width, height });
      setView((v) => ({ ...v, width }));
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const timeline = state.timeline;
  const deepest = maxTop(timeline, size.height);
  const top = Math.min(view.top ?? 0, deepest);
  const shown: View = { ...view, width: size.width, top };

  // Zooms asked for from the keyboard, each applied once while rendering.
  const request = state.zoomRequest;
  const [zoomed, setZoomed] = useState(0);
  if (request && request.seq !== zoomed && timeline && size.width > 0) {
    setZoomed(request.seq);
    setView(
      request.kind === "fit"
        ? { ...zoomToFit(timeline, size.width), top: view.top }
        : zoomAround(view, request.kind === "in" ? 1.5 : 1 / 1.5, size.width / 2),
    );
  }

  // Show the whole timeline when a project opens.
  const projectId = state.opened?.info.id ?? null;
  useEffect(() => {
    if (timeline && size.width > 0 && projectId !== fitted.current) {
      fitted.current = projectId;
      setView(zoomToFit(timeline, size.width));
    }
  }, [timeline, projectId, size.width]);

  useEffect(() => {
    const element = canvas.current;
    const ctx = element?.getContext("2d");
    if (!element || !ctx || size.width === 0) return;
    const scale = window.devicePixelRatio || 1;
    element.width = Math.round(size.width * scale);
    element.height = Math.round(size.height * scale);
    ctx.setTransform(scale, 0, 0, scale, 0, 0);
    const drag = dragging.current;
    const assets = new Map(state.media.map((asset) => [asset.id, asset]));
    drawTimeline(ctx, timeline, shown, size.height, {
      playhead: state.playhead,
      skim: state.skim,
      selection: state.selection,
      also: state.also,
      drag: drag && target ? { clip: drag.clip.id, start: target.start, end: target.end, track: target.track.id } : null,
      drop,
      palette,
      asset: (id) => assets.get(id),
      frame: (asset: PictureAsset, flicks: number) => {
        const url = editor.pictures.frame(asset, flicks);
        return url ? decoded(url, redraw.current) : null;
      },
      peaks: (asset: PictureAsset) => editor.pictures.peaks(asset),
    });
    // Everything the drawing depends on; `shown` is rebuilt from these each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [timeline, view, top, size, state.playhead, state.skim, state.selection, state.also, state.media, target, drop, lang, palette, pictures, decodedCount, editor]);

  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const x = event.offsetX;
      if (event.ctrlKey || event.metaKey) {
        setView((v) => zoomAround(v, Math.exp(-event.deltaY * 0.01), x));
      } else if (event.shiftKey || (Math.abs(event.deltaY) > Math.abs(event.deltaX) && event.offsetX < HEADER)) {
        // Up and down the tracks: with Shift, or over the track headers.
        setView((v) => ({ ...v, top: Math.max(0, (v.top ?? 0) + (event.deltaY || event.deltaX)) }));
      } else {
        setView((v) => scrollBy(v, event.deltaX !== 0 ? event.deltaX : event.deltaY));
      }
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => element.removeEventListener("wheel", onWheel);
  }, []);

  const pointer = (event: React.PointerEvent<HTMLCanvasElement> | React.DragEvent<HTMLElement>) => {
    const rect = (canvas.current ?? event.currentTarget).getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };

  /** Where an asset dropped at (x, y) lands: its time and the track under it, when that fits. */
  const dropAt = (asset: string, x: number, y: number) => {
    if (!timeline) return null;
    const found = state.media.find((a) => a.id === asset);
    const start = Math.max(0, xToTime(Math.max(HEADER, x), shown));
    const imageLength = 5 * 705_600_000;
    const length = found?.duration?.flicks ?? imageLength;
    const ordered = rows(timeline);
    const row = rowAt(y, top);
    const under = ordered[row];
    const fits = (track: TrackView) =>
      track.kind === (found?.kind === "audio" ? "audio" : "video") && track.clips.every((c) => c.end.flicks <= start || c.start.flicks >= start + length);
    if (under && fits(under)) return { start, end: start + length, row, track: under.id };
    // Elsewhere: the first free track of its kind, drawn at the row it was dropped on.
    return { start, end: start + length, row: Math.max(0, Math.min(ordered.length - 1, row)) };
  };

  const empty = !timeline || timeline.tracks.every((track) => track.clips.length === 0);
  return (
    <section className="panel timeline-panel" aria-label={t("timeline.label")}>
      <Toolbar
        editor={editor}
        state={state}
        view={shown}
        onZoom={(zoom) => {
          if (!timeline) return;
          setView((v) => (zoom === "fit" ? { ...zoomToFit(timeline, size.width), top: v.top } : zoomAround(v, zoom / v.zoom, playheadX(state.playhead, v, size.width))));
        }}
      />
      <TimelineTabs editor={editor} state={state} />
      <div
        className={cx("timeline", drop && "dropping")}
        ref={box}
        tabIndex={0}
        aria-label={t("timeline.label")}
        onDragOver={(event) => {
          if (!event.dataTransfer.types.includes(ASSET_DRAG)) return;
          event.preventDefault();
          event.dataTransfer.dropEffect = "copy";
          // The asset is not readable until the drop; its length shows from the bin's listing.
          const { x, y } = pointer(event);
          const id = draggedAsset();
          const place = id ? dropAt(id, x, y) : null;
          setDrop(place);
        }}
        onDragLeave={() => setDrop(null)}
        onDrop={(event) => {
          const asset = event.dataTransfer.getData(ASSET_DRAG);
          setDrop(null);
          if (!asset) return;
          event.preventDefault();
          const { x, y } = pointer(event);
          const place = dropAt(asset, x, y);
          if (place) void editor.addToTimeline(asset, { start: place.start, track: place.track });
        }}
      >
        <canvas
          ref={canvas}
          style={{ width: size.width, height: size.height }}
          onPointerDown={(event) => {
            if (!timeline) return;
            const { x, y } = pointer(event);
            if (onRuler(y)) {
              scrubbing.current = true;
              event.currentTarget.setPointerCapture(event.pointerId);
              void editor.seek(xToTime(x, shown));
              return;
            }
            const hit = hitTest(timeline, shown, x, y);
            if (state.tool === "blade" && hit?.clip) {
              void editor.bladeAt(hit.clip.id, xToTime(x, shown));
              return;
            }
            if (event.shiftKey && hit?.clip) {
              editor.toggleSelected(hit.clip.id);
              return;
            }
            editor.select(hit?.clip?.id ?? null);
            if (hit?.clip) {
              dragging.current = { clip: hit.clip, track: hit.track, mode: grabAt(hit.clip, shown, x), fromX: x };
              snaps.current = state.snapping ? snapPoints(timeline, hit.clip.id, state.playhead) : [];
              event.currentTarget.setPointerCapture(event.pointerId);
            }
          }}
          onPointerMove={(event) => {
            const { x, y } = pointer(event);
            if (scrubbing.current) {
              void editor.scrub(xToTime(x, shown));
              return;
            }
            const drag = dragging.current;
            if (drag && timeline) {
              const next = dragTarget(drag, timeline, shown, x, y, parseRate(timeline.format.rate), snaps.current);
              latest.current = unchanged(drag, next) ? null : next;
              setTarget(latest.current);
              return;
            }
            // Show what a press would grab, and the frame under the pointer when skimming.
            const hit = timeline ? hitTest(timeline, shown, x, y) : null;
            const grab = hit?.clip ? grabAt(hit.clip, shown, x) : null;
            event.currentTarget.style.cursor =
              state.tool === "blade" && hit?.clip ? "crosshair" : grab === "move" ? "grab" : grab ? "ew-resize" : onRuler(y) ? "col-resize" : "default";
            if (state.skimming && !onRuler(y) && x >= HEADER) void editor.skim(Math.max(0, xToTime(x, shown)));
          }}
          onPointerUp={() => {
            scrubbing.current = false;
            const drag = dragging.current;
            const target = latest.current;
            dragging.current = null;
            latest.current = null;
            setTarget(null);
            if (!drag || !target) return;
            if (drag.mode === "move") {
              const track = target.track.id !== drag.track.id ? target.track.id : undefined;
              void editor.moveClip(drag.clip.id, target.start, track);
            } else if (drag.mode === "trim-start") {
              void editor.trimClip(drag.clip.id, { start: target.start });
            } else {
              void editor.trimClip(drag.clip.id, { end: target.end });
            }
          }}
          onPointerLeave={() => void editor.skim(null)}
          onPointerCancel={() => {
            scrubbing.current = false;
            dragging.current = null;
            latest.current = null;
            setTarget(null);
          }}
        />
        <div className="track-headers" style={{ width: HEADER, top: RULER }} aria-label={t("timeline.tracks")}>
          {timeline &&
            rows(timeline).map((track, row) => {
              const y = rowY(row, top) - RULER;
              if (y + ROW < 0 || y > size.height) return null;
              return <TrackHeader key={track.id} track={track} tracks={timeline.tracks} y={y} />;
            })}
        </div>
        <div className="ruler-corner" style={{ width: HEADER, height: RULER }} aria-hidden="true" />
        {empty && timeline && (
          <div className="timeline-empty" style={{ left: HEADER, top: RULER }}>
            <Glyph icon={Upload} size="lg" />
            <p>{t("timeline.empty")}</p>
          </div>
        )}
      </div>
    </section>
  );
}

/** Where the playhead is on screen, so zooming from the toolbar keeps it in place. */
function playheadX(playhead: number, view: View, width: number): number {
  const x = HEADER + (playhead / 705_600_000 - view.scroll) * view.zoom;
  return x >= HEADER && x <= width ? x : HEADER + (width - HEADER) / 2;
}
