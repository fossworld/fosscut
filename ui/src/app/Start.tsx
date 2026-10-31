import { Bot, Clock, FolderOpen, Moon, Plus, RotateCcw, Settings as Gear, Sun, Trash2, Undo2, X } from "lucide-react";
import { useState } from "react";

import type { ProjectSummary } from "../engine/types";
import { language, t } from "../i18n";
import { setTheme, useTheme } from "../theme/theme";
import { Empty, Glyph, IconButton, cx } from "../ui/controls";
import { type Editor, type EditorState, preferencesOf } from "./editor";
import { ASPECTS, type AspectId } from "./layout";
import { Mark } from "./Logo";

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

/** When a project last changed, as a date and time in the interface's language. */
function changedAt(ms: number): string {
  if (!ms) return "";
  return new Intl.DateTimeFormat(language(), { dateStyle: "medium", timeStyle: "short" }).format(new Date(ms));
}

/** A drawing of a canvas shape, for the new project choices. */
function Shape({ width, height }: { width: number; height: number }) {
  const scale = 30 / Math.max(width, height);
  return <span className="shape-preview" style={{ width: width * scale, height: height * scale }} aria-hidden="true" />;
}

/**
 * Shown when no project is open: new projects by canvas shape, opening one, the recent
 * projects as cards with a frame of each, searched by name or folder, and the trash.
 */
export function Start({ editor, state }: { editor: Editor; state: EditorState }) {
  const [search, setSearch] = useState("");
  const [view, setView] = useState<"recent" | "trash">("recent");
  const theme = useTheme();
  const projects = matching(state.projects, search);
  const rate = preferencesOf(state).timeline.frame_rate;
  const names: Record<AspectId, string> = {
    "16:9": t("app.start.aspect.landscape"),
    "9:16": t("app.start.aspect.vertical"),
    "1:1": t("app.start.aspect.square"),
    "4:5": t("app.start.aspect.portrait"),
  };
  return (
    <div className="start">
      <aside className="start-side">
        <div className="start-brand">
          <Mark size={30} />
          <span className="wordmark">FossCut</span>
        </div>
        <nav className="start-nav" aria-label={t("app.start.places")}>
          <button type="button" className={cx("side-item", view === "recent" && "active")} aria-current={view === "recent" ? "page" : undefined} onClick={() => setView("recent")}>
            <Glyph icon={Clock} size="sm" />
            <span>{t("app.start.recent")}</span>
            <span className="count">{state.projects.length}</span>
          </button>
          <button type="button" className={cx("side-item", view === "trash" && "active")} aria-current={view === "trash" ? "page" : undefined} onClick={() => setView("trash")}>
            <Glyph icon={Trash2} size="sm" />
            <span>{t("app.start.trash.title")}</span>
            {state.trash.length > 0 && <span className="count">{state.trash.length}</span>}
          </button>
        </nav>
        <div className="start-agents">
          <Glyph icon={Bot} size="sm" />
          <p>
            {t("app.start.agents.before")} <code>fosscut mcp</code> {t("app.start.agents.after")}
          </p>
        </div>
        <div className="start-foot">
          <IconButton icon={Gear} label={t("app.settings")} action="basic.settings" onClick={() => editor.openSettings()} />
          <IconButton
            icon={theme === "dark" ? Sun : Moon}
            label={theme === "dark" ? t("app.theme.light") : t("app.theme.dark")}
            onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
          />
        </div>
      </aside>
      <main className="start-main">
        {view === "recent" ? (
          <>
            <section className="start-new" aria-label={t("app.start.new")}>
              <h1>{t("app.start.title")}</h1>
              <div className="new-cards">
                {ASPECTS.map((aspect) => (
                  <button
                    key={aspect.id}
                    type="button"
                    className="new-card"
                    title={t("app.start.new.sized", { width: aspect.width, height: aspect.height, rate })}
                    onClick={() => void editor.create({ width: aspect.width, height: aspect.height })}
                  >
                    <span className="new-art">
                      <Shape width={aspect.width} height={aspect.height} />
                      <span className="plus">
                        <Glyph icon={Plus} size="sm" />
                      </span>
                    </span>
                    <span className="new-name">{names[aspect.id]}</span>
                    <span className="new-detail">{aspect.id}</span>
                  </button>
                ))}
                <button type="button" className="new-card open" onClick={() => void editor.pick()} title={t("app.start.open.help")}>
                  <span className="new-art">
                    <Glyph icon={FolderOpen} size="lg" />
                  </span>
                  <span className="new-name">{t("app.start.open")}</span>
                  <span className="new-detail">{t("app.start.open.detail")}</span>
                </button>
              </div>
            </section>
            <section className="start-recent" aria-label={t("app.start.recent")}>
              <header className="start-recent-head">
                <h2>{t("app.start.recent")}</h2>
                <span className="spacer" />
                {state.projects.length > 0 && (
                  <input type="search" placeholder={t("app.start.search")} aria-label={t("app.start.search")} value={search} onChange={(e) => setSearch(e.target.value)} />
                )}
              </header>
              {state.projects.length === 0 ? (
                <Empty icon={Clock} title={t("app.start.none")}>
                  <p className="hint">{t("app.start.none.help")}</p>
                </Empty>
              ) : (
                <ul className="projects">
                  {projects.map((project) => (
                    <li key={project.path} className="project">
                      <button type="button" className="project-open" onClick={() => void editor.open(project.path)} title={project.path}>
                        <span className="project-thumb">
                          {project.thumbnail ? <img src={project.thumbnail} alt="" /> : <Mark size={28} />}
                          <span className="badge duration">{project.duration.timecode}</span>
                        </span>
                        <span className="project-name">{project.name}</span>
                        <span className="project-detail">
                          {changedAt(project.modified_ms)} · {readableSize(project.bytes)}
                        </span>
                      </button>
                      <IconButton
                        icon={Trash2}
                        size="sm"
                        variant="solid"
                        className="project-trash"
                        label={t("app.start.trash")}
                        tooltip={t("app.start.trash.help")}
                        onClick={() => void editor.trashProject(project.path)}
                      />
                    </li>
                  ))}
                </ul>
              )}
              {state.projects.length > 0 && projects.length === 0 && <p className="hint">{t("app.start.noMatch")}</p>}
            </section>
          </>
        ) : (
          <section className="start-trash" aria-label={t("app.start.trash.title")}>
            <header className="start-recent-head">
              <h2>{t("app.start.trash.title")}</h2>
            </header>
            <p className="hint">{t("app.start.trash.note")}</p>
            {state.trash.length === 0 ? (
              <Empty icon={Trash2} title={t("app.start.trash.empty")} />
            ) : (
              <ul className="trash-list">
                {state.trash.map((entry) => (
                  <li key={entry.id}>
                    <Glyph icon={Undo2} size="sm" className="faint" />
                    <span className="trash-name" title={entry.original}>
                      {entry.name}
                      <span className="faint">{changedAt(entry.trashed_ms)}</span>
                    </span>
                    <button type="button" className="small" onClick={() => void editor.restoreProject(entry.id)}>
                      <Glyph icon={RotateCcw} size="sm" />
                      {t("app.start.restore")}
                    </button>
                    <button type="button" className="small danger" onClick={() => void editor.deleteTrashed(entry.id)}>
                      <Glyph icon={X} size="sm" />
                      {t("app.start.delete")}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        )}
      </main>
    </div>
  );
}
