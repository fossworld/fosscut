import { Clapperboard, Ellipsis, Plus, Smartphone, X } from "lucide-react";
import { useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import { ASPECTS, aspectName } from "../app/layout";
import { t } from "../i18n";
import { Glyph, IconButton, Menu } from "../ui/controls";

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
              className="tab-name-input"
              aria-label={t("timeline.rename")}
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
                type="button"
                role="tab"
                aria-selected={active}
                title={t("timeline.rename.help")}
                onClick={() => void editor.showTimeline(tab.id)}
                onDoubleClick={() => setNaming({ id: tab.id, name: tab.name })}
              >
                {tab.role === "compound" && <Glyph icon={Clapperboard} size="sm" />}
                <span>{tab.role === "compound" ? t("timeline.compound", { name: tab.name }) : tab.name}</span>
                <span className="tab-aspect">{aspectName(tab.format.width, tab.format.height)}</span>
              </button>
              {info.timelines.length > 1 && (
                <IconButton icon={X} size="sm" className="close" label={t("timeline.remove.help")} onClick={() => void editor.removeTimeline(tab.id)} />
              )}
            </span>
          );
        })}
      <IconButton icon={Plus} size="sm" className="add" label={t("timeline.add.help")} onClick={() => void editor.addTimeline()} />
      <span className="spacer" />
      <Menu
        label={t("timeline.actions")}
        icon={Ellipsis}
        align="end"
        className="timeline-actions"
        disabled={!state.timeline}
        items={[
          { id: "vertical-person", label: t("timeline.vertical.person"), icon: Smartphone, onSelect: () => void editor.verticalVersion("person") },
          { id: "vertical-subject", label: t("timeline.vertical.subject"), icon: Smartphone, onSelect: () => void editor.verticalVersion("subject") },
          ...ASPECTS.map((aspect, index) => ({
            id: `version-${aspect.id}`,
            label: t("app.aspect.version", { aspect: aspect.id }),
            detail: `${aspect.width} × ${aspect.height}`,
            separated: index === 0,
            onSelect: () => void editor.makeVersion(aspect.width, aspect.height),
          })),
        ]}
      />
    </div>
  );
}
