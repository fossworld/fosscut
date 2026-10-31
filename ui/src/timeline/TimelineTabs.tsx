import { useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import { t } from "../i18n";

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
    <div className="timeline-tabs" role="tablist" aria-label={t("timeline.tabs")}>
      {info.timelines
        .filter((tab) => tab.role !== "compound" || tab.id === info.primary_timeline)
        .map((tab) => {
          const active = tab.id === info.primary_timeline;
          return naming?.id === tab.id ? (
            <input
              key={tab.id}
              autoFocus
              value={naming.name}
              onChange={(e) => setNaming({ id: tab.id, name: e.target.value })}
              onBlur={finish}
              onKeyDown={(e) => {
                if (e.key === "Enter") finish();
                if (e.key === "Escape") setNaming(null);
              }}
            />
          ) : (
            <span key={tab.id} className={active ? "tab active" : "tab"}>
              <button
                role="tab"
                aria-selected={active}
                title={t("timeline.rename.help")}
                onClick={() => void editor.showTimeline(tab.id)}
                onDoubleClick={() => setNaming({ id: tab.id, name: tab.name })}
              >
                {tab.role === "compound" ? t("timeline.compound", { name: tab.name }) : tab.name}
              </button>
              {info.timelines.length > 1 && (
                <button className="close" title={t("timeline.remove.help")} onClick={() => void editor.removeTimeline(tab.id)}>
                  x
                </button>
              )}
            </span>
          );
        })}
      <button className="add" title={t("timeline.add.help")} onClick={() => void editor.addTimeline()}>
        +
      </button>
    </div>
  );
}
