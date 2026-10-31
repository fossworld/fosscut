import { useState } from "react";

import type { Editor } from "../app/editor";
import type { ClipView } from "../engine/types";
import { t } from "../i18n";
import { findClip } from "../timeline/geometry";
import { BLUR_MAX, type FillKind, blurAmount, canvasCommand, fillKind, fillOf, fillTargets } from "./canvasRequest";
import { Swatch } from "./Text";

/**
 * The canvas fill of the selected video and image clips: nothing, the clip's own picture
 * blurred to cover the canvas, or a colour, as one `clip.canvas` edit of them all.
 */
export function CanvasFill({ editor, clip }: { editor: Editor; clip: ClipView }) {
  const [draft, setDraft] = useState<number | null>(null);
  const timeline = editor.getState().timeline;
  const selected = editor
    .selected()
    .map((id) => (timeline ? findClip(timeline, id) : null))
    .filter((c): c is ClipView => c !== null);
  const clips = fillTargets(clip, selected);
  const kind = fillKind(clip);
  const apply = (next: FillKind, value?: number | string) =>
    void editor.edit([canvasCommand(clips, fillOf(next, clip.canvas, value))]);
  const amount = draft ?? (clip.canvas?.kind === "blur" ? clip.canvas.amount : 0);
  const commit = () => {
    if (draft !== null && blurAmount(draft) !== amount) apply("blur", draft);
    setDraft(null);
  };
  return (
    <div className="options canvas-fill" title={t("canvas.fill.help")}>
      <h3>{t("canvas.title")}</h3>
      <div className="buttons" role="radiogroup" aria-label={t("canvas.fill")}>
        {(["none", "blur", "color"] as const).map((choice) => (
          <button
            key={choice}
            role="radio"
            aria-checked={kind === choice}
            className={kind === choice ? "active" : ""}
            onClick={() => kind !== choice && apply(choice)}
          >
            {t(`canvas.${choice}`)}
          </button>
        ))}
      </div>
      {kind === "blur" && (
        <label className="param">
          <span>{t("canvas.amount")}</span>
          <span className="control">
            <input
              type="range"
              min={0}
              max={BLUR_MAX}
              step={0.005}
              value={amount}
              aria-label={t("canvas.amount")}
              onChange={(event) => setDraft(Number(event.target.value))}
              onPointerUp={commit}
              onKeyUp={commit}
              onBlur={commit}
            />
            <span className="value">{Math.round(blurAmount(amount) * 1000) / 10}%</span>
          </span>
        </label>
      )}
      {kind === "color" && clip.canvas?.kind === "color" && (
        <Swatch label={t("canvas.pick")} value={clip.canvas.color} onCommit={(color) => apply("color", color)} />
      )}
      {clips.length > 1 && <p className="muted">{t("canvas.several", { count: clips.length })}</p>}
    </div>
  );
}
