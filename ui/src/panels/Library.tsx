import { useEffect, useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import {
  EVERYTHING,
  KINDS,
  type LibraryFilter,
  addPlan,
  counts,
  itemDetail,
  kindName,
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
        placeholder="Search the library"
        aria-label="Search the library"
        value={filter.search}
        onChange={(e) => setFilter({ ...filter, search: e.target.value })}
      />
      <div className="filters">
        <select
          aria-label="Kind"
          value={filter.kind}
          onChange={(e) => setFilter({ ...filter, kind: e.target.value as LibraryFilter["kind"] })}
        >
          <option value="">Everything ({items.length})</option>
          {KINDS.map(({ kind, plural }) => (
            <option key={kind} value={kind} disabled={per[kind] === 0}>
              {plural} ({per[kind]})
            </option>
          ))}
        </select>
      </div>
      <p className="muted note">Made by FossCut: everything here is free to use, also commercially.</p>
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
                {restyles ? "Apply" : "Add"}
              </button>
            </li>
          );
        })}
        {state.library === null && <li className="muted">Reading the library.</li>}
        {state.library !== null && shown.length === 0 && <li className="muted">Nothing matches.</li>}
      </ul>
    </section>
  );
}
