import {
  ArrowRightLeft,
  AudioLines,
  Box,
  Captions,
  Check,
  Clapperboard,
  Layers,
  LayoutTemplate,
  type LucideIcon,
  Move,
  Music,
  Palette,
  PersonStanding,
  Plus,
  ScanEye,
  Sparkles,
  Sticker,
  SunMedium,
  Type,
  WandSparkles,
  X,
} from "lucide-react";
import { useRef, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import type { LibraryKind } from "../engine/types";
import { type MessageKey, t } from "../i18n";
import { findClip } from "../timeline/geometry";
import { Glyph, IconButton, StripBar, cx, useStripEnds } from "../ui/controls";
import { type EffectGroup, effectGroups, offeredEffects, targetsOf } from "./effectFields";
import { LibraryItems } from "./Library";
import { Media, MediaFolders } from "./Media";
import { nextClip } from "./Motion";
import { Speech } from "./Speech";

/** The categories of the resource browser, in the order of its strip. */
export const CATEGORIES = ["media", "audio", "text", "stickers", "effects", "particles", "objects", "transitions", "captions", "looks", "templates", "voice"] as const;
export type Category = (typeof CATEGORIES)[number];

const ICONS: Record<Category, LucideIcon> = {
  media: Clapperboard,
  audio: Music,
  text: Type,
  stickers: Sticker,
  effects: Sparkles,
  particles: WandSparkles,
  objects: Box,
  transitions: ArrowRightLeft,
  captions: Captions,
  looks: Palette,
  templates: LayoutTemplate,
  voice: AudioLines,
};

/** The sub-sections of the library categories, as library kinds. */
const SECTIONS: Partial<Record<Category, LibraryKind[]>> = {
  audio: ["sound", "music"],
  text: ["title", "text_style"],
  stickers: ["sticker", "shape", "background"],
};

/** The icon of an effect's card, by its category. */
export function effectIcon(category: string): LucideIcon {
  switch (category) {
    case "color":
      return Palette;
    case "light":
      return SunMedium;
    case "body":
      return PersonStanding;
    case "audio":
      return AudioLines;
    case "motion":
      return Move;
    case "detail":
      return ScanEye;
    default:
      return Sparkles;
  }
}

/** A narrow list of a category's sections. */
function SideList<T extends string>({ items, value, onChange, label }: { items: { id: T; label: string }[]; value: T; onChange: (id: T) => void; label: string }) {
  return (
    <nav className="side-list" aria-label={label}>
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          className={cx("side-item", item.id === value && "active")}
          aria-current={item.id === value ? "true" : undefined}
          onClick={() => onChange(item.id)}
          title={item.label}
        >
          <span>{item.label}</span>
        </button>
      ))}
    </nav>
  );
}

/** A search box over a category's items. */
function Search({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  return (
    <div className="browser-filters">
      <input type="search" placeholder={t("media.library.search")} aria-label={t("media.library.search")} value={value} onChange={(e) => onChange(e.target.value)} />
    </div>
  );
}

/** Titles: plain text and the title templates, added at the playhead. */
function Titles({ editor, state, search }: { editor: Editor; state: EditorState; search: string }) {
  const templates = state.catalog?.text.templates ?? [];
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  const matches = (text: string) => words.every((w) => text.toLowerCase().includes(w));
  const named = new Set(templates.map((tpl) => tpl.name));
  return (
    <>
      <ul className="library-items">
        {matches(t("transport.title.plain")) && (
          <li className="library-card lib-title">
            <span className="library-art" aria-hidden="true">
              <span className="art-text plain">{t("app.title.text")}</span>
            </span>
            <span className="library-about">
              <span className="title">{t("transport.title.plain")}</span>
              <span className="description">{t("transport.title.plain.help")}</span>
            </span>
            <span className="library-actions">
              <IconButton icon={Plus} size="sm" variant="solid" label={t("transport.title.add")} disabled={!state.timeline} onClick={() => void editor.addTitle("")} />
            </span>
          </li>
        )}
        {templates
          .filter((tpl) => matches(`${tpl.name} ${tpl.description}`))
          .map((tpl) => (
            <li key={tpl.name} className="library-card lib-title">
              <span className="library-art" aria-hidden="true">
                <span className="art-text">{tpl.lines?.[0] ?? tpl.name}</span>
              </span>
              <span className="library-about">
                <span className="title">{tpl.name.replaceAll("-", " ")}</span>
                <span className="description">{tpl.description}</span>
              </span>
              <span className="library-actions">
                <IconButton icon={Plus} size="sm" variant="solid" label={t("transport.title.add")} disabled={!state.timeline} onClick={() => void editor.addTitle(tpl.name)} />
              </span>
            </li>
          ))}
      </ul>
      {/* Library titles the catalogue does not already offer. */}
      {(state.library ?? []).some((item) => item.kind === "title" && !named.has(item.name)) && (
        <LibraryItems editor={editor} state={state} kinds={["title"]} search={search} />
      )}
    </>
  );
}

/** Effects of the catalogue by group, added to the selected clip; and adjustment layers. */
function EffectsBrowser({ editor, state, group }: { editor: Editor; state: EditorState; group: EffectGroup }) {
  const selected = state.timeline && state.selection ? findClip(state.timeline, state.selection) : null;
  const targets = selected ? targetsOf(selected, state.media) : { picture: false, sound: false };
  const all = effectGroups(state.catalog?.effects ?? []).find((g) => g.kind === group)?.effects ?? [];
  const fits = new Set((selected ? offeredEffects(all, targets) : []).map((spec) => spec.type));
  return (
    <>
      {group === "picture" && (
        <ul className="library-items">
          <li className="library-card lib-adjustment">
            <span className="library-art" aria-hidden="true">
              <Glyph icon={Layers} size="lg" />
            </span>
            <span className="library-about">
              <span className="title">{t("transport.adjustment")}</span>
              <span className="description">{t("transport.adjustment.help")}</span>
            </span>
            <span className="library-actions">
              <IconButton icon={Plus} size="sm" variant="solid" label={t("transport.adjustment")} disabled={!state.timeline} onClick={() => void editor.addAdjustment()} />
            </span>
          </li>
        </ul>
      )}
      {!selected && <p className="hint">{t("media.effects.select")}</p>}
      <ul className="library-items">
        {all.map((spec) => (
          <li key={spec.type} className={cx("library-card", `lib-effect-${spec.category}`)}>
            <span className="library-art" aria-hidden="true">
              <Glyph icon={effectIcon(spec.category)} size="lg" />
            </span>
            <span className="library-about">
              <span className="title">{spec.title}</span>
              <span className="description">{spec.summary}</span>
            </span>
            <span className="library-actions">
              <IconButton
                icon={Plus}
                size="sm"
                variant="solid"
                label={t("media.effects.add")}
                tooltip={fits.has(spec.type) ? t("media.effects.add") : t("media.effects.select")}
                disabled={!selected || !fits.has(spec.type)}
                onClick={() => selected && void editor.addEffect(selected.id, spec.type)}
              />
            </span>
          </li>
        ))}
        {all.length === 0 && <li className="hint">{t("media.nothing")}</li>}
      </ul>
    </>
  );
}

/** FossCut's 3D object presets (ADR-0040), added at the playhead in the 3D scene. */
function ObjectsBrowser({ editor, state, search }: { editor: Editor; state: EditorState; search: string }) {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  const presets = (state.catalog?.library?.models ?? []).filter((preset) =>
    words.every((w) => `${preset.name} ${preset.title} ${preset.description ?? ""} ${(preset.tags ?? []).join(" ")}`.toLowerCase().includes(w)),
  );
  return (
    <ul className="library-items">
      {presets.map((preset) => (
        <li key={preset.name} className="library-card lib-objects">
          <span className="library-art" aria-hidden="true">
            <Glyph icon={Box} size="lg" />
          </span>
          <span className="library-about">
            <span className="title">{preset.title}</span>
            <span className="description">{preset.description}</span>
          </span>
          <span className="library-actions">
            <IconButton
              icon={Plus}
              size="sm"
              variant="solid"
              label={t("media.library.add")}
              tooltip={t("objects.add.help")}
              disabled={!state.timeline}
              onClick={() => void editor.addModel({ preset: preset.name })}
            />
          </span>
        </li>
      ))}
      {presets.length === 0 && <li className="hint">{t("media.nothing")}</li>}
    </ul>
  );
}

/** Transitions of the catalogue, set on the cut after the selected clip. */
function TransitionsBrowser({ editor, state }: { editor: Editor; state: EditorState }) {
  const timeline = state.timeline;
  const selected = timeline && state.selection ? findClip(timeline, state.selection) : null;
  const next = timeline && selected ? nextClip(timeline, selected) : null;
  return (
    <>
      <p className="hint">{selected ? (next ? t("motion.transition.over") : t("motion.transition.touch")) : t("media.transitions.select")}</p>
      <ul className="library-items grid">
        {(state.catalog?.transitions ?? []).map((spec) => {
          const on = selected?.transition?.type === spec.type;
          return (
            <li key={spec.type} className={cx("library-card", "lib-transition", on && "on")}>
              <span className="library-art transition-art" aria-hidden="true">
                <span className="from" />
                <span className="to" />
              </span>
              <span className="library-about">
                <span className="title">{spec.title}</span>
                <span className="description">{spec.summary}</span>
              </span>
              <span className="library-actions">
                {on ? (
                  <IconButton icon={X} size="sm" label={t("media.transitions.remove")} onClick={() => selected && void editor.setTransition(selected.id, "")} />
                ) : (
                  <IconButton
                    icon={on ? Check : Plus}
                    size="sm"
                    variant="solid"
                    label={t("media.transitions.apply")}
                    disabled={!next}
                    onClick={() => selected && void editor.setTransition(selected.id, spec.type)}
                  />
                )}
              </span>
            </li>
          );
        })}
      </ul>
    </>
  );
}

/**
 * The resource browser on the left: a strip of categories (the project's media, sounds and
 * music, titles, stickers and shapes, effects, transitions, captions, looks, templates and
 * voices), a narrow list of a category's sections, and its items, each added where the
 * editor stands.
 */
export function Browser({ editor, state }: { editor: Editor; state: EditorState }) {
  const strip = useRef<HTMLDivElement>(null);
  const ends = useStripEnds(strip);
  const [category, setCategory] = useState<Category>("media");
  const [sections, setSections] = useState<Partial<Record<Category, string>>>({});
  const [folder, setFolder] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const kinds = SECTIONS[category];
  const section = (sections[category] ?? kinds?.[0] ?? "") as string;
  const setSection = (value: string) => setSections({ ...sections, [category]: value });
  const groups: { id: EffectGroup; label: string }[] = [
    { id: "picture", label: t("effects.group.picture") },
    { id: "light", label: t("effects.group.light") },
    { id: "body", label: t("effects.group.body") },
    { id: "sound", label: t("effects.group.sound") },
  ];
  let side: React.ReactNode = null;
  let content: React.ReactNode = null;
  if (category === "media") {
    side = <MediaFolders editor={editor} state={state} folder={folder} onFolder={setFolder} />;
    content = <Media editor={editor} state={state} folder={folder} />;
  } else if (kinds) {
    side = (
      <SideList
        label={t(`media.category.${category}` as MessageKey)}
        items={kinds.map((kind) => ({ id: kind, label: t(`media.library.plural.${kind}`) }))}
        value={section}
        onChange={setSection}
      />
    );
    content = (
      <>
        <Search value={search} onChange={setSearch} />
        {section === "title" ? (
          <Titles editor={editor} state={state} search={search} />
        ) : (
          <LibraryItems editor={editor} state={state} kinds={[section as LibraryKind]} search={search} />
        )}
      </>
    );
  } else if (category === "effects") {
    const group = (sections.effects ?? "picture") as EffectGroup;
    side = <SideList label={t("media.category.effects")} items={groups} value={group} onChange={(g) => setSections({ ...sections, effects: g })} />;
    content = <EffectsBrowser editor={editor} state={state} group={group} />;
  } else if (category === "particles") {
    content = (
      <>
        <p className="hint">{t("particles.browser.help")}</p>
        <Search value={search} onChange={setSearch} />
        <LibraryItems editor={editor} state={state} kinds={["particles"]} search={search} />
      </>
    );
  } else if (category === "objects") {
    content = (
      <>
        <p className="hint">{t("objects.browser.help")}</p>
        <Search value={search} onChange={setSearch} />
        <ObjectsBrowser editor={editor} state={state} search={search} />
      </>
    );
  } else if (category === "transitions") {
    content = <TransitionsBrowser editor={editor} state={state} />;
  } else if (category === "captions") {
    content = (
      <>
        <div className="info-card">
          <Glyph icon={Captions} size="lg" />
          <div>
            <p className="info-title">{t("media.captions.title")}</p>
            <p className="hint">{t("media.captions.help")}</p>
          </div>
        </div>
        <h3 className="browser-heading">{t("media.library.plural.caption_style")}</h3>
        <LibraryItems editor={editor} state={state} kinds={["caption_style"]} />
      </>
    );
  } else if (category === "looks") {
    content = (
      <>
        <Search value={search} onChange={setSearch} />
        <LibraryItems editor={editor} state={state} kinds={["look"]} search={search} />
      </>
    );
  } else if (category === "templates") {
    content = (
      <>
        <p className="hint">{t("media.templates.help")}</p>
        <LibraryItems editor={editor} state={state} kinds={["template"]} />
      </>
    );
  } else if (category === "voice") {
    content = <Speech editor={editor} state={state} embedded />;
  }
  return (
    <aside className="panel browser-panel" aria-label={t("media.browser")}>
      <StripBar strip={strip} ends={ends} labels={{ earlier: t("app.scroll.earlier"), later: t("app.scroll.later") }}>
        <div
          className={cx("category-strip", ends.start && "more-start", ends.end && "more-end")}
          role="tablist"
          aria-label={t("media.categories")}
          ref={strip}
        >
          {CATEGORIES.map((id) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={id === category}
              className={cx("category", id === category && "active")}
              title={t(`media.category.${id}.help` as MessageKey)}
              onClick={(event) => {
                setCategory(id);
                setSearch("");
                event.currentTarget.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
              }}
            >
              <Glyph icon={ICONS[id]} size="lg" />
              <span>{t(`media.category.${id}` as MessageKey)}</span>
            </button>
          ))}
        </div>
      </StripBar>
      <div className={cx("browser-body", side ? "with-side" : null)}>
        {side}
        <div className="browser-content" role="tabpanel">
          {content}
        </div>
      </div>
    </aside>
  );
}
