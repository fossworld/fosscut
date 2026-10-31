import { useEffect } from "react";

import type { ModelInfo } from "../engine/types";
import { t } from "../i18n";
import { downloadSize, megabytes } from "./assist";
import type { Editor, InstallPrompt } from "./editor";
import { Problem } from "./Export";

/** How often a running install is read while the dialog shows, in milliseconds. */
const POLL = 500;

/** Where a model downloads from, by host. */
function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function Model({ model }: { model: ModelInfo }) {
  return (
    <li className="model">
      <div className="model-head">
        <strong>{model.name}</strong>
        <span className="muted">{t("models.install.size", { size: megabytes(model.size_bytes) })}</span>
      </div>
      <p className="hint">
        {t("models.install.license", { license: model.license })}
        {model.url ? ` · ${t("models.install.from", { host: hostOf(model.url) })}` : ""}
      </p>
      {model.attribution && <p className="hint credit">{model.attribution}</p>}
      {model.note && <p className="hint">{model.note}</p>}
    </li>
  );
}

/** The install that runs, in words, with a bar. */
function Installing({ prompt }: { prompt: InstallPrompt }) {
  const current = prompt.current;
  if (!current) return <p className="hint">{t("models.install.waiting")}</p>;
  return (
    <div className="install-progress" aria-live="polite">
      <p className="hint">
        {t("models.install.progress", {
          name: current.name,
          done: megabytes(current.done_bytes),
          total: megabytes(current.total_bytes),
        })}
      </p>
      <progress max={Math.max(1, current.total_bytes)} value={current.done_bytes} />
    </div>
  );
}

/**
 * The models something needs, shown before anything downloads (ADR-0033 item 2): each with
 * its size, license, source and credit. They install only when the person agrees, one after
 * the other, and what asked for them runs once they are installed.
 */
export function ModelInstall({ editor, prompt }: { editor: Editor; prompt: InstallPrompt }) {
  useEffect(() => {
    if (!prompt.installing) return;
    const timer = window.setInterval(() => void editor.pollInstall(), POLL);
    return () => window.clearInterval(timer);
  }, [editor, prompt.installing]);
  const feature = t(`models.feature.${prompt.purpose}`);
  return (
    <div className="dialog-backdrop">
      <div className="dialog model-install" role="dialog" aria-modal="true" aria-labelledby="model-install-title">
        <h2 id="model-install-title">{t("models.install.title", { feature })}</h2>
        <p className="hint">{t("models.install.intro")}</p>
        <ul className="models" aria-label={t("models.install.list")}>
          {prompt.models.map((model) => (
            <Model key={model.name} model={model} />
          ))}
        </ul>
        <p>{t("models.install.total", { size: megabytes(downloadSize(prompt.models)) })}</p>
        {prompt.installing && <Installing prompt={prompt} />}
        {prompt.error && <Problem error={prompt.error} />}
        <div className="buttons">
          <span className="spacer" />
          <button disabled={prompt.installing} onClick={() => editor.cancelInstall()}>
            {t("models.install.cancel")}
          </button>
          <button className="primary" disabled={prompt.installing} onClick={() => void editor.confirmInstall()}>
            {prompt.error ? t("models.install.retry") : t("models.install.confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}
