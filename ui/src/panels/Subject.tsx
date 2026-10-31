import { useEffect, useState } from "react";

import { SUBJECT_KEY } from "../app/assist";
import type { Assist, Editor } from "../app/editor";
import { Problem } from "../app/Problem";
import type { ClipView, EffectView, MatteSource } from "../engine/types";
import { formatNumber, t } from "../i18n";

/** How often a running analysis is read while its panel shows, in milliseconds. */
export const ANALYSIS_POLL = 500;

/** The work of `kinds` on `clip`, when it is what runs or ran last. */
export function assistFor(assist: Assist | null, kinds: Assist["kind"][], clip: string | null): Assist | null {
  return assist && kinds.includes(assist.kind) && assist.clip === clip ? assist : null;
}

/** Reads the running analysis twice a second while `on`. */
export function usePollAnalysis(editor: Editor, on: boolean): void {
  useEffect(() => {
    if (!on) return;
    const timer = window.setInterval(() => void editor.pollAnalysis(), ANALYSIS_POLL);
    return () => window.clearInterval(timer);
  }, [editor, on]);
}

/** What an analysis has done, in words. */
export function analysisLine(assist: Assist): string {
  const progress = assist.job?.progress;
  if (!progress || progress.total === 0) return t("models.analysis.starting");
  return t("models.analysis.progress", { done: progress.done, total: progress.total });
}

/** Work with local models while it runs (with a Stop for analyses), then what it did or why not. */
export function AssistState({ editor, assist, busy }: { editor: Editor; assist: Assist | null; busy?: string }) {
  if (!assist) return null;
  if (assist.busy) {
    const analyzing = assist.job !== null || assist.kind === "background" || assist.kind === "body";
    const progress = assist.job?.progress;
    return (
      <div className="outcome assist-busy" aria-live="polite">
        <p className="hint">{analyzing ? analysisLine(assist) : (busy ?? t("models.analysis.starting"))}</p>
        {analyzing && progress && progress.total > 0 && <progress max={progress.total} value={progress.done} />}
        {assist.job?.state === "running" && (
          <button className="link" onClick={() => void editor.cancelAnalysis()}>
            {t("models.analysis.cancel")}
          </button>
        )}
      </div>
    );
  }
  if (!assist.report && !assist.error) return null;
  return (
    <div className="outcome">
      {assist.error ? <Problem error={assist.error} /> : <p className="hint">{assist.report}</p>}
      <button className="link" onClick={() => editor.dismissAssist()}>
        {t("models.dismiss")}
      </button>
    </div>
  );
}

/** A slider of a number from `min` to `max`, committed when let go. */
function Slider({
  label,
  help,
  value,
  min,
  max,
  disabled,
  onCommit,
}: {
  label: string;
  help: string;
  value: number;
  min: number;
  max: number;
  disabled: boolean;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState<number | null>(null);
  const shown = draft ?? value;
  const commit = () => {
    if (draft !== null && draft !== value) onCommit(draft);
    setDraft(null);
  };
  return (
    <label className="param" title={help}>
      <span />
      <span>{label}</span>
      <span className="control">
        <input
          type="range"
          min={min}
          max={max}
          step={(max - min) / 100}
          value={shown}
          disabled={disabled}
          aria-label={label}
          onChange={(event) => setDraft(Number(event.target.value))}
          onPointerUp={commit}
          onKeyUp={commit}
        />
        <span className="value">{formatNumber(Math.round(shown * 1000) / 1000)}</span>
      </span>
    </label>
  );
}

const SOURCES: MatteSource[] = ["person", "subject"];
const FILLS = ["none", "color", "blur"] as const;

/** A `key.subject` effect's parameter, or its default. */
function param<T>(effect: EffectView | undefined, name: string, fallback: T): T {
  const value = effect?.params?.[name];
  return value === undefined ? fallback : (value as T);
}

/**
 * Removing a video clip's background (ADR-0033 item 3): analyze its matte on this computer,
 * a person by default or any subject slowly, then key the clip with it; once it is keyed,
 * the edge's threshold and feather, removing the subject instead, and what shows behind.
 */
export function Subject({ editor, clip }: { editor: Editor; clip: ClipView }) {
  const { assist } = editor.getState();
  const effect = clip.effects?.find((e) => e.type === SUBJECT_KEY);
  const current = param<MatteSource>(effect, "source", "person");
  const [chosen, setChosen] = useState<MatteSource>(current);
  const mine = assistFor(assist, ["background", "body"], clip.id);
  const busy = assist?.busy === true;
  usePollAnalysis(editor, mine?.busy === true && mine.job?.state === "running");
  const set = (params: Record<string, unknown>) =>
    effect && void editor.edit([{ command: "effect.set", args: { clip: clip.id, effect: effect.id, params } }]);
  const source = effect ? current : chosen;
  const fill = param<(typeof FILLS)[number]>(effect, "background", "none");
  return (
    <section className="effects subject">
      <h2>{t("models.background.title")}</h2>
      <label className="choice" title={t("models.background.source.help")}>
        <span>{t("models.background.source")}</span>
        <select
          value={source}
          disabled={busy}
          onChange={(event) => {
            const next = event.target.value as MatteSource;
            setChosen(next);
            // A keyed clip changes its matte at once: analyzed first when it is not cached.
            if (effect) void editor.removeBackground(clip.id, next);
          }}
        >
          {SOURCES.map((s) => (
            <option key={s} value={s}>
              {t(`models.background.source.${s}`)}
            </option>
          ))}
        </select>
      </label>
      {!effect && (
        <div className="buttons">
          <button disabled={busy} title={t("models.background.help")} onClick={() => void editor.removeBackground(clip.id, chosen)}>
            {t("models.background.start")}
          </button>
        </div>
      )}
      {effect && (
        <div className="params">
          <Slider
            key={`threshold:${String(param(effect, "threshold", 0.5))}`}
            label={t("models.background.threshold")}
            help={t("models.background.threshold.help")}
            value={param(effect, "threshold", 0.5)}
            min={0}
            max={1}
            disabled={busy}
            onCommit={(threshold) => set({ threshold })}
          />
          <Slider
            key={`feather:${String(param(effect, "feather", 0.004))}`}
            label={t("models.background.feather")}
            help={t("models.background.feather.help")}
            value={param(effect, "feather", 0.004)}
            min={0}
            max={0.1}
            disabled={busy}
            onCommit={(feather) => set({ feather })}
          />
          <label className="toggle">
            <input
              type="checkbox"
              checked={param(effect, "invert", false)}
              disabled={busy}
              onChange={(event) => set({ invert: event.target.checked })}
            />
            <span>{t("models.background.invert")}</span>
          </label>
          <label className="choice">
            <span>{t("models.background.fill")}</span>
            <select value={fill} disabled={busy} onChange={(event) => set({ background: event.target.value })}>
              {FILLS.map((f) => (
                <option key={f} value={f}>
                  {t(`models.background.fill.${f}`)}
                </option>
              ))}
            </select>
          </label>
          {fill === "color" && (
            <label className="choice">
              <span>{t("models.background.color")}</span>
              <input
                type="color"
                value={param(effect, "color", "#000000").toLowerCase()}
                disabled={busy}
                aria-label={t("models.background.color")}
                onChange={(event) => set({ color: event.target.value.toUpperCase() })}
              />
            </label>
          )}
          {fill === "blur" && (
            <Slider
              key={`blur:${String(param(effect, "blur", 0.02))}`}
              label={t("models.background.blur")}
              help={t("models.background.blur")}
              value={param(effect, "blur", 0.02)}
              min={0}
              max={0.2}
              disabled={busy}
              onCommit={(blur) => set({ blur })}
            />
          )}
        </div>
      )}
      {mine?.kind === "body" && <h2 title={t("effects.group.body.help")}>{t("models.body.title")}</h2>}
      <AssistState editor={editor} assist={mine} />
    </section>
  );
}
