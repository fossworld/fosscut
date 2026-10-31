import { useState } from "react";

import type { Editor, EditorState } from "../app/editor";
import { multicamAngles } from "../app/multicam";
import type { AssetView, PreviewSource } from "../engine/types";
import { type MessageKey, t } from "../i18n";
import { ALL, type MediaFilter, type SortKey, shown } from "./mediaFilter";
import { MulticamForm } from "./Multicam";

/** The kind filter's choices, with the keys of their names (looked up when shown). */
const KINDS: [MediaFilter["kind"], MessageKey][] = [
  ["", "media.filter.all"],
  ["video", "media.filter.video"],
  ["audio", "media.filter.audio"],
  ["image", "media.filter.image"],
  ["lut", "media.filter.lut"],
  ["font", "media.filter.font"],
];

const SORTS: SortKey[] = ["imported", "name", "duration", "kind"];

/** The mark of an asset the preview decodes from a copy: its label and its explanation. */
export function sourceMark(source: PreviewSource | undefined): { label: string; help: string } | null {
  if (!source || source === "original") return null;
  if (source === "remux") return { label: t("media.mark.remux"), help: t("media.mark.remux.help") };
  return {
    label: t("media.mark.proxy"),
    help: t(source.proxy === "quarter" ? "media.mark.proxy.quarter" : "media.mark.proxy.half"),
  };
}

function Row({
  editor,
  asset,
  chosen,
  onChoose,
  source,
}: {
  editor: Editor;
  asset: AssetView;
  chosen: boolean;
  onChoose: (on: boolean) => void;
  source?: PreviewSource;
}) {
  const mark = sourceMark(source);
  const [naming, setNaming] = useState<string | null>(null);
  const finish = () => {
    if (naming !== null && naming.trim() && naming !== asset.name) void editor.renameAsset(asset.id, naming);
    setNaming(null);
  };
  return (
    <li className={asset.missing ? "asset missing" : "asset"}>
      <input
        type="checkbox"
        checked={chosen}
        onChange={(e) => onChoose(e.target.checked)}
        aria-label={t("media.choose", { name: asset.name })}
      />
      {naming === null ? (
        <span className="name" title={t("media.rename.help")} onDoubleClick={() => setNaming(asset.name)}>
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
      {mark && (
        <span className="mark" title={mark.help}>
          {mark.label}
        </span>
      )}
      <span className="muted">
        {asset.missing
          ? t("media.missing")
          : (asset.duration?.timecode ?? (asset.families?.join(", ") || t(`media.kind.${asset.kind}`)))}
      </span>
      <span className="actions">
        {asset.missing ? (
          <button title={t("media.relink.help")} onClick={() => void editor.relinkAsset(asset.id)}>
            {t("media.relink")}
          </button>
        ) : (
          asset.kind !== "lut" &&
          asset.kind !== "font" && (
            <button title={t("media.add.help")} onClick={() => void editor.addToTimeline(asset.id)}>
              {t("media.add")}
            </button>
          )
        )}
        <button title={t("media.show.help")} disabled={asset.missing} onClick={() => void editor.revealAsset(asset.id)}>
          {t("media.show")}
        </button>
        <button
          title={asset.used_by.length > 0 ? t("media.remove.withClips") : t("media.remove.help")}
          onClick={() => void editor.removeAsset(asset.id)}
        >
          {t("media.remove")}
        </button>
      </span>
    </li>
  );
}

/** The media bin: search, filter by kind and folder, sort, add to the timeline, rename,
 * file in folders, relink missing files, and start a timeline (or a library template)
 * from chosen media. */
export function Media({ editor, state }: { editor: Editor; state: EditorState }) {
  const [filter, setFilter] = useState<MediaFilter>(ALL);
  const [newFolder, setNewFolder] = useState("");
  const assets = shown(state.media, filter);
  const ids = new Set(state.media.map((a) => a.id));
  const picked = state.chosen.filter((id) => ids.has(id));
  const update = (change: Partial<MediaFilter>) => setFilter({ ...filter, ...change });
  const compounds = (state.opened?.info.timelines ?? []).filter((timeline) => timeline.role === "compound");
  const angles = multicamAngles(state.media, picked);
  return (
    <section className="media">
      <input
        type="search"
        placeholder={t("media.search")}
        value={filter.search}
        onChange={(e) => update({ search: e.target.value })}
      />
      <div className="filters">
        <select value={filter.kind} onChange={(e) => update({ kind: e.target.value as MediaFilter["kind"] })}>
          {KINDS.map(([value, label]) => (
            <option key={value} value={value}>
              {t(label)}
            </option>
          ))}
        </select>
        <select
          value={filter.folder ?? "*"}
          onChange={(e) => update({ folder: e.target.value === "*" ? null : e.target.value })}
        >
          <option value="*">{t("media.folder.every")}</option>
          <option value="">{t("media.folder.top")}</option>
          {state.folders.map((f) => (
            <option key={f} value={f}>
              {f}
            </option>
          ))}
        </select>
        <select value={filter.sort} onChange={(e) => update({ sort: e.target.value as SortKey })}>
          {SORTS.map((value) => (
            <option key={value} value={value}>
              {t(`media.sort.${value}`)}
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
            onChoose={(on) => editor.chooseMedia(asset.id, on)}
            source={state.sources[asset.id]}
          />
        ))}
        {assets.length === 0 && (
          <li className="muted">{state.media.length === 0 ? t("media.empty") : t("media.nothing")}</li>
        )}
      </ul>
      {compounds.length > 0 && (
        <>
          <h3>{t("media.compounds")}</h3>
          <ul className="assets">
            {compounds.map((compound) => (
              <li key={compound.id} className="asset">
                <span className="name">{compound.name}</span>
                <span className="muted">{compound.duration.timecode}</span>
                <span className="actions">
                  <button title={t("media.add.help")} onClick={() => void editor.addCompound(compound.id)}>
                    {t("media.add")}
                  </button>
                  <button title={t("media.open.help")} onClick={() => void editor.showTimeline(compound.id)}>
                    {t("media.open")}
                  </button>
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
      <div className="buttons">
        <button disabled={picked.length === 0} onClick={() => void editor.addTimeline(picked)} title={t("media.newTimeline.help")}>
          {t("media.newTimeline")}
        </button>
        <select
          value=""
          disabled={picked.length === 0}
          onChange={(e) => {
            const target = e.target.value;
            if (target !== "") void editor.moveAssets(picked, target === "/" ? "" : target);
          }}
          aria-label={t("media.move.help")}
        >
          <option value="">{t("media.move")}</option>
          <option value="/">{t("media.folder.top")}</option>
          {state.folders.map((f) => (
            <option key={f} value={f}>
              {f}
            </option>
          ))}
        </select>
      </div>
      {angles.length >= 2 && <MulticamForm editor={editor} angles={angles} playhead={state.playhead} />}
      <div className="buttons">
        <input
          placeholder={t("media.folder.new")}
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
          <button title={t("media.folder.remove.help")} onClick={() => void editor.folder("remove", filter.folder ?? "")}>
            {t("media.folder.remove")}
          </button>
        )}
      </div>
    </section>
  );
}
