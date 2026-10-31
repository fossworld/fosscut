import { type ReactNode, useState } from "react";

import type { ProjectSummary } from "../engine/types";
import { language, t } from "../i18n";
import { type Editor, type EditorState, preferencesOf } from "./editor";

/** A size in bytes as people read it: 12 KB, 3.4 MB, in the interface's language. */
export function readableSize(bytes: number): string {
  const units = ["app.size.kb", "app.size.mb", "app.size.gb"] as const;
  let value = bytes;
  let unit = -1;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const key = units[unit];
  if (!key) return t("app.size.bytes", { count: bytes });
  const tenths = new Intl.NumberFormat(language(), { minimumFractionDigits: 1, maximumFractionDigits: 1, useGrouping: false });
  return t(key, { value: value < 10 ? tenths.format(value) : String(Math.round(value)) });
}

/** The projects whose name or folder has every word of `search`. */
export function matching(projects: ProjectSummary[], search: string): ProjectSummary[] {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  return projects.filter((p) => words.every((w) => `${p.name} ${p.path}`.toLowerCase().includes(w)));
}

/** Shown when no project is open: the project browser, with search and the trash. */
export function Start({ editor, state, settings }: { editor: Editor; state: EditorState; settings?: ReactNode }) {
  const [search, setSearch] = useState("");
  const [showTrash, setShowTrash] = useState(false);
  const projects = matching(state.projects, search);
  return (
    <div className="start">
      <div className="card browser">
        <div className="top">
          <h1>FossCut</h1>
          <span className="spacer" />
          <button className="primary" onClick={() => void editor.pick()}>
            {t("app.start.open")}
          </button>
          <button onClick={() => void editor.create()} title={t("app.start.new.help", { rate: preferencesOf(state).timeline.frame_rate })}>
            {t("app.start.new")}
          </button>
          {settings}
        </div>
        <p className="muted">
          {t("app.start.agents.before")} <code>fosscut mcp</code> {t("app.start.agents.after")}
        </p>
        {state.projects.length > 0 && (
          <input
            type="search"
            placeholder={t("app.start.search")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        )}
        <ul className="projects">
          {projects.map((project) => (
            <li key={project.path} className="project">
              <button className="open" onClick={() => void editor.open(project.path)} title={project.path}>
                {project.thumbnail ? <img src={project.thumbnail} alt="" /> : <span className="blank" />}
                <strong>{project.name}</strong>
                <span className="muted">
                  {project.duration.timecode} · {readableSize(project.bytes)}
                </span>
              </button>
              <button className="link" title={t("app.start.trash.help")} onClick={() => void editor.trashProject(project.path)}>
                {t("app.start.trash")}
              </button>
            </li>
          ))}
        </ul>
        {state.projects.length > 0 && projects.length === 0 && <p className="muted">{t("app.start.noMatch")}</p>}
        {state.trash.length > 0 && (
          <div className="trash">
            <button className="link" onClick={() => setShowTrash(!showTrash)}>
              {t("app.start.trashed", { count: state.trash.length })}
            </button>
            {showTrash && (
              <ul>
                {state.trash.map((entry) => (
                  <li key={entry.id}>
                    <span title={entry.original}>{entry.name}</span>
                    <button onClick={() => void editor.restoreProject(entry.id)}>{t("app.start.restore")}</button>
                    <button onClick={() => void editor.deleteTrashed(entry.id)}>{t("app.start.delete")}</button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
