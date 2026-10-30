import { useLayoutEffect, useRef, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import type { ClipView, MaskView } from "../engine/types";
import { findClip } from "../timeline/geometry";
import { maskValues } from "../panels/maskFields";
import { type Frame, fitCanvas, maskToPicture, pictureFrame, pictureToMask, toPicture, toScreen } from "./placement";

/** The outline of a mask in picture coordinates, for drawing. */
function outline(clip: ClipView, mask: MaskView, playhead: number, aspect: number): [number, number][][] {
  const v = maskValues(clip, mask, playhead);
  const place = (q: [number, number]) => maskToPicture(q, v.center, v.size, v.rotation, aspect);
  switch (mask.shape ?? "rectangle") {
    case "brush":
      return (mask.strokes ?? []).map((s) => s.points);
    case "path":
      return [[...(mask.points ?? []).map((p) => place(p.at)), place(mask.points?.[0]?.at ?? [0, 0])]];
    case "ellipse":
    case "heart":
    case "star":
      return [Array.from({ length: 49 }, (_, i) => place([Math.cos((i / 48) * 2 * Math.PI), Math.sin((i / 48) * 2 * Math.PI)]))];
    case "linear":
      return [[place([-1, 0]), place([1, 0])]];
    case "mirror":
      return [
        [place([-1, -1]), place([1, -1])],
        [place([-1, 1]), place([1, 1])],
      ];
    default:
      return [[place([-1, -1]), place([1, -1]), place([1, 1]), place([-1, 1]), place([-1, -1])]];
  }
}

/**
 * Over the preview: outlines of the selected clip's masks and, while a mask tool is on,
 * drawing a pen path (click to add points, Enter or Done to finish) or painting brush
 * strokes. The overlay passes pointer events through unless a tool is on.
 */
export function MaskOverlay({ editor, state }: { editor: Editor; state: EditorState }) {
  const box = useRef<SVGSVGElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [points, setPoints] = useState<[number, number][]>([]);
  const [stroke, setStroke] = useState<[number, number][] | null>(null);
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

  const timeline = state.timeline;
  const tool = state.maskTool;
  const clipId = tool?.clip ?? state.selection;
  const clip = timeline && clipId ? findClip(timeline, clipId) : null;
  const canvas: [number, number] = [timeline?.format.width ?? 1920, timeline?.format.height ?? 1080];
  const screen = fitCanvas({ x: 0, y: 0, ...size }, canvas[0] / canvas[1]);
  const frame: Frame | null = clip ? pictureFrame(clip, canvas, screen, state.playhead) : null;
  const aspect = clip?.picture ? clip.picture[0] / clip.picture[1] : 16 / 9;
  const path = (picture: [number, number][]) =>
    frame
      ? picture
          .map((p) => toScreen(frame, p))
          .map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`)
          .join(" ")
      : "";
  const local = (event: React.PointerEvent): [number, number] | null => {
    const element = box.current;
    if (!element || !frame) return null;
    const rect = element.getBoundingClientRect();
    return toPicture(frame, [event.clientX - rect.left, event.clientY - rect.top]);
  };
  const editing = tool && clip ? clip.masks?.find((m) => m.id === tool.mask) : undefined;
  const finishPath = () => {
    if (!tool) return;
    // Points go into the mask's box: the edited mask's, or the whole picture for a new one.
    const v = editing && clip ? maskValues(clip, editing, state.playhead) : { center: [0.5, 0.5] as [number, number], size: [1, 1] as [number, number], rotation: 0 };
    void editor.finishMask({ points: points.map((p) => ({ at: pictureToMask(p, v.center, v.size, v.rotation, aspect) })) });
    setPoints([]);
  };

  return (
    <svg
      ref={box}
      className={`mask-overlay${tool ? " drawing" : ""}`}
      tabIndex={tool ? 0 : -1}
      onKeyDown={(event) => {
        if (event.key === "Enter" && tool?.mode === "pen") finishPath();
        if (event.key === "Escape") {
          setPoints([]);
          editor.setMaskTool(null);
        }
      }}
      onPointerDown={(event) => {
        if (!tool) return;
        const p = local(event);
        if (!p) return;
        if (tool.mode === "pen") setPoints([...points, p]);
        else {
          event.currentTarget.setPointerCapture(event.pointerId);
          setStroke([p]);
        }
      }}
      onPointerMove={(event) => {
        if (!stroke) return;
        const p = local(event);
        if (p) setStroke([...stroke, p]);
      }}
      onPointerUp={() => {
        if (!stroke || !tool) return;
        void editor.finishMask({ stroke: { points: stroke, radius: tool.radius, erase: tool.erase } });
        setStroke(null);
      }}
    >
      {frame &&
        clip?.masks
          ?.filter((m) => m.enabled !== false)
          .flatMap((m) => outline(clip, m, state.playhead, aspect).map((line, i) => <path key={`${m.id}-${i}`} d={path(line)} />))}
      {points.length > 0 && <path className="draft" d={path(points)} />}
      {points.map((p, i) => {
        const [x, y] = frame ? toScreen(frame, p) : [0, 0];
        return <circle key={i} cx={x} cy={y} r={3} />;
      })}
      {stroke && <path className="draft" d={path(stroke)} />}
      {tool?.mode === "pen" && (
        <foreignObject x={8} y={8} width={220} height={36}>
          <div className="overlay-buttons">
            <button onClick={finishPath} disabled={points.length < 3}>
              Done
            </button>
            <button
              onClick={() => {
                setPoints([]);
                editor.setMaskTool(null);
              }}
            >
              Cancel
            </button>
          </div>
        </foreignObject>
      )}
    </svg>
  );
}
