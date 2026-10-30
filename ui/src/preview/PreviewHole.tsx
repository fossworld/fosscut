import { useLayoutEffect, useRef } from "react";

import type { Editor } from "../app/editor";

/**
 * The hole the preview shows through (ADR-0010, path A): a transparent box whose place on
 * the page the engine draws video into. It reports where it is whenever that changes. A
 * web build substitutes a canvas here.
 */
export function PreviewHole({ editor }: { editor: Editor }) {
  const hole = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const element = hole.current;
    if (!element) return;
    const report = () => {
      const rect = element.getBoundingClientRect();
      void editor.setViewport({ x: rect.left, y: rect.top, width: rect.width, height: rect.height });
    };
    const observer = new ResizeObserver(report);
    observer.observe(element);
    window.addEventListener("resize", report);
    report();
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", report);
      void editor.setViewport({ x: 0, y: 0, width: 0, height: 0 });
    };
  }, [editor]);

  return <div className="preview-hole" ref={hole} aria-label="Preview" />;
}
