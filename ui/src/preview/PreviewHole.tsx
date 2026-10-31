import { useLayoutEffect, useRef } from "react";

import type { Editor } from "../app/editor";
import { t } from "../i18n";

/** How far the preview zooms in, relative to the canvas fitted into the player. */
export const ZOOMS = [1, 1.5, 2, 3, 4] as const;

/** Pan offsets that keep a view `zoom` times the hole's size covering the hole. */
export function clampPan(pan: [number, number], zoom: number, size: { width: number; height: number }): [number, number] {
  const limit = (extent: number) => Math.max(0, ((zoom - 1) * extent) / 2);
  const clamp = (value: number, extent: number) => Math.min(limit(extent), Math.max(-limit(extent), value));
  return [clamp(pan[0], size.width), clamp(pan[1], size.height)];
}

/**
 * The hole the preview shows through (ADR-0010, path A): a transparent box whose place on
 * the page the engine draws video into. Zoomed in, the view inside it is larger than the
 * hole and moved by `pan`; the engine draws into the view and the panels around the hole
 * hide what falls outside. It reports where the view is whenever that changes. A web build
 * substitutes a canvas here.
 */
export function PreviewHole({
  editor,
  zoom = 1,
  pan = [0, 0],
  onWheel,
  children,
}: {
  editor: Editor;
  zoom?: number;
  pan?: [number, number];
  onWheel?: (event: WheelEvent, size: { width: number; height: number }) => void;
  children?: React.ReactNode;
}) {
  const hole = useRef<HTMLDivElement>(null);
  const view = useRef<HTMLDivElement>(null);
  const wheel = useRef(onWheel);
  useLayoutEffect(() => {
    wheel.current = onWheel;
  });

  useLayoutEffect(() => {
    const element = view.current;
    const outer = hole.current;
    if (!element || !outer) return;
    const report = () => {
      const rect = element.getBoundingClientRect();
      void editor.setViewport({ x: rect.left, y: rect.top, width: rect.width, height: rect.height });
    };
    const observer = new ResizeObserver(report);
    observer.observe(element);
    observer.observe(outer);
    window.addEventListener("resize", report);
    const onWheelEvent = (event: WheelEvent) => {
      const rect = outer.getBoundingClientRect();
      wheel.current?.(event, { width: rect.width, height: rect.height });
    };
    outer.addEventListener("wheel", onWheelEvent, { passive: false });
    report();
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", report);
      outer.removeEventListener("wheel", onWheelEvent);
      void editor.setViewport({ x: 0, y: 0, width: 0, height: 0 });
    };
  }, [editor]);

  // Zooming and panning move the view without resizing the hole.
  const [panX, panY] = pan;
  useLayoutEffect(() => {
    const rect = view.current?.getBoundingClientRect();
    if (rect) void editor.setViewport({ x: rect.left, y: rect.top, width: rect.width, height: rect.height });
  }, [editor, zoom, panX, panY]);

  const size = `${zoom * 100}%`;
  const offset = (axis: 0 | 1) => `calc(${50 - zoom * 50}% + ${pan[axis]}px)`;
  return (
    // Focusable, so the canvas shortcuts (nudging the selected clips) work once clicked.
    <div className="preview-hole" ref={hole} tabIndex={-1} aria-label={t("preview.label")}>
      <div className="preview-view" ref={view} style={{ width: size, height: size, left: offset(0), top: offset(1) }}>
        {children}
      </div>
    </div>
  );
}
