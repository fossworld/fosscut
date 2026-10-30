import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import { drawTimeline } from "./draw";
import { parseRate } from "../transport/time";
import {
  type Drag,
  type DragTarget,
  type View,
  dragTarget,
  grabAt,
  hitTest,
  onRuler,
  scrollBy,
  unchanged,
  xToTime,
  zoomAround,
  zoomToFit,
} from "./geometry";

interface Props {
  editor: Editor;
  state: EditorState;
}

/** The timeline: click a clip to select it, drag it to move it, drag its edges to trim it,
 * drag on the ruler to move the playhead, scroll to move along it, and pinch or Mod+scroll
 * to zoom. */
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
      selection: state.selection,
      drag:
        drag && target
          ? { clip: drag.clip.id, start: target.start, end: target.end, track: target.track.id }
          : null,
    });
  }, [timeline, view, size, state.playhead, state.selection, target]);

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
    <div className="timeline" ref={box}>
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
          editor.select(hit?.clip?.id ?? null);
          if (hit?.clip) {
            dragging.current = { clip: hit.clip, track: hit.track, mode: grabAt(hit.clip, view, x), fromX: x };
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
            const next = dragTarget(drag, timeline, view, x, y, parseRate(timeline.format.rate));
            latest.current = unchanged(drag, next) ? null : next;
            setTarget(latest.current);
            return;
          }
          // Show what a press would grab.
          const hit = timeline ? hitTest(timeline, view, x, y) : null;
          const grab = hit?.clip ? grabAt(hit.clip, view, x) : null;
          event.currentTarget.style.cursor = grab === "move" ? "grab" : grab ? "ew-resize" : "default";
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
