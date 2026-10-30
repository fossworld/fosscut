import type { Editor, EditorState } from "./editor";

/** Shown when no project is open: open, create, or pick a recent project. */
export function Start({ editor, state }: { editor: Editor; state: EditorState }) {
  return (
    <div className="start">
      <div className="card">
        <h1>FossCut</h1>
        <p className="muted">
          Open a project to edit it here. Agents connected through <code>fosscut mcp</code> edit the same project, and
          you see their changes as they land.
        </p>
        <div className="actions">
          <button className="primary" onClick={() => void editor.pick()}>
            Open project
          </button>
          <button onClick={() => void editor.create()}>New project</button>
        </div>
        {state.recent.length > 0 && (
          <>
            <h2>Recent</h2>
            <ul className="recent">
              {state.recent.map((path) => (
                <li key={path}>
                  <button className="link" onClick={() => void editor.open(path)} title={path}>
                    {path.split(/[\\/]/).pop()}
                  </button>
                  <span className="muted path">{path}</span>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>
    </div>
  );
}
