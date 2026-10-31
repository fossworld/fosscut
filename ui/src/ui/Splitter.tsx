import { useRef } from "react";

import { cx } from "./controls";

/**
 * A bar between two panels that resizes one of them: dragged, or moved with the arrow
 * keys once focused. `grows` says which way of moving the pointer makes the panel larger.
 */
export function Splitter({
  orientation,
  label,
  value,
  min,
  max,
  grows,
  onChange,
}: {
  /** `vertical` sits between columns and moves left and right. */
  orientation: "vertical" | "horizontal";
  label: string;
  value: number;
  min: number;
  max: number;
  grows: "right" | "left" | "down" | "up";
  onChange: (value: number) => void;
}) {
  const start = useRef<{ at: number; value: number } | null>(null);
  const sign = grows === "right" || grows === "down" ? 1 : -1;
  const coordinate = (event: React.PointerEvent) => (orientation === "vertical" ? event.clientX : event.clientY);
  return (
    <div
      className={cx("splitter", orientation)}
      role="separator"
      aria-orientation={orientation}
      aria-label={label}
      title={label}
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={(event) => {
        start.current = { at: coordinate(event), value };
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (!start.current) return;
        const moved = (coordinate(event) - start.current.at) * sign;
        onChange(Math.min(max, Math.max(min, start.current.value + moved)));
      }}
      onPointerUp={() => (start.current = null)}
      onPointerCancel={() => (start.current = null)}
      onKeyDown={(event) => {
        const keys = orientation === "vertical" ? ["ArrowLeft", "ArrowRight"] : ["ArrowUp", "ArrowDown"];
        const index = keys.indexOf(event.key);
        if (index < 0) return;
        event.preventDefault();
        // The arrows resize here; they do not also step the playhead.
        event.stopPropagation();
        const direction = (index === 1 ? 1 : -1) * sign;
        onChange(Math.min(max, Math.max(min, value + direction * (event.shiftKey ? 64 : 16))));
      }}
    />
  );
}
