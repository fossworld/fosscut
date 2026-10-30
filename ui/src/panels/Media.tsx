import { useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import type { AssetView } from "../engine/types";
import { ALL, type MediaFilter, type SortKey, shown } from "./mediaFilter";

const KINDS: [MediaFilter["kind"], string][] = [
  ["", "All kinds"],
  ["video", "Video"],
  ["audio", "Audio"],
  ["image", "Images"],
  ["lut", "LUTs"],
  ["font", "Fonts"],
];

const SORTS: [SortKey, string][] = [
  ["imported", "Imported"],
  ["name", "Name"],
  ["duration", "Length"],
  ["kind", "Kind"],
];

function Row({
  editor,
  asset,
  chosen,
  onChoose,
}: {
  editor: Editor;
  asset: AssetView;
  chosen: boolean;
  onChoose: (on: boolean) => void;
}) {
  const [naming, setNaming] = useState<string | null>(null);
  const finish = () => {
    if (naming !== null && naming.trim() && naming !== asset.name) void editor.renameAsset(asset.id, naming);
    setNaming(null);
  };
  return (
    <li className={asset.missing ? "asset missing" : "asset"}>
      <input type="checkbox" checked={chosen} onChange={(e) => onChoose(e.target.checked)} aria-label={`Choose ${asset.name}`} />
      {naming === null ? (
        <span className="name" title="Double-click to rename" onDoubleClick={() => setNaming(asset.name)}>
          {asset.name}
        </span>
      ) : (
        <input
          className="name"
          autoFocus
          value={naming}
          onChange={(e) => setNaming(e.target.value)}
          onBlur={finish}
          onKeyDown={(e) => {
            if (e.key === "Enter") finish();
            if (e.key === "Escape") setNaming(null);
          }}
        />
      )}
      <span className="muted">
        {asset.missing ? "missing" : (asset.duration?.timecode ?? (asset.families?.join(", ") || asset.kind))}
      </span>
      <span className="actions">
        {asset.missing ? (
          <button title="Find where the file is now" onClick={() => void editor.relinkAsset(asset.id)}>
            Relink
          </button>
        ) : (
          asset.kind !== "lut" &&
          asset.kind !== "font" && (
            <button title="Add at the playhead" onClick={() => void editor.addToTimeline(asset.id)}>
              Add
            </button>
          )
        )}
        <button title="Show the file" disabled={asset.missing} onClick={() => void editor.revealAsset(asset.id)}>
          Show
        </button>
        <button
          title={asset.used_by.length > 0 ? "Remove from the bin, with the clips that show it" : "Remove from the bin"}
          onClick={() => void editor.removeAsset(asset.id)}
        >
          Remove
        </button>
      </span>
    </li>
  );
}

/** The media bin: search, filter by kind and folder, sort, add to the timeline, rename,
 * file in folders, relink missing files, and start a timeline from chosen media. */
export function Media({ editor, state }: { editor: Editor; state: EditorState }) {
  const [filter, setFilter] = useState<MediaFilter>(ALL);
  const [chosen, setChosen] = useState<string[]>([]);
  const [newFolder, setNewFolder] = useState("");
  const assets = shown(state.media, filter);
  const ids = new Set(state.media.map((a) => a.id));
  const picked = chosen.filter((id) => ids.has(id));
  const update = (change: Partial<MediaFilter>) => setFilter({ ...filter, ...change });
  const compounds = (state.opened?.info.timelines ?? []).filter((t) => t.role === "compound");
  return (
    <section className="media">
      <h2>Media</h2>
      <input
        type="search"
        placeholder="Search media"
        value={filter.search}
        onChange={(e) => update({ search: e.target.value })}
      />
      <div className="filters">
        <select value={filter.kind} onChange={(e) => update({ kind: e.target.value as MediaFilter["kind"] })}>
          {KINDS.map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
        <select
          value={filter.folder ?? "*"}
          onChange={(e) => update({ folder: e.target.value === "*" ? null : e.target.value })}
        >
          <option value="*">Every folder</option>
          <option value="">Top level</option>
          {state.folders.map((f) => (
            <option key={f} value={f}>
              {f}
            </option>
          ))}
        </select>
        <select value={filter.sort} onChange={(e) => update({ sort: e.target.value as SortKey })}>
          {SORTS.map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
      </div>
      <ul className="assets">
        {assets.map((asset) => (
          <Row
            key={asset.id}
            editor={editor}
            asset={asset}
            chosen={picked.includes(asset.id)}
            onChoose={(on) => setChosen(on ? [...picked, asset.id] : picked.filter((id) => id !== asset.id))}
          />
        ))}
        {assets.length === 0 && <li className="muted">{state.media.length === 0 ? "No media yet." : "Nothing matches."}</li>}
      </ul>
      {compounds.length > 0 && (
        <>
          <h3>Compound clips</h3>
          <ul className="assets">
            {compounds.map((t) => (
              <li key={t.id} className="asset">
                <span className="name">{t.name}</span>
                <span className="muted">{t.duration.timecode}</span>
                <span className="actions">
                  <button title="Add at the playhead" onClick={() => void editor.addCompound(t.id)}>
                    Add
                  </button>
                  <button title="Edit the timeline inside it" onClick={() => void editor.showTimeline(t.id)}>
                    Open
                  </button>
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
      <div className="buttons">
        <button disabled={picked.length === 0} onClick={() => void editor.addTimeline(picked)} title="A new timeline with the chosen media, one after another">
          New timeline
        </button>
        <select
          value=""
          disabled={picked.length === 0}
          onChange={(e) => {
            const target = e.target.value;
            if (target !== "") void editor.moveAssets(picked, target === "/" ? "" : target);
          }}
          aria-label="Move the chosen media"
        >
          <option value="">Move to</option>
          <option value="/">Top level</option>
          {state.folders.map((f) => (
            <option key={f} value={f}>
              {f}
            </option>
          ))}
        </select>
      </div>
      <div className="buttons">
        <input
          placeholder="New folder, such as B-roll/Day 1"
          value={newFolder}
          onChange={(e) => setNewFolder(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && newFolder.trim()) {
              void editor.folder("add", newFolder.trim());
              setNewFolder("");
            }
          }}
        />
        {filter.folder && (
          <button title="Remove this folder; what is in it moves up" onClick={() => void editor.folder("remove", filter.folder ?? "")}>
            Remove folder
          </button>
        )}
      </div>
    </section>
  );
}
