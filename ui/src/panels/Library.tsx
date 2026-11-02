import {
  Check,
  Image,
  LayoutTemplate,
  type LucideIcon,
  Music,
  Palette,
  Play,
  Plus,
  Shapes,
  Sparkles,
  Sticker,
  Subtitles,
  Type,
  Volume2,
} from "lucide-react";
import { useEffect, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import type { LibraryItem, LibraryKind } from "../engine/types";
import { t } from "../i18n";
import { Glyph, IconButton, cx } from "../ui/controls";
import {
  EVERYTHING,
  KINDS,
  type LibraryFilter,
  addPlan,
  counts,
  itemDetail,
  kindName,
  kindPlural,
  placingOf,
  shownItems,
  titleOf,
} from "./libraryFilter";

/** The icon of a library kind. */
export const LIBRARY_ICONS: Record<LibraryKind, LucideIcon> = {
  sticker: Sticker,
  shape: Shapes,
  background: Image,
  title: Type,
  text_style: Type,
  caption_style: Subtitles,
  look: Palette,
  particles: Sparkles,
  template: LayoutTemplate,
  sound: Volume2,
  music: Music,
};

/**
 * Items of FossCut's own library (ADR-0028) of `kinds`, matching `search`, as cards: what
 * each is, and a button that adds it where the editor stands (looks and caption styles
 * restyle; templates build a timeline with the media chosen in the bin). Sounds can be
 * heard first.
 */
export function LibraryItems({
  editor,
  state,
  kinds,
  search = "",
  compact = false,
}: {
  editor: Editor;
  state: EditorState;
  kinds: LibraryKind[];
  search?: string;
  /** One line per item, for the inspector. */
  compact?: boolean;
}) {
  useEffect(() => {
    void editor.loadLibrary();
  }, [editor]);
  const items = (state.library ?? []).filter((item) => kinds.includes(item.kind));
  const shown = shownItems(items, { kind: "", search });
  const at = placingOf(state.timeline, state.selection, state.playhead, state.media, state.chosen);
  if (state.library === null) return <p className="hint">{t("media.library.reading")}</p>;
  if (shown.length === 0) return <p className="hint">{t("media.nothing")}</p>;
  return (
    <ul className={cx("library-items", compact ? "compact" : null)}>
      {shown.map((item) => (
        <LibraryCard key={`${item.kind}:${item.name}`} editor={editor} state={state} item={item} plan={addPlan(item, at)} showKind={kinds.length > 1} />
      ))}
    </ul>
  );
}

function LibraryCard({
  editor,
  state,
  item,
  plan,
  showKind,
}: {
  editor: Editor;
  state: EditorState;
  item: LibraryItem;
  plan: ReturnType<typeof addPlan>;
  showKind: boolean;
}) {
  const detail = itemDetail(item);
  const restyles = item.kind === "look" || item.kind === "caption_style";
  const blocked = "blocked" in plan || !state.timeline;
  return (
    <li className={cx("library-card", `lib-${item.kind}`)}>
      <span className="library-art" aria-hidden="true">
        <Glyph icon={LIBRARY_ICONS[item.kind]} size="lg" />
        {(item.kind === "title" || item.kind === "text_style" || item.kind === "sticker") && <span className="art-text">{titleOf(item)}</span>}
      </span>
      <span className="library-about">
        <span className="title">
          {titleOf(item)}
          {showKind && <span className="kind"> {kindName(item.kind)}</span>}
        </span>
        <span className="description">{item.description}</span>
        {detail && <span className="detail">{detail}</span>}
      </span>
      <span className="library-actions">
        {item.kind === "sound" && <IconButton icon={Play} size="sm" label={t("media.library.listen")} onClick={() => void editor.playSound(item.name)} />}
        <IconButton
          icon={restyles ? Check : Plus}
          size="sm"
          variant="solid"
          label={restyles ? t("media.library.apply") : t("media.library.add")}
          tooltip={"blocked" in plan ? plan.blocked : plan.does}
          disabled={blocked}
          onClick={() => "request" in plan && void editor.addFromLibrary(plan.request)}
        />
      </span>
    </li>
  );
}

/**
 * FossCut's own library (ADR-0028) as one list: shapes, stickers, backgrounds, titles,
 * text and caption styles, looks, templates, sound effects and music, searched by kind
 * and words. Add puts an item in at the playhead; looks go on the selected clip, and
 * templates build a new timeline with the media chosen in the bin.
 */
export function Library({ editor, state }: { editor: Editor; state: EditorState }) {
  const [filter, setFilter] = useState<LibraryFilter>(EVERYTHING);
  const items = state.library ?? [];
  const per = counts(items);
  return (
    <section className="library">
      <input
        type="search"
        placeholder={t("media.library.search")}
        aria-label={t("media.library.search")}
        value={filter.search}
        onChange={(e) => setFilter({ ...filter, search: e.target.value })}
      />
      <div className="filters">
        <select
          aria-label={t("media.library.kind")}
          value={filter.kind}
          onChange={(e) => setFilter({ ...filter, kind: e.target.value as LibraryFilter["kind"] })}
        >
          <option value="">{t("media.library.everything", { count: items.length })}</option>
          {KINDS.map(({ kind }) => (
            <option key={kind} value={kind} disabled={per[kind] === 0}>
              {t("media.library.count", { kind: kindPlural(kind), count: per[kind] })}
            </option>
          ))}
        </select>
      </div>
      <p className="muted note">{t("media.library.note")}</p>
      <LibraryItems editor={editor} state={state} kinds={filter.kind ? [filter.kind] : KINDS.map((k) => k.kind)} search={filter.search} />
    </section>
  );
}
