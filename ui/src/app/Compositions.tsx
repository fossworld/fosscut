import { FileCode2, FolderOpen, Globe, Plus, X } from "lucide-react";
import { useEffect, useState } from "react";

import type { WebImportRequest, WebPage } from "../engine/types";
import { formatNumber, t } from "../i18n";
import { Glyph, IconButton } from "../ui/controls";
import type { Editor, EditorState, RemotionDialog, WebImportDialog } from "./editor";
import { Problem } from "./Problem";

/** How often a running render is read while its dialog shows, in milliseconds. */
const POLL = 500;

/** The form of a composition to import: what its meta tag says, or the timeline's canvas. */
export interface CompositionForm {
  name: string;
  width: string;
  height: string;
  fps: string;
  seconds: string;
}

/** The form a page starts from: its meta tag's values, the timeline's canvas and rate for the rest. */
export function compositionForm(page: WebPage, canvas: { width: number; height: number; rate: string } | null): CompositionForm {
  return {
    name: page.name,
    width: String(page.width ?? canvas?.width ?? 1080),
    height: String(page.height ?? canvas?.height ?? 1920),
    fps: page.fps ?? canvas?.rate ?? "30",
    seconds: page.duration_seconds !== null ? String(page.duration_seconds) : "5",
  };
}

/** The import request a form makes, or null with what is wrong in it. */
export function importRequest(page: WebPage, form: CompositionForm): { request: WebImportRequest } | { problem: string } {
  const whole = (text: string) => (/^\d+$/.test(text.trim()) ? Number(text) : NaN);
  const width = whole(form.width);
  const height = whole(form.height);
  if (!(width >= 1 && width <= 8192 && height >= 1 && height <= 8192)) return { problem: t("web.import.size.invalid") };
  const fps = form.fps.trim();
  if (!/^\d+(\.\d+)?(\/\d+)?$/.test(fps)) return { problem: t("web.import.fps.invalid") };
  const seconds = Number(form.seconds.replace(",", "."));
  if (!(Number.isFinite(seconds) && seconds > 0 && seconds <= 3600)) return { problem: t("web.import.duration.invalid") };
  const request: WebImportRequest = { path: page.path, width, height, fps, duration_seconds: seconds };
  if (form.name.trim() && form.name.trim() !== page.name) request.name = form.name.trim();
  return { request };
}

/**
 * Importing a web composition (ADR-0039): the page or folder chosen, its size, frame rate
 * and length (from its meta tag where it gives them), whether a browser can render it, and
 * the network addresses that keep it from importing.
 */
export function WebImport({ editor, state, dialog }: { editor: Editor; state: EditorState; dialog: WebImportDialog }) {
  const page = dialog.page;
  const canvas = state.timeline?.format ?? null;
  const [form, setForm] = useState<CompositionForm>(() => compositionForm(page, canvas));
  const [place, setPlace] = useState(true);
  const checked = importRequest(page, form);
  const field = (key: keyof CompositionForm, label: string, unit: string, help?: string) => (
    <label className="field" title={help}>
      <span />
      <span>{label}</span>
      <input value={form[key]} inputMode={key === "name" ? undefined : "decimal"} onChange={(event) => setForm({ ...form, [key]: event.target.value })} />
      <span className="unit">{unit}</span>
    </label>
  );
  return (
    <div className="dialog-backdrop">
      <div className="dialog web-import" role="dialog" aria-modal="true" aria-labelledby="web-import-title">
        <header className="dialog-head">
          <span className="dialog-icon" aria-hidden="true">
            <Glyph icon={Globe} />
          </span>
          <h2 id="web-import-title">{t("web.import.title")}</h2>
          <IconButton icon={X} label={t("web.close")} disabled={dialog.busy} onClick={() => editor.closeWebImport()} />
        </header>
        <p className="hint">{t("web.import.intro")}</p>
        <p className="path" title={page.path}>
          {page.path}
        </p>
        <label className="choice wide">
          <span>{t("web.import.name")}</span>
          <input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} />
        </label>
        <div className="fields">
          {field("width", t("web.import.width"), "px")}
          {field("height", t("web.import.height"), "px")}
          {field("fps", t("web.import.fps"), t("web.import.fps.unit"), t("web.import.fps.help"))}
          {field("seconds", t("web.import.duration"), "s")}
        </div>
        {page.width !== null && page.height !== null && page.duration_seconds !== null ? (
          <p className="hint">{t("web.import.tagged")}</p>
        ) : (
          <p className="hint">{t("web.import.untagged")}</p>
        )}
        {page.problem && <p className="hint warning">{page.problem}</p>}
        {page.remote.length > 0 && (
          <div className="problem" role="alert">
            <p>{t("web.import.remote", { count: page.remote.length })}</p>
            <ul className="addresses">
              {page.remote.map((address) => (
                <li key={address}>{address}</li>
              ))}
            </ul>
          </div>
        )}
        <p className={page.browser ? "hint" : "hint warning"}>{page.browser ? t("web.browser.found", { path: page.browser }) : t("web.browser.missing")}</p>
        {"problem" in checked && <p className="hint warning">{checked.problem}</p>}
        <label className="toggle">
          <input type="checkbox" checked={place} onChange={(event) => setPlace(event.target.checked)} />
          <span>{t("web.import.place")}</span>
        </label>
        {dialog.error && <Problem error={dialog.error} />}
        <div className="buttons">
          <span className="spacer" />
          <button disabled={dialog.busy} onClick={() => editor.closeWebImport()}>
            {t("web.cancel")}
          </button>
          <button
            className="primary"
            disabled={dialog.busy || "problem" in checked || page.remote.length > 0}
            onClick={() => "request" in checked && void editor.importComposition(checked.request, place)}
          >
            {dialog.busy ? t("web.import.busy") : t("web.import.confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Props for a composition, as JSON text; empty is none. */
export function parseProps(text: string): { props: unknown } | { problem: string } {
  if (!text.trim()) return { props: undefined };
  try {
    const props: unknown = JSON.parse(text);
    if (typeof props !== "object" || props === null || Array.isArray(props)) return { problem: t("web.remotion.props.object") };
    return { props };
  } catch {
    return { problem: t("web.remotion.props.invalid") };
  }
}

/** The first step: the project's folder, the composition, its props and its entry point. */
function Choose({ editor, dialog }: { editor: Editor; dialog: RemotionDialog }) {
  const [project, setProject] = useState(dialog.choice?.project ?? "");
  const [composition, setComposition] = useState(dialog.choice?.composition ?? "");
  const [props, setProps] = useState(dialog.choice?.props === undefined ? "" : JSON.stringify(dialog.choice.props, null, 2));
  const [entry, setEntry] = useState(dialog.choice?.entry ?? "");
  const parsed = parseProps(props);
  const ready = project.trim() !== "" && composition.trim() !== "" && "props" in parsed;
  return (
    <>
      <p className="hint">{t("web.remotion.intro")}</p>
      <div className="choice wide">
        <span>{t("web.remotion.project")}</span>
        <span className="picked">
          <input value={project} placeholder={t("web.remotion.project.placeholder")} aria-label={t("web.remotion.project")} onChange={(event) => setProject(event.target.value)} />
          <IconButton
            icon={FolderOpen}
            label={t("web.remotion.project.pick")}
            onClick={() =>
              void editor.pickRemotionProject(t("web.remotion.project.pick")).then((folder) => {
                if (folder) setProject(folder);
              })
            }
          />
        </span>
      </div>
      <label className="choice wide">
        <span>{t("web.remotion.composition")}</span>
        <input value={composition} placeholder={t("web.remotion.composition.placeholder")} onChange={(event) => setComposition(event.target.value)} />
      </label>
      <label className="choice wide column">
        <span>{t("web.remotion.props")}</span>
        <textarea rows={4} value={props} spellCheck={false} placeholder={t("web.remotion.props.placeholder")} onChange={(event) => setProps(event.target.value)} />
      </label>
      {"problem" in parsed && <p className="hint warning">{parsed.problem}</p>}
      <label className="choice wide">
        <span>{t("web.remotion.entry")}</span>
        <input value={entry} placeholder={t("web.remotion.entry.placeholder")} onChange={(event) => setEntry(event.target.value)} />
      </label>
      {dialog.error && <Problem error={dialog.error} />}
      <div className="buttons">
        <span className="spacer" />
        <button onClick={() => editor.closeRemotion()}>{t("web.cancel")}</button>
        <button
          className="primary"
          disabled={!ready || dialog.busy}
          onClick={() =>
            "props" in parsed &&
            void editor.checkRemotion({
              project: project.trim(),
              composition: composition.trim(),
              ...(parsed.props === undefined ? {} : { props: parsed.props }),
              ...(entry.trim() ? { entry: entry.trim() } : {}),
            })
          }
        >
          {t("web.remotion.continue")}
        </button>
      </div>
    </>
  );
}

/** The second step: the licence note and the command, and agreeing to them. Nothing ran yet. */
function Agree({ editor, dialog }: { editor: Editor; dialog: RemotionDialog }) {
  return (
    <>
      <div className="licence" role="note">
        <p className="licence-title">{t("web.remotion.licence")}</p>
        <p>{dialog.licence}</p>
      </div>
      <p className="hint">{t("web.remotion.command")}</p>
      <pre className="command">{dialog.command.join(" ")}</pre>
      <p className="hint">{t("web.remotion.agree.help")}</p>
      {dialog.error && <Problem error={dialog.error} />}
      <div className="buttons">
        <button onClick={() => editor.resetRemotion()}>{t("web.remotion.back")}</button>
        <span className="spacer" />
        <button onClick={() => editor.closeRemotion()}>{t("web.cancel")}</button>
        <button className="primary" disabled={dialog.busy} onClick={() => void editor.confirmRemotion()}>
          {t("web.remotion.agree")}
        </button>
      </div>
    </>
  );
}

/** The last step: the render's progress and cancelling it, then what it became. */
function Rendering({ editor, dialog }: { editor: Editor; dialog: RemotionDialog }) {
  const job = dialog.job;
  if (!job) return null;
  const { done, total } = job.progress;
  const asset = job.remotion?.asset;
  return (
    <>
      <p className="path" title={job.output}>
        {job.output}
      </p>
      {job.state === "running" && (
        <div className="install-progress" aria-live="polite">
          <p className="hint">{total > 0 ? t("web.remotion.progress", { done, total }) : t("web.remotion.starting")}</p>
          <progress max={Math.max(1, total)} value={done} />
        </div>
      )}
      {job.state === "succeeded" && <p>{t("web.remotion.done", { file: (job.output ?? job.job).split(/[\\/]/).pop() ?? job.job })}</p>}
      {job.state === "cancelled" && <p className="hint">{t("web.remotion.cancelled")}</p>}
      {job.state === "failed" && job.error && <Problem error={job.error} />}
      {dialog.error && <Problem error={dialog.error} />}
      <div className="buttons">
        {job.state === "running" ? (
          <>
            <span className="spacer" />
            <button onClick={() => editor.closeRemotion()}>{t("web.remotion.hide")}</button>
            <button className="danger" onClick={() => void editor.cancelRemotion()}>
              {t("web.remotion.cancel")}
            </button>
          </>
        ) : (
          <>
            <button onClick={() => editor.resetRemotion()}>{t("web.remotion.another")}</button>
            <span className="spacer" />
            {asset && (
              <button
                className="primary"
                onClick={() => {
                  void editor.addToTimeline(asset);
                  editor.closeRemotion();
                }}
              >
                <Glyph icon={Plus} size="sm" />
                {t("web.remotion.add")}
              </button>
            )}
            <button onClick={() => editor.closeRemotion()}>{t("web.close")}</button>
          </>
        )}
      </div>
    </>
  );
}

/**
 * Rendering a composition of the user's own Remotion project (ADR-0039): choose the project
 * and composition, read the licence note and the command and agree to them (FossCut never
 * installs or runs Remotion before), then follow the render, which imports the file when
 * it is done.
 */
export function RemotionRender({ editor, dialog }: { editor: Editor; dialog: RemotionDialog }) {
  const running = dialog.job?.state === "running";
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => void editor.pollRemotion(), POLL);
    return () => window.clearInterval(timer);
  }, [editor, running]);
  const steps = ["choose", "agree", "render"] as const;
  return (
    <div className="dialog-backdrop">
      <div className="dialog remotion" role="dialog" aria-modal="true" aria-labelledby="remotion-title">
        <header className="dialog-head">
          <span className="dialog-icon" aria-hidden="true">
            <Glyph icon={FileCode2} />
          </span>
          <h2 id="remotion-title">{t("web.remotion.title")}</h2>
          <span className="step-count">{t("web.remotion.step", { step: formatNumber(steps.indexOf(dialog.step) + 1), steps: formatNumber(steps.length) })}</span>
          <IconButton icon={X} label={t("web.close")} onClick={() => editor.closeRemotion()} />
        </header>
        {dialog.step === "choose" && <Choose editor={editor} dialog={dialog} />}
        {dialog.step === "agree" && <Agree editor={editor} dialog={dialog} />}
        {dialog.step === "render" && <Rendering editor={editor} dialog={dialog} />}
      </div>
    </div>
  );
}
