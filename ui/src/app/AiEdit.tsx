import { Bot, Check, ChevronDown, ChevronUp, Copy, FileVideo, RotateCcw, Settings as Gear, Sparkles, X } from "lucide-react";
import { useEffect, useState } from "react";

import { formatNumber, language, t } from "../i18n";
import type { MessageKey } from "../i18n";
import { Glyph, IconButton, Segmented, cx } from "../ui/controls";
import { type AiEditDialog, agentChoice, clock, commandLine, fileName, runState } from "./agentFlow";
import type { Editor, EditorState } from "./editor";
import { ASPECTS, type AspectId } from "./layout";
import { Problem } from "./Problem";

/** How often a running agent is read, in milliseconds. */
const POLL = 1000;

type Props = { editor: Editor; dialog: AiEditDialog };

/** A button that copies `text`, and says so for a moment. */
function CopyButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);
  return (
    <button
      type="button"
      className="small"
      onClick={() =>
        void navigator.clipboard
          ?.writeText(text)
          .then(() => setCopied(true))
          .catch(() => undefined)
      }
    >
      <Glyph icon={copied ? Check : Copy} size="sm" />
      {copied ? t("agent.copied") : label}
    </button>
  );
}

/** The first step: the video and the canvas of the project made from it. */
function VideoStep({ editor, dialog }: Props) {
  const names: Record<AspectId, string> = {
    "9:16": t("app.start.aspect.vertical"),
    "16:9": t("app.start.aspect.landscape"),
    "1:1": t("app.start.aspect.square"),
    "4:5": t("app.start.aspect.portrait"),
  };
  const order: AspectId[] = ["9:16", "16:9", "1:1", "4:5"];
  const aspect = ASPECTS.find((a) => a.id === dialog.aspect);
  return (
    <>
      <p className="hint">{t("agent.video.intro")}</p>
      <div className="agent-video">
        <span className="agent-video-icon" aria-hidden="true">
          <Glyph icon={FileVideo} size="lg" />
        </span>
        <span className="agent-video-name">
          <span className="name">{dialog.video ? fileName(dialog.video) : ""}</span>
          <span className="path" title={dialog.video ?? undefined}>
            {dialog.video}
          </span>
        </span>
        <button type="button" className="small" disabled={dialog.busy} onClick={() => void editor.pickAiVideo()}>
          {t("agent.video.other")}
        </button>
      </div>
      <div className="choice wide">
        <span title={t("agent.video.canvas.help")}>{t("agent.video.canvas")}</span>
        <Segmented<AspectId>
          label={t("agent.video.canvas")}
          value={dialog.aspect}
          onChange={(id) => editor.setAiAspect(id)}
          options={order.map((id) => ({ id, label: `${names[id]} ${id}` }))}
        />
      </div>
      {aspect && (
        <p className="hint muted">
          {t("agent.video.size", { width: formatNumber(aspect.width), height: formatNumber(aspect.height) })} · {t("agent.video.save")}
        </p>
      )}
      {dialog.error && <Problem error={dialog.error} />}
      <div className="buttons">
        <span className="spacer" />
        <button disabled={dialog.busy} onClick={() => editor.closeAiEdit()}>
          {t("agent.cancel")}
        </button>
        <button className="primary" disabled={dialog.busy || !dialog.video} onClick={() => void editor.createAiProject()}>
          {dialog.busy ? t("agent.video.busy") : t("agent.video.create")}
        </button>
      </div>
    </>
  );
}

/** The brief, editable, and the caps, or, with no agent, how to run one by hand. */
function BriefStep({ editor, dialog }: Props) {
  const setup = dialog.setup;
  const checked = agentChoice(dialog, false);
  const problem = "problem" in checked ? t(checked.problem, { limit: formatNumber(setup?.max_turns_limit ?? 1000) }) : null;
  const brief = (
    <label className="choice wide column">
      <span>{t("agent.brief.label")}</span>
      <textarea className="brief" rows={9} value={dialog.brief} spellCheck={false} onChange={(event) => editor.editAiBrief({ brief: event.target.value })} />
    </label>
  );
  if (setup && !setup.agent) {
    return (
      <>
        <div className="licence" role="note">
          <p className="licence-title">{t("agent.missing.title")}</p>
          <p>{t("agent.missing.text")}</p>
          {setup.tried.length > 0 && (
            <details className="tried">
              <summary>{t("agent.missing.tried")}</summary>
              <ul className="addresses">
                {setup.tried.map((place) => (
                  <li key={place}>{place}</li>
                ))}
              </ul>
            </details>
          )}
        </div>
        {brief}
        <div className="row-end">
          <CopyButton text={dialog.brief} label={t("agent.copy.brief")} />
        </div>
        <p className="hint">{t("agent.missing.connect")}</p>
        {setup.setup.map((line) => (
          <div className="copy-line" key={line}>
            <pre className="command">{line}</pre>
            <CopyButton text={line} label={t("agent.copy")} />
          </div>
        ))}
        <p className="hint">{t("agent.missing.server")}</p>
        <div className="copy-line">
          <pre className="command">{setup.server}</pre>
          <CopyButton text={setup.server} label={t("agent.copy")} />
        </div>
        {dialog.error && <Problem error={dialog.error} />}
        <div className="buttons">
          <button onClick={() => editor.openSettings("general")}>
            <Glyph icon={Gear} size="sm" />
            {t("agent.missing.settings")}
          </button>
          <span className="spacer" />
          <button onClick={() => editor.closeAiEdit()}>{t("agent.close")}</button>
          <button className="primary" disabled={dialog.busy} onClick={() => void editor.loadAgent()}>
            <Glyph icon={RotateCcw} size="sm" />
            {t("agent.missing.again")}
          </button>
        </div>
      </>
    );
  }
  const via = setup?.agent ? (`agent.brief.via.${setup.agent.via}` as MessageKey) : null;
  return (
    <>
      {setup?.agent && (
        <div className="agent-found">
          <span className="agent-video-icon" aria-hidden="true">
            <Glyph icon={Bot} size="lg" />
          </span>
          <span className="agent-video-name">
            <span className="name">
              {setup.agent.name}
              {via && <span className="muted"> · {t(via)}</span>}
            </span>
            <span className="path" title={setup.agent.path}>
              {setup.agent.path}
            </span>
          </span>
        </div>
      )}
      {brief}
      <div className="brief-foot">
        <p className="hint">{t("agent.brief.help", { skill: setup?.skill ?? "talking-head-reel" })}</p>
        <button type="button" className="link" disabled={!setup || dialog.brief === setup.brief} onClick={() => editor.resetAiBrief()}>
          {t("agent.brief.reset")}
        </button>
      </div>
      <div className="fields agent-caps">
        <label className="field" title={t("agent.turns.help")}>
          <span />
          <span>{t("agent.turns")}</span>
          <input value={dialog.maxTurns} inputMode="numeric" onChange={(event) => editor.editAiBrief({ maxTurns: event.target.value })} />
          <span className="unit">{t("agent.turns.unit")}</span>
        </label>
        <label className="field" title={t("agent.budget.help")}>
          <span />
          <span>{t("agent.budget")}</span>
          <input value={dialog.budget} inputMode="decimal" placeholder={t("agent.budget.placeholder")} onChange={(event) => editor.editAiBrief({ budget: event.target.value })} />
          <span className="unit">{t("agent.budget.unit")}</span>
        </label>
      </div>
      {problem && <p className="hint warning">{problem}</p>}
      {dialog.error && <Problem error={dialog.error} />}
      <div className="buttons">
        <span className="spacer" />
        <button disabled={dialog.busy} onClick={() => editor.closeAiEdit()}>
          {t("agent.brief.later")}
        </button>
        <button className="primary" disabled={dialog.busy || !setup || problem !== null} onClick={() => void editor.checkAgent()}>
          {t("agent.brief.continue")}
        </button>
      </div>
    </>
  );
}

/** The notes and the exact command, to agree to before anything runs. */
function AgreeStep({ editor, dialog }: Props) {
  return (
    <>
      <div className="licence" role="note">
        <p className="licence-title">{t("agent.agree.title")}</p>
        {dialog.notes.map((note) => (
          <p key={note}>{note}</p>
        ))}
      </div>
      <p className="hint">{t("agent.agree.command")}</p>
      <pre className="command">{commandLine(dialog.command, t("agent.agree.brief"))}</pre>
      <p className="hint">{t("agent.agree.help")}</p>
      {dialog.error && <Problem error={dialog.error} />}
      <div className="buttons">
        <button onClick={() => editor.backToBrief()}>{t("agent.back")}</button>
        <span className="spacer" />
        <button onClick={() => editor.closeAiEdit()}>{t("agent.cancel")}</button>
        <button className="primary" disabled={dialog.busy} onClick={() => void editor.confirmAgent()}>
          <Glyph icon={Sparkles} size="sm" />
          {t("agent.agree.start")}
        </button>
      </div>
    </>
  );
}

/** US dollars in the interface's language. */
function dollars(value: number): string {
  return new Intl.NumberFormat(language(), { style: "currency", currency: "USD", minimumFractionDigits: 2 }).format(value);
}

/**
 * The agent at work, docked beside the editor so its edits show live: turns of the cap,
 * tool calls, time and cost, what it does now and says, and cancelling; then what it did.
 */
function RunPanel({ editor, dialog }: Props) {
  const job = dialog.job;
  const running = job?.state === "running";
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => void editor.pollAgent(), POLL);
    return () => window.clearInterval(timer);
  }, [editor, running]);
  if (!job) return null;
  const report = job.agent;
  const state = runState(job);
  const turns = report?.turns ?? 0;
  const max = report?.max_turns ?? job.progress.total;
  return (
    <section className={cx("agent-run", dialog.collapsed && "collapsed")} aria-live="polite" aria-labelledby="agent-run-title">
      <header className="agent-run-head">
        <span className="dialog-icon" aria-hidden="true">
          <Glyph icon={Sparkles} size="sm" />
        </span>
        <h2 id="agent-run-title">{t("agent.run.title")}</h2>
        <span className={cx("agent-state", state)}>{t(`agent.run.state.${state}` as MessageKey)}</span>
        <IconButton
          icon={dialog.collapsed ? ChevronDown : ChevronUp}
          size="sm"
          label={dialog.collapsed ? t("agent.run.expand") : t("agent.run.collapse")}
          onClick={() => editor.collapseAiRun(!dialog.collapsed)}
        />
        {!running && <IconButton icon={X} size="sm" label={t("agent.close")} onClick={() => editor.closeAiEdit()} />}
      </header>
      {running && <progress className="agent-progress" max={Math.max(1, max)} value={turns} />}
      {!dialog.collapsed && (
        <>
          <dl className="agent-stats">
            <div>
              <dt>{t("agent.run.turns")}</dt>
              <dd>{t("agent.run.turnsOf", { turns: formatNumber(turns), max: formatNumber(max) })}</dd>
            </div>
            <div>
              <dt>{t("agent.run.tools")}</dt>
              <dd>{formatNumber(report?.tool_calls ?? 0)}</dd>
            </div>
            <div>
              <dt>{t("agent.run.time")}</dt>
              <dd>{clock(job.elapsed_ms)}</dd>
            </div>
            {report?.cost_usd !== undefined && (
              <div>
                <dt>{t("agent.run.cost")}</dt>
                <dd>{dollars(report.cost_usd)}</dd>
              </div>
            )}
          </dl>
          {running && (
            <div className="agent-now">
              {report?.last_tool ? <p className="agent-tool">{t("agent.run.now", { tool: report.last_tool })}</p> : <p className="hint">{t("agent.run.waiting")}</p>}
              {report?.last_message && <p className="agent-said">{report.last_message}</p>}
            </div>
          )}
          {report?.connected === false && <p className="hint warning">{t("agent.run.unreachable")}</p>}
          {(report?.tool_errors ?? 0) > 0 && running && <p className="hint">{t("agent.run.errors", { count: report?.tool_errors ?? 0 })}</p>}
          {running && <p className="hint">{t("agent.run.live")}</p>}
          {state === "finished" && (
            <>
              {report?.summary && (
                <div className="agent-summary">
                  <p className="licence-title">{t("agent.run.summary")}</p>
                  <p>{report.summary}</p>
                </div>
              )}
              <p className="hint">{t("agent.run.review")}</p>
            </>
          )}
          {state === "stopped" && <p className="hint">{t("agent.run.stopped")}</p>}
          {state === "limit" && <p className="hint warning">{t("agent.run.limit")}</p>}
          {state === "failed" && job.error && <Problem error={job.error} />}
          {dialog.error && <Problem error={dialog.error} />}
          <div className="buttons">
            {running ? (
              <>
                <span className="spacer" />
                <button className="danger" onClick={() => void editor.cancelAgent()}>
                  {t("agent.run.cancel")}
                </button>
              </>
            ) : (
              <>
                {state !== "finished" && <button onClick={() => editor.backToBrief()}>{t("agent.run.again")}</button>}
                <span className="spacer" />
                <button className="primary" onClick={() => editor.closeAiEdit()}>
                  {t("agent.close")}
                </button>
              </>
            )}
          </div>
        </>
      )}
    </section>
  );
}

/**
 * An AI edit of a video by the user's own agent (ADR-0043): the video and canvas, then the
 * brief and caps (or how to run an agent by hand when none is found), then the exact command
 * and the notes to agree to before anything runs, then the agent at work in a panel beside
 * the editor while its edits land.
 */
export function AiEdit({ editor, state }: { editor: Editor; state: EditorState }) {
  const dialog = state.aiEdit;
  if (!dialog.open) return null;
  if (dialog.step === "run") return <RunPanel editor={editor} dialog={dialog} />;
  const steps = ["video", "brief", "agree"] as const;
  return (
    <div className="dialog-backdrop">
      <div className={cx("dialog", "ai-edit", dialog.step)} role="dialog" aria-modal="true" aria-labelledby="ai-edit-title">
        <header className="dialog-head">
          <span className="dialog-icon" aria-hidden="true">
            <Glyph icon={Sparkles} />
          </span>
          <h2 id="ai-edit-title">{t("agent.title")}</h2>
          <span className="step-count">{t("agent.step", { step: formatNumber(steps.indexOf(dialog.step) + 1), steps: formatNumber(steps.length) })}</span>
          <IconButton icon={X} label={t("agent.close")} disabled={dialog.busy} onClick={() => editor.closeAiEdit()} />
        </header>
        {dialog.step === "video" && <VideoStep editor={editor} dialog={dialog} />}
        {dialog.step === "brief" && <BriefStep editor={editor} dialog={dialog} />}
        {dialog.step === "agree" && <AgreeStep editor={editor} dialog={dialog} />}
      </div>
    </div>
  );
}
