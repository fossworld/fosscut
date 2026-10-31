import {
  Bot,
  Check,
  FolderOpen,
  Keyboard,
  LoaderCircle,
  Moon,
  PanelLeft,
  PanelRight,
  Proportions,
  Settings as Gear,
  Sun,
  SunMoon,
  Upload,
} from "lucide-react";

import { t } from "../i18n";
import { who } from "../panels/runs";
import { type ThemeChoice, setTheme, useThemeChoice } from "../theme/theme";
import { Glyph, IconButton, Menu, cx } from "../ui/controls";
import { type Editor, type EditorState, mediaWorkLine } from "./editor";
import { ASPECTS, aspectName, setLayout, useLayout } from "./layout";
import { Logo } from "./Logo";

const THEME_ICONS = { dark: Moon, light: Sun, system: SunMoon } as const;

/** The app menu behind the logo: back to the projects, settings, shortcuts and theme. */
function AppMenu({ editor }: { editor: Editor }) {
  const theme = useThemeChoice();
  const themes: ThemeChoice[] = ["dark", "light", "system"];
  return (
    <Menu
      label={t("app.menu")}
      className="app-menu"
      buttonClassName="logo-button"
      items={[
        { id: "projects", label: t("app.menu.projects"), icon: FolderOpen, onSelect: () => void editor.close() },
        { id: "settings", label: t("app.settings"), icon: Gear, separated: true, onSelect: () => editor.openSettings() },
        { id: "shortcuts", label: t("app.menu.shortcuts"), icon: Keyboard, onSelect: () => editor.openSettings("shortcuts") },
        ...themes.map((choice, index) => ({
          id: `theme-${choice}`,
          label: t(`app.theme.${choice}`),
          icon: THEME_ICONS[choice],
          detail: theme === choice ? t("app.theme.current") : undefined,
          separated: index === 0,
          onSelect: () => setTheme(choice),
        })),
      ]}
    >
      <Logo />
    </Menu>
  );
}

/**
 * The bar across the top of the editor: the app menu, whether the project is saved and
 * who is editing it, the project's name, and on the right the canvas shape, the panels,
 * Settings and Export.
 */
export function TopBar({ editor, state }: { editor: Editor; state: EditorState }) {
  const layout = useLayout();
  const opened = state.opened;
  if (!opened) return null;
  const agents = state.clients.filter((c) => c.name.startsWith("agent:"));
  const working = mediaWorkLine(state.mediaWork);
  const format = state.timeline?.format;
  const shape = format ? aspectName(format.width, format.height) : "";
  const running = state.exporting.job?.state === "running";
  const job = state.exporting.job;
  const progress = job && job.progress.total > 0 ? Math.min(1, job.progress.done / job.progress.total) : null;
  return (
    <header className="topbar">
      <div className="topbar-start">
        <AppMenu editor={editor} />
        <span className="save-state" role="status" title={t("app.saved.help")}>
          <Glyph icon={Check} size="sm" />
          <span>{t("app.saved")}</span>
          <span className="faint">{t("app.revision", { revision: opened.info.revision })}</span>
        </span>
        {working && (
          <span className="chip work" role="status" title={t("app.work.help")}>
            <Glyph icon={LoaderCircle} size="sm" className="spin" />
            <span>{working}</span>
          </span>
        )}
        {agents.length > 0 && (
          <span className="chip live" title={agents.map((a) => who(a.name)).join(", ")}>
            <span className="pulse" aria-hidden="true" />
            <Glyph icon={Bot} size="sm" />
            <span>
              {agents.length === 1
                ? t("app.agents.one", { name: who(agents[0]?.name ?? "") })
                : t("app.agents.many", { count: agents.length })}
            </span>
          </span>
        )}
      </div>
      <div className="topbar-title">
        <h1 title={opened.bundle}>{opened.info.name}</h1>
      </div>
      <div className="topbar-end">
        {format && (
          <Menu
            label={t("app.aspect.help")}
            align="end"
            buttonClassName="chip-button"
            items={ASPECTS.map((aspect) => ({
              id: aspect.id,
              label: t("app.aspect.version", { aspect: aspect.id }),
              detail: `${aspect.width} × ${aspect.height}`,
              disabled: aspect.id === shape,
              onSelect: () => void editor.makeVersion(aspect.width, aspect.height),
            }))}
          >
            <Glyph icon={Proportions} size="sm" />
            <span>{shape}</span>
            <span className="faint">
              {format.width} × {format.height}
            </span>
          </Menu>
        )}
        <span className="divider" aria-hidden="true" />
        <IconButton
          icon={PanelLeft}
          label={t("app.layout.browser")}
          pressed={layout.showBrowser}
          onClick={() => setLayout({ showBrowser: !layout.showBrowser })}
        />
        <IconButton
          icon={PanelRight}
          label={t("app.layout.inspector")}
          pressed={layout.showInspector}
          onClick={() => setLayout({ showInspector: !layout.showInspector })}
        />
        <IconButton icon={Gear} label={t("app.settings")} action="basic.settings" onClick={() => editor.openSettings()} />
        <button
          type="button"
          className={cx("primary", "export-button", running && "running")}
          onClick={() => void editor.openExport()}
          disabled={!state.timeline}
          title={running ? t("app.export.running.help") : t("app.export.help")}
        >
          <Glyph icon={running ? LoaderCircle : Upload} size="sm" className={running ? "spin" : undefined} />
          <span>
            {running
              ? progress != null
                ? t("app.exporting.percent", { percent: Math.round(progress * 100) })
                : t("app.exporting")
              : t("app.export")}
          </span>
        </button>
      </div>
    </header>
  );
}
