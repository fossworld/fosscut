import { useState } from "react";

import type { Editor, EditorState } from "../app/editor";

/** The project's timelines as tabs: click one to show it, double-click to rename, + to add
 * one, x to remove it. */
export function TimelineTabs({ editor, state }: { editor: Editor; state: EditorState }) {
  const info = state.opened?.info;
  const [naming, setNaming] = useState<{ id: string; name: string } | null>(null);
  if (!info) return null;
  const finish = () => {
    if (naming) void editor.renameTimeline(naming.id, naming.name);
    setNaming(null);
  };
  return (
    <div className="timeline-tabs" role="tablist" aria-label="Timelines">
      {info.timelines
        .filter((t) => t.role !== "compound" || t.id === info.primary_timeline)
        .map((t) => {
          const active = t.id === info.primary_timeline;
          return naming?.id === t.id ? (
            <input
              key={t.id}
              autoFocus
              value={naming.name}
              onChange={(e) => setNaming({ id: t.id, name: e.target.value })}
              onBlur={finish}
              onKeyDown={(e) => {
                if (e.key === "Enter") finish();
                if (e.key === "Escape") setNaming(null);
              }}
            />
          ) : (
            <span key={t.id} className={active ? "tab active" : "tab"}>
              <button
                role="tab"
                aria-selected={active}
                title="Double-click to rename"
                onClick={() => void editor.showTimeline(t.id)}
                onDoubleClick={() => setNaming({ id: t.id, name: t.name })}
              >
                {t.role === "compound" ? `${t.name} (compound)` : t.name}
              </button>
              {info.timelines.length > 1 && (
                <button className="close" title="Remove this timeline" onClick={() => void editor.removeTimeline(t.id)}>
                  x
                </button>
              )}
            </span>
          );
        })}
      <button className="add" title="Add a timeline" onClick={() => void editor.addTimeline()}>
        +
      </button>
    </div>
  );
}
