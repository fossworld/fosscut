import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import { isMulticam } from "../app/multicam";
import { angleKey, isTyping } from "../keyboard";
import { drawTimeline } from "./draw";
import { parseRate } from "../transport/time";
import {
  type Drag,
  type DragTarget,
  type View,
  dragTarget,
  findClip,
  grabAt,
  hitTest,
  onRuler,
  scrollBy,
  snapPoints,
  unchanged,
  xToTime,
  zoomAround,
  zoomToFit,
} from "./geometry";

interface Props {
  editor: Editor;
  state: EditorState;
}

/** The timeline: click a clip to select it (or split it, with the blade), drag it to move
 * it, drag its edges to trim it, drag on the ruler to move the playhead, scroll to move
 * along it, and pinch or Mod+scroll to zoom. Dragged edges snap when snapping is on, and
 * hovering shows the frame there when skimming is on. With a multicam clip selected and
 * the timeline focused, 1 to 9 cut to that angle at the playhead. */
export function Timeline({ editor, state }: Props) {
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  // Zero until the box is measured: the timeline is fitted to the real width.
  const [size, setSize] = useState({ width: 0, height: 240 });
  const [view, setView] = useState<View>({ zoom: 60, scroll: 0, width: 800 });
  const fitted = useRef<string | null>(null);
  const scrubbing = useRef(false);
  const dragging = useRef<Drag | null>(null);
  // The latest drop target: state draws it, the ref is what a release commits, since a
  // release can arrive before the last move has rendered.
  const [target, setTarget] = useState<DragTarget | null>(null);
  const latest = useRef<DragTarget | null>(null);
  const snaps = useRef<number[]>([]);

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

  // Zooms asked for from the keyboard, each applied once while rendering.
  const request = state.zoomRequest;
  const [zoomed, setZoomed] = useState(0);
  if (request && request.seq !== zoomed && state.timeline && size.width > 0) {
    setZoomed(request.seq);
    setView(
      request.kind === "fit"
        ? zoomToFit(state.timeline, size.width)
        : zoomAround(view, request.kind === "in" ? 1.5 : 1 / 1.5, size.width / 2),
    );
  }

  // Show the whole timeline when a project opens.
  const timeline = state.timeline;
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
    drawTimeline(ctx, timeline, { ...view, width: size.width }, size.height, {
      playhead: state.playhead,
      skim: state.skim,
      selection: state.selection,
      also: state.also,
      drag:
        drag && target
          ? { clip: drag.clip.id, start: target.start, end: target.end, track: target.track.id }
          : null,
    });
  }, [timeline, view, size, state.playhead, state.skim, state.selection, state.also, target]);

  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const x = event.offsetX;
      if (event.ctrlKey || event.metaKey) {
        setView((v) => zoomAround(v, Math.exp(-event.deltaY * 0.01), x));
      } else {
        setView((v) => scrollBy(v, event.deltaX !== 0 ? event.deltaX : event.deltaY));
      }
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => element.removeEventListener("wheel", onWheel);
  }, []);

  const pointer = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };

  return (
    <div
      className="timeline"
      ref={box}
      tabIndex={0}
      aria-label="Timeline"
      onKeyDown={(event) => {
        const angle = angleKey({
          key: event.key,
          metaKey: event.metaKey,
          ctrlKey: event.ctrlKey,
          shiftKey: event.shiftKey,
          altKey: event.altKey,
          typing: isTyping(event.target),
        });
        const selected = timeline && state.selection ? findClip(timeline, state.selection) : null;
        if (angle === null || !isMulticam(selected)) return;
        event.preventDefault();
        void editor.switchAngle(angle);
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
            void editor.seek(xToTime(x, view));
            return;
          }
          const hit = hitTest(timeline, view, x, y);
          if (state.tool === "blade" && hit?.clip) {
            void editor.bladeAt(hit.clip.id, xToTime(x, view));
            return;
          }
          if (event.shiftKey && hit?.clip) {
            editor.toggleSelected(hit.clip.id);
            return;
          }
          editor.select(hit?.clip?.id ?? null);
          if (hit?.clip) {
            dragging.current = { clip: hit.clip, track: hit.track, mode: grabAt(hit.clip, view, x), fromX: x };
            snaps.current = state.snapping ? snapPoints(timeline, hit.clip.id, state.playhead) : [];
            event.currentTarget.setPointerCapture(event.pointerId);
          }
        }}
        onPointerMove={(event) => {
          const { x, y } = pointer(event);
          if (scrubbing.current) {
            void editor.seek(xToTime(x, view));
            return;
          }
          const drag = dragging.current;
          if (drag && timeline) {
            const next = dragTarget(drag, timeline, view, x, y, parseRate(timeline.format.rate), snaps.current);
            latest.current = unchanged(drag, next) ? null : next;
            setTarget(latest.current);
            return;
          }
          // Show what a press would grab, and the frame under the pointer when skimming.
          const hit = timeline ? hitTest(timeline, view, x, y) : null;
          const grab = hit?.clip ? grabAt(hit.clip, view, x) : null;
          event.currentTarget.style.cursor =
            state.tool === "blade" && hit?.clip
              ? "crosshair"
              : grab === "move"
                ? "grab"
                : grab
                  ? "ew-resize"
                  : "default";
          if (state.skimming && !onRuler(y)) void editor.skim(Math.max(0, xToTime(x, view)));
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
    </div>
  );
}
