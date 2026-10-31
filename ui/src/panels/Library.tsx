import { useEffect, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import { t } from "../i18n";
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

/**
 * FossCut's own library (ADR-0028): shapes, stickers, backgrounds, titles, text and
 * caption styles, looks, templates, sound effects and music, searched by kind and words.
 * Add puts an item in at the playhead; looks go on the selected clip, and templates build
 * a new timeline with the media chosen in the bin.
 */
export function Library({ editor, state }: { editor: Editor; state: EditorState }) {
  const [filter, setFilter] = useState<LibraryFilter>(EVERYTHING);
  useEffect(() => {
    void editor.loadLibrary();
  }, [editor]);
  const items = state.library ?? [];
  const shown = shownItems(items, filter);
  const per = counts(items);
  const at = placingOf(state.timeline, state.selection, state.playhead, state.media, state.chosen);
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
      <ul className="items">
        {shown.map((item) => {
          const plan = addPlan(item, at);
          const detail = itemDetail(item);
          const restyles = item.kind === "look" || item.kind === "caption_style";
          return (
            <li key={`${item.kind}:${item.name}`} className="item">
              <div className="about">
                <span className="title">
                  {titleOf(item)}
                  {filter.kind === "" && <span className="kind"> {kindName(item.kind)}</span>}
                </span>
                <span className="description">{item.description}</span>
                {detail && <span className="detail">{detail}</span>}
              </div>
              <button
                disabled={"blocked" in plan || !state.timeline}
                title={"blocked" in plan ? plan.blocked : plan.does}
                onClick={() => "request" in plan && void editor.addFromLibrary(plan.request)}
              >
                {restyles ? t("media.library.apply") : t("media.library.add")}
              </button>
            </li>
          );
        })}
        {state.library === null && <li className="muted">{t("media.library.reading")}</li>}
        {state.library !== null && shown.length === 0 && <li className="muted">{t("media.nothing")}</li>}
      </ul>
    </section>
  );
}
