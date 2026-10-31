import { AudioWaveform, Crosshair, Flag, Focus, Scissors, Sparkles, Wand2 } from "lucide-react";
import { useState } from "react";

import type { Editor, EditorState, Measurement } from "../app/editor";
import { analyzerName, candidateRange, candidateReasons, candidateSeconds } from "../app/smart";
import type { ClipView, SubjectChoice } from "../engine/types";
import { formatNumber, t } from "../i18n";
import { rows } from "../timeline/geometry";
import { trackName } from "../timeline/Timeline";
import { formatTime, parseRate } from "../transport/time";
import { Glyph, Segmented, cx } from "../ui/controls";
import { Outcome } from "./Measured";
import { AssistState, usePollAnalysis } from "./Subject";

/**
 * What a planned edit of `kind` did or why it could not, and when an analysis is missing,
 * a button that runs it and tries again.
 */
export function SmartOutcome({ editor, state, kind, clip }: { editor: Editor; state: EditorState; kind: Measurement["kind"]; clip: string | null }) {
  const missing = state.missingAnalysis;
  const shown = state.measurement?.kind === kind && state.measurement.clip === clip;
  const analysis = state.assist?.kind === "analysis" && state.analysisFor === kind ? state.assist : null;
  usePollAnalysis(editor, analysis?.busy === true);
  return (
    <>
      <AssistState editor={editor} assist={analysis} />
      {shown && state.measurement?.busy && (
        <p className="hint busy-line" aria-live="polite">
          <Glyph icon={Sparkles} size="sm" className="spin" />
          {t("motion.planning")}
        </p>
      )}
      <Outcome editor={editor} measurement={state.measurement} kind={kind} clip={clip} />
      {shown && missing?.kind === kind && !state.measurement?.busy && (
        <div className="buttons">
          <button type="button" className="primary small" onClick={() => void editor.runMissingAnalysis()}>
            <Glyph icon={Wand2} size="sm" />
            {t("motion.analyze", { analyzer: analyzerName(missing.analyzer) })}
          </button>
        </div>
      )}
    </>
  );
}

/** Who a crop follows: a person or the subject. */
function SourceChoice({ value, onChange }: { value: SubjectChoice; onChange: (value: SubjectChoice) => void }) {
  return (
    <div className="choice">
      <span>{t("motion.follow")}</span>
      <Segmented
        label={t("motion.follow")}
        value={value}
        onChange={onChange}
        options={[
          { id: "person", label: t("motion.follow.person"), help: t("motion.follow.person.help") },
          { id: "subject", label: t("motion.follow.subject"), help: t("motion.follow.subject.help") },
        ]}
      />
    </div>
  );
}

/**
 * Auto reframe and camera tracking of a video clip: its crop follows a person or the
 * subject through the clip (shot by shot when asked), or it moves like a camera that
 * follows them, zoomed in.
 */
export function Reframe({ editor, state, clip }: { editor: Editor; state: EditorState; clip: ClipView }) {
  const [source, setSource] = useState<SubjectChoice>("person");
  const [shots, setShots] = useState(false);
  const [zoom, setZoom] = useState(1.5);
  const busy = state.measurement?.busy === true || state.assist?.busy === true;
  return (
    <>
      <section className="effects reframe">
        <h2>
          <span>{t("motion.reframe")}</span>
          <Glyph icon={Focus} size="sm" />
        </h2>
        <p className="hint">{t("motion.reframe.help")}</p>
        <SourceChoice value={source} onChange={setSource} />
        <label className="toggle" title={t("motion.reframe.shots.help")}>
          <input type="checkbox" checked={shots} onChange={(event) => setShots(event.target.checked)} />
          <span>{t("motion.reframe.shots")}</span>
        </label>
        <div className="buttons">
          <button type="button" disabled={busy} onClick={() => void editor.reframe(clip.id, source, shots)}>
            <Glyph icon={Focus} size="sm" />
            {t("motion.reframe.run")}
          </button>
        </div>
        <SmartOutcome editor={editor} state={state} kind="reframe" clip={clip.id} />
      </section>
      <section className="effects camera-track">
        <h2>
          <span>{t("motion.camera")}</span>
          <Glyph icon={Crosshair} size="sm" />
        </h2>
        <p className="hint">{t("motion.camera.help")}</p>
        <label className="param">
          <span>{t("motion.camera.zoom")}</span>
          <span className="control">
            <input
              type="range"
              min={1}
              max={4}
              step={0.1}
              value={zoom}
              aria-label={t("motion.camera.zoom")}
              onChange={(event) => setZoom(Number(event.target.value))}
            />
            <span className="value">{t("transport.rate", { rate: formatNumber(zoom) })}</span>
          </span>
        </label>
        <div className="buttons">
          <button type="button" disabled={busy} onClick={() => void editor.cameraTrack(clip.id, source, zoom)}>
            <Glyph icon={Crosshair} size="sm" />
            {t("motion.camera.run")}
          </button>
        </div>
        <SmartOutcome editor={editor} state={state} kind="camera" clip={clip.id} />
      </section>
    </>
  );
}

/** How many beats apart markers and cuts land: every beat, every other, every bar. */
const EVERY = [1, 2, 4] as const;

/**
 * Beat sync for a music clip: a marker on its beats, and the cuts of a video track moved
 * onto them, with what moved and what stayed put.
 */
export function Beats({ editor, state, clip }: { editor: Editor; state: EditorState; clip: ClipView }) {
  const timeline = state.timeline;
  const tracks = timeline ? rows(timeline).filter((track) => track.kind === "video" && track.clips.length > 1) : [];
  const [every, setEvery] = useState<number>(1);
  const [track, setTrack] = useState<string>("");
  const [cuts, setCuts] = useState<Awaited<ReturnType<Editor["cutToBeats"]>>>(null);
  const chosen = track || tracks[tracks.length - 1]?.id || "";
  const busy = state.measurement?.busy === true || state.assist?.busy === true;
  const rate = parseRate(timeline?.format.rate ?? "30");
  const time = (flicks: number) => formatTime(flicks, rate);
  return (
    <section className="effects beats">
      <h2>
        <span>{t("motion.beats")}</span>
        <Glyph icon={AudioWaveform} size="sm" />
      </h2>
      <p className="hint">{t("motion.beats.help")}</p>
      <label className="choice">
        <span>{t("motion.beats.every")}</span>
        <select value={every} onChange={(event) => setEvery(Number(event.target.value))}>
          {EVERY.map((n) => (
            <option key={n} value={n}>
              {t(`motion.beats.every.${n}`)}
            </option>
          ))}
        </select>
      </label>
      <div className="buttons">
        <button type="button" disabled={busy} onClick={() => void editor.beatMarkers(clip.id, every)}>
          <Glyph icon={Flag} size="sm" />
          {t("motion.beats.mark")}
        </button>
      </div>
      <SmartOutcome editor={editor} state={state} kind="beats" clip={clip.id} />
      <h3>{t("motion.cuts")}</h3>
      {tracks.length === 0 ? (
        <p className="hint">{t("motion.cuts.noTrack")}</p>
      ) : (
        <>
          <label className="choice">
            <span>{t("motion.cuts.track")}</span>
            <select value={chosen} onChange={(event) => setTrack(event.target.value)}>
              {tracks.map((tr) => (
                <option key={tr.id} value={tr.id}>
                  {trackName(tr, timeline?.tracks ?? [])}
                </option>
              ))}
            </select>
          </label>
          <div className="buttons">
            <button
              type="button"
              disabled={busy || !chosen}
              onClick={() => void editor.cutToBeats(clip.id, chosen, every).then((result) => setCuts(result))}
            >
              <Glyph icon={Scissors} size="sm" />
              {t("motion.cuts.run")}
            </button>
          </div>
          <SmartOutcome editor={editor} state={state} kind="cuts" clip={clip.id} />
          {cuts && (cuts.cuts.snapped.length > 0 || cuts.cuts.unsnapped.length > 0) && (
            <ul className="cut-list" aria-label={t("motion.cuts")}>
              {cuts.cuts.snapped.map((cut) => (
                <li key={`${cut.before}-${cut.after}`} className="snapped">
                  <span className="cut-time">{time(cut.to)}</span>
                  <span>{cut.from === cut.to ? t("motion.cuts.onBeat") : t("motion.cuts.moved", { from: time(cut.from) })}</span>
                </li>
              ))}
              {cuts.cuts.unsnapped.map((cut) => (
                <li key={`u-${cut.after}`} className="unsnapped">
                  <span className="cut-time">{time(cut.at)}</span>
                  <span>{cut.reason}</span>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}

/** What a short's vertical crop follows: a person, the subject, or nothing (centred). */
type Follow = SubjectChoice | "centre";

/**
 * Long video to shorts: the moments of a clip that would stand alone, best first, with
 * what they say and why they scored, and a short made of the one chosen, as a new
 * vertical timeline that follows its subject, with captions when asked.
 */
export function Shorts({ editor, state, clip }: { editor: Editor; state: EditorState; clip: ClipView }) {
  const [min, setMin] = useState(20);
  const [max, setMax] = useState(60);
  const [count, setCount] = useState(5);
  const [follow, setFollow] = useState<Follow>("person");
  const [captions, setCaptions] = useState(true);
  const list = state.shortList?.clip === clip.id ? state.shortList : null;
  const busy = state.measurement?.busy === true || state.assist?.busy === true;
  const number = (value: number, set: (n: number) => void, label: string, low: number, high: number) => (
    <label className="choice">
      <span>{label}</span>
      <input
        type="number"
        className="seconds"
        min={low}
        max={high}
        value={value}
        onChange={(event) => {
          const n = Number(event.target.value);
          if (Number.isFinite(n)) set(Math.min(high, Math.max(low, n)));
        }}
      />
    </label>
  );
  return (
    <>
      <section className="effects shorts">
        <h2>
          <span>{t("motion.shorts")}</span>
          <Glyph icon={Sparkles} size="sm" />
        </h2>
        <p className="hint">{t("motion.shorts.help")}</p>
        {number(min, setMin, t("motion.shorts.min"), 5, 180)}
        {number(max, setMax, t("motion.shorts.max"), 10, 180)}
        {number(count, setCount, t("motion.shorts.count"), 1, 20)}
        <div className="buttons">
          <button
            type="button"
            className="primary small"
            disabled={busy}
            onClick={() => void editor.findShorts(clip.id, { min_seconds: Math.min(min, max), max_seconds: Math.max(min, max), count })}
          >
            <Glyph icon={Sparkles} size="sm" />
            {t("motion.shorts.find")}
          </button>
        </div>
        <SmartOutcome editor={editor} state={state} kind="shorts" clip={clip.id} />
      </section>
      {list && list.candidates.length > 0 && (
        <section className="effects">
          <h2>{t("motion.shorts.moments")}</h2>
          <div className="choice">
            <span>{t("motion.follow")}</span>
            <Segmented
              label={t("motion.follow")}
              value={follow}
              onChange={setFollow}
              options={[
                { id: "person", label: t("motion.follow.person") },
                { id: "subject", label: t("motion.follow.subject") },
                { id: "centre", label: t("motion.follow.centre") },
              ]}
            />
          </div>
          <label className="toggle">
            <input type="checkbox" checked={captions} onChange={(event) => setCaptions(event.target.checked)} />
            <span>{t("motion.shorts.captions")}</span>
          </label>
          <ol className="short-list">
            {list.candidates.map((candidate, index) => (
              <li key={`${candidate.timeline_start}-${candidate.timeline_end}`} className="short-card">
                <div className="short-head">
                  <span className="short-rank">{index + 1}</span>
                  <span className="short-score" title={t("motion.shorts.score")}>
                    <span className={cx("meter")} style={{ width: `${Math.round(candidate.score * 100)}%` }} />
                  </span>
                  <span className="short-seconds">{t("transport.seconds", { seconds: formatNumber(candidateSeconds(candidate)) })}</span>
                </div>
                <p className="short-text">{candidate.text}</p>
                {candidateReasons(candidate).length > 0 && (
                  <p className="short-reasons">
                    {candidateReasons(candidate).map((reason) => (
                      <span key={reason} className="chip quiet">
                        {reason}
                      </span>
                    ))}
                  </p>
                )}
                <div className="buttons">
                  <button type="button" className="small" onClick={() => void editor.seek(candidate.timeline_start)}>
                    {t("motion.shorts.go")}
                  </button>
                  <button
                    type="button"
                    className="small primary"
                    disabled={busy}
                    onClick={() =>
                      void editor.makeShort({
                        clip: clip.id,
                        start: candidateRange(candidate)[0],
                        end: candidateRange(candidate)[1],
                        name: t("motion.short.name", { number: index + 1 }),
                        width: 1080,
                        height: 1920,
                        follow: follow === "centre" ? null : follow,
                        captions,
                      })
                    }
                  >
                    {t("motion.shorts.make")}
                  </button>
                </div>
              </li>
            ))}
          </ol>
          <SmartOutcome editor={editor} state={state} kind="short" clip={clip.id} />
        </section>
      )}
    </>
  );
}

/** Voice filter presets for a clip's sound (`voice.apply`), replacing its voice filters or added to them. */
export function VoicePresets({ editor, state, clip }: { editor: Editor; state: EditorState; clip: ClipView }) {
  const presets = state.catalog?.voice_presets ?? [];
  const [replace, setReplace] = useState(true);
  if (presets.length === 0) return null;
  return (
    <section className="effects voice-presets">
      <h2>{t("effects.voice.title")}</h2>
      <label className="choice">
        <span>{t("effects.voice.preset")}</span>
        <select value="" aria-label={t("effects.voice.preset")} onChange={(event) => event.target.value && void editor.applyVoice(clip.id, event.target.value, replace)}>
          <option value="">{t("effects.voice.choose")}</option>
          {presets.map((preset) => (
            <option key={preset.name} value={preset.name} title={preset.description}>
              {preset.title}
            </option>
          ))}
        </select>
      </label>
      <label className="toggle" title={t("effects.voice.replace.help")}>
        <input type="checkbox" checked={replace} onChange={(event) => setReplace(event.target.checked)} />
        <span>{t("effects.voice.replace")}</span>
      </label>
    </section>
  );
}
