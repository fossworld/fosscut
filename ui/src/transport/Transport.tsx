import { Expand, Pause, Play, Ruler, ScanSearch, SkipBack, SkipForward, SquareDashed, StepBack, StepForward } from "lucide-react";

import { type Editor, type EditorState, preferencesOf } from "../app/editor";
import { aspectName } from "../app/layout";
import { formatNumber, t } from "../i18n";
import { ZOOMS } from "../preview/PreviewHole";
import { Glyph, IconButton, Menu } from "../ui/controls";
import { formatTime, parseRate } from "./time";

/**
 * The player's control bar: the playhead's time and the timeline's length (shown as the
 * preferences say), going to the start, a frame back, play and pause, a frame on and the
 * end, and on the right the preview's zoom, safe zones, rulers and guides, and full screen.
 */
export function Transport({
  editor,
  state,
  zoom,
  onZoom,
  safeZones,
  onSafeZones,
}: {
  editor: Editor;
  state: EditorState;
  zoom: number;
  onZoom: (zoom: number) => void;
  safeZones: boolean;
  onSafeZones: (on: boolean) => void;
}) {
  const timeline = state.timeline;
  const rate = parseRate(timeline?.format.rate ?? "30");
  const display = preferencesOf(state).timeline.time_display;
  const now = formatTime(state.playhead, rate, display);
  const length = formatTime(timeline?.duration.flicks ?? 0, rate, display);
  const format = timeline?.format;
  const zoomName = (z: number) => (z === 1 ? t("transport.zoom.fit") : t("transport.zoom.percent", { percent: z * 100 }));
  return (
    <div className="transport" role="toolbar" aria-label={t("transport.label")}>
      <div className="transport-time">
        <span className="timecode now">{now}</span>
        <span className="timecode total">/ {length}</span>
        {state.shuttle !== 0 && state.shuttle !== 1 && <span className="rate">{t("transport.rate", { rate: formatNumber(state.shuttle) })}</span>}
      </div>
      <div className="transport-buttons">
        <IconButton icon={SkipBack} label={t("transport.start")} action="player.start" className="secondary" onClick={() => void editor.seek(0)} disabled={!timeline} />
        <IconButton icon={StepBack} label={t("transport.previous")} action="player.frame_back" onClick={() => void editor.step(-1)} disabled={!timeline} />
        <IconButton
          icon={state.playing ? Pause : Play}
          label={state.playing ? t("transport.pause") : t("transport.play")}
          action="player.play"
          className="play"
          size="lg"
          variant="solid"
          onClick={() => void editor.togglePlay()}
          disabled={!timeline}
        />
        <IconButton icon={StepForward} label={t("transport.next")} action="player.frame_forward" onClick={() => void editor.step(1)} disabled={!timeline} />
        <IconButton icon={SkipForward} label={t("transport.end")} action="player.end" className="secondary" onClick={() => void editor.seekEnd()} disabled={!timeline} />
      </div>
      <div className="transport-view">
        {format && (
          <span className="chip quiet" title={t("transport.canvas.help", { width: format.width, height: format.height, rate: format.rate })}>
            {aspectName(format.width, format.height)}
          </span>
        )}
        <Menu
          label={t("transport.zoom")}
          align="end"
          buttonClassName="chip-button"
          items={ZOOMS.map((z) => ({ id: String(z), label: zoomName(z), checked: z === zoom, onSelect: () => onZoom(z) }))}
        >
          <Glyph icon={ScanSearch} size="sm" />
          <span>{zoomName(zoom)}</span>
        </Menu>
        <IconButton icon={SquareDashed} label={t("transport.safe")} tooltip={t("transport.safe.help")} className="tertiary" pressed={safeZones} onClick={() => onSafeZones(!safeZones)} />
        <IconButton icon={Ruler} label={t("app.rulers")} tooltip={t("app.rulers.help")} action="player.rulers" className="secondary" pressed={state.rulers} onClick={() => editor.toggleRulers()} />
        <IconButton icon={Expand} label={t("app.fullscreen")} tooltip={t("app.fullscreen.help")} action="player.fullscreen" pressed={state.fullscreen} onClick={() => void editor.toggleFullscreen()} />
      </div>
    </div>
  );
}
