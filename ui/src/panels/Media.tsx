import {
  Box,
  ChevronDown,
  Clapperboard,
  Ellipsis,
  FileCode2,
  FileType,
  FolderInput,
  FolderOpen,
  Globe,
  Image,
  LayoutGrid,
  Link2,
  List,
  Mic,
  Music,
  Palette,
  Pencil,
  PenTool,
  Plus,
  Shapes,
  Trash2,
  TriangleAlert,
  Upload,
  Film,
  type LucideIcon,
  X,
} from "lucide-react";
import { useState, useSyncExternalStore } from "react";

import { ASSET_DRAG, dragAsset } from "../app/dragging";
import type { Editor, EditorState } from "../app/editor";
import { multicamAngles } from "../app/multicam";
import { recordedTime } from "../app/recorder";
import { Pictures, onGrid } from "../app/pictures";
import type { AssetView, PreviewSource } from "../engine/types";
import { type MessageKey, t } from "../i18n";
import { FLICKS_PER_SECOND } from "../transport/time";
import { Empty, Glyph, IconButton, Menu, Segmented, cx } from "../ui/controls";
import { ALL, type MediaFilter, type SortKey, shown } from "./mediaFilter";
import { MulticamForm } from "./Multicam";

/** The kind filter's choices, with the keys of their names (looked up when shown). */
const KINDS: [MediaFilter["kind"], MessageKey][] = [
  ["", "media.filter.all"],
  ["video", "media.filter.video"],
  ["audio", "media.filter.audio"],
  ["image", "media.filter.image"],
  ["vector", "media.filter.vector"],
  ["web", "media.filter.web"],
  ["model", "media.filter.model"],
  ["lut", "media.filter.lut"],
  ["font", "media.filter.font"],
];

const SORTS: SortKey[] = ["imported", "name", "duration", "kind"];

/** The icon of an asset's kind. */
export const ASSET_ICONS: Record<AssetView["kind"], LucideIcon> = {
  video: Film,
  audio: Music,
  image: Image,
  lut: Palette,
  font: FileType,
  vector: PenTool,
  web: Globe,
  model: Box,
};

/**
 * True when clips can show the asset: drawings as images, compositions as video (ADR-0036,
 * ADR-0039), 3D models as 3D objects (ADR-0040).
 */
export function placeable(asset: AssetView): boolean {
  return !asset.missing && asset.kind !== "lut" && asset.kind !== "font";
}

/** How many frames hovering over a video's tile runs through. */
const SKIM_FRAMES = 12;

/** The mark of an asset the preview decodes from a copy: its label and its explanation. */
export function sourceMark(source: PreviewSource | undefined): { label: string; help: string } | null {
  if (!source || source === "original") return null;
  if (source === "remux") return { label: t("media.mark.remux"), help: t("media.mark.remux.help") };
  return {
    label: t("media.mark.proxy"),
    help: t(source.proxy === "quarter" ? "media.mark.proxy.quarter" : "media.mark.proxy.half"),
  };
}

/** The frame hovering at `fraction` of a tile shows: one of a few, on the pictures' grid. */
export function skimTime(asset: Pick<AssetView, "duration">, fraction: number): number {
  const length = asset.duration?.flicks ?? 0;
  const index = Math.min(SKIM_FRAMES - 1, Math.max(0, Math.floor(fraction * SKIM_FRAMES)));
  return onGrid((((index + 0.5) / SKIM_FRAMES) * length) / FLICKS_PER_SECOND, 0.5);
}

/** What the tile says under its picture: the length, or what kind of file it is. */
function detail(asset: AssetView): string {
  if (asset.missing) return t("media.missing");
  return asset.duration?.timecode ?? (asset.families?.join(", ") || t(`media.kind.${asset.kind}`));
}

function Tile({
  editor,
  asset,
  chosen,
  source,
  list,
}: {
  editor: Editor;
  asset: AssetView;
  chosen: boolean;
  source?: PreviewSource;
  list: boolean;
}) {
  const mark = sourceMark(source);
  const [naming, setNaming] = useState<string | null>(null);
  const [skim, setSkim] = useState<number | null>(null);
  const finish = () => {
    if (naming !== null && naming.trim() && naming !== asset.name) void editor.renameAsset(asset.id, naming);
    setNaming(null);
  };
  const canPlace = placeable(asset);
  const picture = Pictures.pictured(asset)
    ? skim !== null && asset.kind === "video"
      ? (editor.pictures.frame(asset, skimTime(asset, skim)) ?? editor.pictures.poster(asset))
      : editor.pictures.poster(asset)
    : null;
  const info = [
    t(`media.kind.${asset.kind}`),
    asset.video ? `${asset.video.width} × ${asset.video.height}` : null,
    asset.duration?.timecode ?? null,
    asset.folder ? asset.folder : null,
    asset.used_by.length > 0 ? t("media.used", { count: asset.used_by.length }) : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <li
      className={cx("tile", list && "row", chosen && "chosen", asset.missing && "missing", `asset-${asset.kind}`)}
      draggable={canPlace}
      onDragStart={(event) => {
        event.dataTransfer.setData(ASSET_DRAG, asset.id);
        event.dataTransfer.effectAllowed = "copy";
        dragAsset(asset.id);
      }}
      onDragEnd={() => dragAsset(null)}
    >
      <div
        className="tile-thumb"
        title={info}
        onPointerMove={(event) => {
          if (asset.kind !== "video") return;
          const rect = event.currentTarget.getBoundingClientRect();
          setSkim((event.clientX - rect.left) / Math.max(1, rect.width));
        }}
        onPointerLeave={() => setSkim(null)}
        onDoubleClick={() => canPlace && void editor.addToTimeline(asset.id)}
      >
        {picture ? (
          <img src={picture} alt="" draggable={false} />
        ) : (
          <span className="tile-placeholder">
            <Glyph icon={asset.missing ? TriangleAlert : ASSET_ICONS[asset.kind]} size="lg" />
          </span>
        )}
        {skim !== null && asset.kind === "video" && <span className="skim-bar" style={{ left: `${skim * 100}%` }} aria-hidden="true" />}
        {!list && asset.duration && <span className="badge duration">{asset.duration.timecode}</span>}
        {mark && (
          <span className="badge mark" title={mark.help}>
            {mark.label}
          </span>
        )}
        <input
          type="checkbox"
          className="tile-check"
          checked={chosen}
          onChange={(e) => editor.chooseMedia(asset.id, e.target.checked)}
          aria-label={t("media.choose", { name: asset.name })}
          title={t("media.choose", { name: asset.name })}
        />
        <span className="tile-actions">
          {asset.missing ? (
            <IconButton icon={Link2} size="sm" variant="solid" label={t("media.relink")} tooltip={t("media.relink.help")} onClick={() => void editor.relinkAsset(asset.id)} />
          ) : (
            canPlace && (
              <IconButton icon={Plus} size="sm" variant="primary" label={t("media.add")} tooltip={t("media.add.help")} onClick={() => void editor.addToTimeline(asset.id)} />
            )
          )}
        </span>
      </div>
      <div className="tile-foot">
        {naming === null ? (
          <span className="tile-name" title={t("media.rename.help")} onDoubleClick={() => setNaming(asset.name)}>
            {asset.name}
          </span>
        ) : (
          <input
            className="tile-name"
            autoFocus
            aria-label={t("media.rename")}
            value={naming}
            onChange={(e) => setNaming(e.target.value)}
            onBlur={finish}
            onKeyDown={(e) => {
              if (e.key === "Enter") finish();
              if (e.key === "Escape") setNaming(null);
            }}
          />
        )}
        {list && <span className="tile-detail">{detail(asset)}</span>}
        <Menu
          label={t("media.more", { name: asset.name })}
          icon={Ellipsis}
          align="end"
          className="tile-menu"
          items={[
            ...(canPlace ? [{ id: "add", label: t("media.add.help"), icon: Plus, onSelect: () => void editor.addToTimeline(asset.id) }] : []),
            ...(asset.kind === "vector" && !asset.missing
              ? [{ id: "explode", label: t("vector.explode"), icon: Shapes, detail: t("vector.paths", { count: asset.vector?.paths ?? 0 }), onSelect: () => void editor.explodeDrawing(asset.id) }]
              : []),
            { id: "rename", label: t("media.rename"), icon: Pencil, onSelect: () => setNaming(asset.name) },
            { id: "show", label: t("media.show.help"), icon: FolderOpen, disabled: asset.missing, onSelect: () => void editor.revealAsset(asset.id) },
            ...(asset.missing ? [{ id: "relink", label: t("media.relink.help"), icon: Link2, onSelect: () => void editor.relinkAsset(asset.id) }] : []),
            {
              id: "remove",
              label: asset.used_by.length > 0 ? t("media.remove.withClips") : t("media.remove.help"),
              icon: Trash2,
              separated: true,
              onSelect: () => void editor.removeAsset(asset.id),
            },
          ]}
        />
      </div>
    </li>
  );
}

/**
 * The media bin as a grid of thumbnails (or a list): import and record at the top, search,
 * filter by kind and sort; hover over a video to run through it, drag a tile onto the
 * timeline or use its add button, double-click a name to rename it; choose media to start
 * a timeline, a template or a multicam clip from them, or to file them in a folder.
 */
export function Media({ editor, state, folder }: { editor: Editor; state: EditorState; folder: string | null }) {
  useSyncExternalStore(editor.pictures.subscribe, editor.pictures.version, editor.pictures.version);
  const [filter, setFilter] = useState<Omit<MediaFilter, "folder">>(ALL);
  const [view, setView] = useState<"grid" | "list">("grid");
  const assets = shown(state.media, { ...filter, folder });
  const ids = new Set(state.media.map((a) => a.id));
  const picked = state.chosen.filter((id) => ids.has(id));
  const update = (change: Partial<MediaFilter>) => setFilter({ ...filter, ...change });
  const compounds = (state.opened?.info.timelines ?? []).filter((timeline) => timeline.role === "compound");
  const angles = multicamAngles(state.media, picked);
  const recording = state.recorder.job?.state === "running";
  return (
    <section className="media">
      <div className="browser-actions">
        <span className="split-button">
          <button type="button" className="primary small" onClick={() => void editor.importMedia()} title={t("media.import.help")}>
            <Glyph icon={Upload} size="sm" />
            {t("media.import")}
          </button>
          <Menu
            label={t("media.import.more")}
            icon={ChevronDown}
            floating
            buttonClassName="primary small"
            items={[
              { id: "page", label: t("web.import.page"), icon: Globe, onSelect: () => void editor.pickComposition(false) },
              { id: "folder", label: t("web.import.folder"), icon: FolderOpen, onSelect: () => void editor.pickComposition(true) },
              { id: "remotion", label: t("web.remotion.open"), icon: FileCode2, separated: true, onSelect: () => editor.openRemotion() },
            ]}
          />
        </span>
        <button
          type="button"
          className={cx("small", recording && "recording")}
          aria-pressed={state.recorder.open}
          onClick={() => (state.recorder.open ? editor.closeRecorder() : void editor.openRecorder())}
          title={t("app.record.help")}
        >
          <Glyph icon={Mic} size="sm" />
          {recording && state.recorder.job ? t("app.recording", { time: recordedTime(state.recorder.job) }) : t("app.record")}
        </button>
        <span className="spacer" />
        <Segmented
          label={t("media.view")}
          value={view}
          onChange={setView}
          options={[
            { id: "grid", label: t("media.view.grid"), icon: LayoutGrid },
            { id: "list", label: t("media.view.list"), icon: List },
          ]}
        />
      </div>
      <div className="browser-filters">
        <input
          type="search"
          placeholder={t("media.search")}
          aria-label={t("media.search")}
          value={filter.search}
          onChange={(e) => update({ search: e.target.value })}
        />
      </div>
      <div className="browser-filters">
        <select aria-label={t("media.filter.kind")} value={filter.kind} onChange={(e) => update({ kind: e.target.value as MediaFilter["kind"] })}>
          {KINDS.map(([value, label]) => (
            <option key={value} value={value}>
              {t(label)}
            </option>
          ))}
        </select>
        <select aria-label={t("media.sort")} value={filter.sort} onChange={(e) => update({ sort: e.target.value as SortKey })}>
          {SORTS.map((value) => (
            <option key={value} value={value}>
              {t(`media.sort.${value}`)}
            </option>
          ))}
        </select>
      </div>
      {state.media.length === 0 ? (
        <Empty icon={Upload} title={t("media.empty")}>
          <p className="hint">{t("media.empty.help")}</p>
          <button type="button" className="primary" onClick={() => void editor.importMedia()}>
            {t("media.import")}
          </button>
        </Empty>
      ) : (
        <ul className={cx("tiles", view === "list" && "list")}>
          {assets.map((asset) => (
            <Tile
              key={asset.id}
              editor={editor}
              asset={asset}
              chosen={picked.includes(asset.id)}
              source={state.sources[asset.id]}
              list={view === "list"}
            />
          ))}
          {assets.length === 0 && <li className="hint none">{t("media.nothing")}</li>}
        </ul>
      )}
      {compounds.length > 0 && folder === null && (
        <>
          <h3 className="browser-heading">{t("media.compounds")}</h3>
          <ul className={cx("tiles", view === "list" && "list")}>
            {compounds.map((compound) => (
              <li key={compound.id} className={cx("tile", view === "list" && "row", "compound")}>
                <div className="tile-thumb" onDoubleClick={() => void editor.addCompound(compound.id)}>
                  <span className="tile-placeholder">
                    <Glyph icon={Clapperboard} size="lg" />
                  </span>
                  {view === "grid" && <span className="badge duration">{compound.duration.timecode}</span>}
                  <span className="tile-actions">
                    <IconButton icon={Plus} size="sm" variant="primary" label={t("media.add")} tooltip={t("media.add.help")} onClick={() => void editor.addCompound(compound.id)} />
                  </span>
                </div>
                <div className="tile-foot">
                  <span className="tile-name">{compound.name}</span>
                  {view === "list" && <span className="tile-detail">{compound.duration.timecode}</span>}
                  <IconButton icon={FolderOpen} size="sm" label={t("media.open")} tooltip={t("media.open.help")} onClick={() => void editor.showTimeline(compound.id)} />
                </div>
              </li>
            ))}
          </ul>
        </>
      )}
      {picked.length > 0 && (
        <div className="chosen-bar" role="region" aria-label={t("media.chosen", { count: picked.length })}>
          <span className="chosen-count">{t("media.chosen", { count: picked.length })}</span>
          <button type="button" className="small" onClick={() => void editor.addTimeline(picked)} title={t("media.newTimeline.help")}>
            {t("media.newTimeline")}
          </button>
          <select
            value=""
            onChange={(e) => {
              const target = e.target.value;
              if (target !== "") void editor.moveAssets(picked, target === "/" ? "" : target);
            }}
            aria-label={t("media.move.help")}
            title={t("media.move.help")}
          >
            <option value="">{t("media.move")}</option>
            <option value="/">{t("media.folder.top")}</option>
            {state.folders.map((f) => (
              <option key={f} value={f}>
                {f}
              </option>
            ))}
          </select>
          <span className="spacer" />
          <IconButton icon={X} size="sm" label={t("media.chosen.clear")} onClick={() => picked.forEach((id) => editor.chooseMedia(id, false))} />
          {angles.length >= 2 && <MulticamForm editor={editor} angles={angles} playhead={state.playhead} />}
        </div>
      )}
    </section>
  );
}

/** The bin's folders, for the browser's side list: every folder, the top level, each folder. */
export function MediaFolders({
  editor,
  state,
  folder,
  onFolder,
}: {
  editor: Editor;
  state: EditorState;
  folder: string | null;
  onFolder: (folder: string | null) => void;
}) {
  const [adding, setAdding] = useState<string | null>(null);
  const finish = () => {
    if (adding?.trim()) void editor.folder("add", adding.trim());
    setAdding(null);
  };
  const entry = (value: string | null, label: string, depth = 0) => (
    <button
      key={value ?? "*"}
      type="button"
      className={cx("side-item", folder === value && "active")}
      aria-current={folder === value ? "true" : undefined}
      style={depth > 0 ? { paddingLeft: 8 + depth * 10 } : undefined}
      onClick={() => onFolder(value)}
      title={label}
    >
      <span>{label}</span>
    </button>
  );
  return (
    <nav className="side-list" aria-label={t("media.folders")}>
      {entry(null, t("media.folder.every"))}
      {entry("", t("media.folder.top"))}
      {state.folders.map((f) => entry(f, f.split("/").pop() ?? f, f.split("/").length - 1))}
      {adding === null ? (
        <button type="button" className="side-item add" onClick={() => setAdding("")} title={t("media.folder.new")}>
          <Glyph icon={FolderInput} size="sm" />
          <span>{t("media.folder.add")}</span>
        </button>
      ) : (
        <input
          autoFocus
          className="side-input"
          placeholder={t("media.folder.new")}
          aria-label={t("media.folder.new")}
          value={adding}
          onChange={(e) => setAdding(e.target.value)}
          onBlur={finish}
          onKeyDown={(e) => {
            if (e.key === "Enter") finish();
            if (e.key === "Escape") setAdding(null);
          }}
        />
      )}
      {folder && (
        <button
          type="button"
          className="side-item danger"
          title={t("media.folder.remove.help")}
          onClick={() => {
            void editor.folder("remove", folder);
            onFolder(null);
          }}
        >
          <Glyph icon={Trash2} size="sm" />
          <span>{t("media.folder.remove")}</span>
        </button>
      )}
    </nav>
  );
}
