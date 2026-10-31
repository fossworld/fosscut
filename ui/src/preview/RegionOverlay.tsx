import { useLayoutEffect, useRef, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import { t } from "../i18n";
import { findClip } from "../timeline/geometry";
import { type Frame, fitCanvas, pictureFrame } from "./placement";
import { regionCorners, regionFromDrag } from "./region";

/**
 * Over the preview: the region of a video clip to track. While drawing, a drag draws a
 * rectangle over what to follow (Escape or Cancel stops); once drawn, its outline shows
 * until it is tracked. The overlay passes pointer events through unless drawing.
 */
export function RegionOverlay({ editor, state }: { editor: Editor; state: EditorState }) {
  const box = useRef<SVGSVGElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [drag, setDrag] = useState<{ from: [number, number]; to: [number, number] } | null>(null);
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

  const pick = state.regionPick;
  const timeline = state.timeline;
  const clip = timeline && pick ? findClip(timeline, pick.clip) : null;
  const canvas: [number, number] = [timeline?.format.width ?? 1920, timeline?.format.height ?? 1080];
  const screen = fitCanvas({ x: 0, y: 0, ...size }, canvas[0] / canvas[1]);
  const frame: Frame | null = clip ? pictureFrame(clip, canvas, screen, pick?.at ?? state.playhead) : null;
  const mirror = { horizontal: clip?.transform?.flip_horizontal, vertical: clip?.transform?.flip_vertical };
  const drawing = Boolean(pick?.drawing && frame);
  const local = (event: React.PointerEvent): [number, number] => {
    const rect = box.current?.getBoundingClientRect();
    return rect ? [event.clientX - rect.left, event.clientY - rect.top] : [0, 0];
  };
  const outline = (points: [number, number][]) =>
    `${points.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ")} Z`;

  return (
    <svg
      ref={box}
      className={`region-overlay${drawing ? " drawing" : ""}`}
      tabIndex={drawing ? 0 : -1}
      aria-hidden={!pick}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          setDrag(null);
          editor.cancelRegion();
        }
      }}
      onPointerDown={(event) => {
        // The buttons over the preview are pressed, not drawn over.
        if (!drawing || (event.target as Element).closest("foreignObject")) return;
        const at = local(event);
        event.currentTarget.setPointerCapture(event.pointerId);
        event.currentTarget.focus();
        setDrag({ from: at, to: at });
      }}
      onPointerMove={(event) => {
        if (drag) setDrag({ ...drag, to: local(event) });
      }}
      onPointerUp={(event) => {
        if (!drag || !frame) return;
        const region = regionFromDrag(frame, drag.from, local(event), mirror);
        setDrag(null);
        if (region) editor.setRegion(region);
      }}
      onPointerCancel={() => setDrag(null)}
    >
      {frame && pick?.region && !drag && <path className="region" d={outline(regionCorners(frame, pick.region, mirror))} />}
      {drag && (
        <rect
          className="region draft"
          x={Math.min(drag.from[0], drag.to[0])}
          y={Math.min(drag.from[1], drag.to[1])}
          width={Math.abs(drag.to[0] - drag.from[0])}
          height={Math.abs(drag.to[1] - drag.from[1])}
        />
      )}
      {drawing && (
        <foreignObject x={8} y={8} width={320} height={36}>
          <div className="overlay-buttons">
            <span className="overlay-hint">{t("preview.region.hint")}</span>
            <button onClick={() => editor.cancelRegion()}>{t("preview.cancel")}</button>
          </div>
        </foreignObject>
      )}
    </svg>
  );
}
