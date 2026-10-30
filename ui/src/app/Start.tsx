import { useState } from "react";

import type { ProjectSummary } from "../engine/types";
import type { Editor, EditorState } from "./editor";

/** A size in bytes as people read it: 12 KB, 3.4 MB. */
export function readableSize(bytes: number): string {
  const units = ["bytes", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return unit === 0 ? `${bytes} bytes` : `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/** The projects whose name or folder has every word of `search`. */
export function matching(projects: ProjectSummary[], search: string): ProjectSummary[] {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  return projects.filter((p) => words.every((w) => `${p.name} ${p.path}`.toLowerCase().includes(w)));
}

/** Shown when no project is open: the project browser, with search and the trash. */
export function Start({ editor, state }: { editor: Editor; state: EditorState }) {
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
            Open project
          </button>
          <button onClick={() => void editor.create()}>New project</button>
        </div>
        <p className="muted">
          Agents connected through <code>fosscut mcp</code> edit the project you open here, and you see their changes as
          they land.
        </p>
        {state.projects.length > 0 && (
          <input type="search" placeholder="Search projects" value={search} onChange={(e) => setSearch(e.target.value)} />
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
              <button className="link" title="Move the project to the trash" onClick={() => void editor.trashProject(project.path)}>
                Move to trash
              </button>
            </li>
          ))}
        </ul>
        {state.projects.length > 0 && projects.length === 0 && <p className="muted">No project matches.</p>}
        {state.trash.length > 0 && (
          <div className="trash">
            <button className="link" onClick={() => setShowTrash(!showTrash)}>
              Trash ({state.trash.length})
            </button>
            {showTrash && (
              <ul>
                {state.trash.map((entry) => (
                  <li key={entry.id}>
                    <span title={entry.original}>{entry.name}</span>
                    <button onClick={() => void editor.restoreProject(entry.id)}>Restore</button>
                    <button onClick={() => void editor.deleteTrashed(entry.id)}>Delete for good</button>
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
