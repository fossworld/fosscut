import { useLayoutEffect, useRef, useState } from "react";

import { type Box, fitCanvas } from "./placement";

/** The safe zones as fractions of the canvas: action within 93 percent, titles within 90. */
export const SAFE = { action: 0.93, title: 0.9 } as const;

/** The box `fraction` of `canvas` takes, centred in it. */
export function inset(canvas: Box, fraction: number): Box {
  const width = canvas.width * fraction;
  const height = canvas.height * fraction;
  return { x: canvas.x + (canvas.width - width) / 2, y: canvas.y + (canvas.height - height) / 2, width, height };
}

/** Lines over the preview where action and titles stay safe from cropping, and the centre. */
export function SafeZones({ aspect }: { aspect: number }) {
  const box = useRef<SVGSVGElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
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
  const canvas = fitCanvas({ x: 0, y: 0, ...size }, aspect);
  const action = inset(canvas, SAFE.action);
  const title = inset(canvas, SAFE.title);
  const cx = canvas.x + canvas.width / 2;
  const cy = canvas.y + canvas.height / 2;
  return (
    <svg className="safe-zones" ref={box} aria-hidden="true">
      {canvas.width > 0 && (
        <>
          <rect className="action" x={action.x} y={action.y} width={action.width} height={action.height} />
          <rect className="title" x={title.x} y={title.y} width={title.width} height={title.height} />
          <path className="centre" d={`M${cx - 8} ${cy}H${cx + 8}M${cx} ${cy - 8}V${cy + 8}`} />
        </>
      )}
    </svg>
  );
}
