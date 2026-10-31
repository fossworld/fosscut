import type { Editor, EditorState } from "../app/editor";
import { t } from "../i18n";
import { clock, describe, runs, who } from "./runs";

/**
 * Who is working on the project (ADR-0010, item 5): the agents connected now, and every
 * change in order, grouped by who made it. The newest change can be undone from here,
 * whoever made it.
 */
export function Activity({ editor, state }: { editor: Editor; state: EditorState }) {
  const grouped = runs(state.history);
  const newest = state.history[0];
  // The command's name sits in a code box in the sentence, wherever the language puts it.
  const [before, after] = t("activity.none").split("{code}");
  return (
    <>
      <section>
        <h2>{t("activity.connected")}</h2>
        {state.clients.length === 0 ? (
          <p className="muted">
            {before}
            <code>fosscut mcp</code>
            {after}
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
          {t("activity.changes")}
          {newest && state.opened?.info.can_undo && (
            <button className="link" onClick={() => void editor.undo()} title={t("activity.undoLatest.help")}>
              {t("activity.undoLatest")}
            </button>
          )}
        </h2>
        {grouped.length === 0 && <p className="muted">{t("activity.noChanges")}</p>}
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
