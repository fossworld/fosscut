import type { Editor, EditorState } from "../app/editor";
import { clock, describe, runs, who } from "./runs";

/**
 * Who is working on the project (ADR-0010, item 5): the agents connected now, and every
 * change in order, grouped by who made it. The newest change can be undone from here,
 * whoever made it.
 */
export function Activity({ editor, state }: { editor: Editor; state: EditorState }) {
  const grouped = runs(state.history);
  const newest = state.history[0];
  return (
    <>
      <section>
        <h2>Connected</h2>
        {state.clients.length === 0 ? (
          <p className="muted">
            No agent is connected. Agents using <code>fosscut mcp</code> join this project when they open it.
          </p>
        ) : (
          <ul className="clients">
            {state.clients.map((client) => (
              <li key={client.id}>
                <span className="dot" />
                {who(client.name)}
              </li>
            ))}
          </ul>
        )}
      </section>
      <section className="history">
        <h2>
          Changes
          {newest && state.opened?.info.can_undo && (
            <button className="link" onClick={() => void editor.undo()} title="Undo the newest change">
              Undo latest
            </button>
          )}
        </h2>
        {grouped.length === 0 && <p className="muted">No changes yet.</p>}
        {grouped.map((run) => (
          <div className={run.agent ? "run agent" : "run"} key={run.entries[0]?.seq}>
            <div className="who">{run.who}</div>
            <ol>
              {run.entries.map((entry) => (
                <li key={entry.seq} className={entry.undone ? "undone" : undefined}>
                  <span>{describe(entry)}</span>
                  <span className="when">{clock(entry.at_ms)}</span>
                  {entry.reason && <div className="reason">{entry.reason}</div>}
                </li>
              ))}
            </ol>
          </div>
        ))}
      </section>
    </>
  );
}
