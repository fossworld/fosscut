import { useLayoutEffect, useRef, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import { RULER, guideAt, guideToScreen, screenToGuide, ticks } from "./guideLines";
import { fitCanvas } from "./placement";

/**
 * Rulers along the top and left of the preview, in canvas pixels, and guides: drag from a
 * ruler to add one, drag a guide to move it, and drag it back onto its ruler (or
 * double-click it) to remove it.
 */
export function Guides({ editor, state }: { editor: Editor; state: EditorState }) {
  const box = useRef<SVGSVGElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [dragging, setDragging] = useState<number | null>(null);
  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return;
    const observer = new ResizeObserver(() => {
      const rect = element.getBoundingClientRect();
      setSize({ width: rect.width, height: rect.height });
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const canvas = [state.timeline?.format.width ?? 1920, state.timeline?.format.height ?? 1080] as const;
  const screen = fitCanvas({ x: 0, y: 0, ...size }, canvas[0] / canvas[1]);
  const local = (event: React.PointerEvent) => {
    const rect = box.current?.getBoundingClientRect();
    return rect ? [event.clientX - rect.left, event.clientY - rect.top] : [0, 0];
  };
  if (!state.rulers) return <svg className="guides" ref={box} aria-hidden="true" />;
  return (
    <svg
      className="guides on"
      ref={box}
      onPointerDown={(event) => {
        const [x, y] = local(event) as [number, number];
        const grabbed = guideAt(state.guides, screen, x, y);
        if (grabbed >= 0) {
          setDragging(grabbed);
        } else if (y >= screen.y && y < screen.y + RULER) {
          // From the top ruler: a horizontal guide.
          setDragging(state.guides.length);
          editor.setGuide(state.guides.length, screenToGuide("y", y, screen));
        } else if (x >= screen.x && x < screen.x + RULER) {
          setDragging(state.guides.length);
          editor.setGuide(state.guides.length, screenToGuide("x", x, screen));
        } else {
          return;
        }
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (dragging === null) return;
        const guide = state.guides[dragging];
        if (!guide) return;
        const [x, y] = local(event) as [number, number];
        editor.setGuide(dragging, screenToGuide(guide.axis, guide.axis === "x" ? x : y, screen));
      }}
      onPointerUp={(event) => {
        if (dragging === null) return;
        const guide = state.guides[dragging];
        const [x, y] = local(event) as [number, number];
        const onRuler = guide && (guide.axis === "x" ? x < screen.x + RULER : y < screen.y + RULER);
        if (onRuler) editor.setGuide(dragging, null);
        setDragging(null);
      }}
      onDoubleClick={(event) => {
        const [x, y] = [event.nativeEvent.offsetX, event.nativeEvent.offsetY];
        const index = guideAt(state.guides, screen, x, y);
        if (index >= 0) editor.setGuide(index, null);
      }}
    >
      <rect className="ruler" x={screen.x} y={screen.y} width={screen.width} height={RULER} />
      <rect className="ruler" x={screen.x} y={screen.y} width={RULER} height={screen.height} />
      {ticks(canvas[0], screen.width).map((at) => {
        const x = screen.x + (at / canvas[0]) * screen.width;
        return (
          <g key={`x${at}`}>
            <line className="tick" x1={x} x2={x} y1={screen.y + RULER - 5} y2={screen.y + RULER} />
            <text x={x + 2} y={screen.y + 10}>
              {at}
            </text>
          </g>
        );
      })}
      {ticks(canvas[1], screen.height).map((at) => {
        const y = screen.y + (at / canvas[1]) * screen.height;
        return <line key={`y${at}`} className="tick" x1={screen.x + RULER - 5} x2={screen.x + RULER} y1={y} y2={y} />;
      })}
      {state.guides.map((guide, index) => {
        const at = guideToScreen(guide, screen);
        return guide.axis === "x" ? (
          <line key={index} className="guide" x1={at} x2={at} y1={screen.y} y2={screen.y + screen.height} />
        ) : (
          <line key={index} className="guide" x1={screen.x} x2={screen.x + screen.width} y1={at} y2={at} />
        );
      })}
    </svg>
  );
}
