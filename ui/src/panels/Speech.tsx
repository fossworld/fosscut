import { useEffect, useState } from "react";

import { defaultVoice, voiceGroups, voiceName } from "../app/assist";
import type { Editor, EditorState } from "../app/editor";
import { Problem } from "../app/Problem";
import type { VoiceInfo } from "../engine/types";
import { formatNumber, t } from "../i18n";
import { AssistState, assistFor } from "./Subject";

/** The voices to choose from, grouped by accent and gender. */
export function VoiceSelect({
  voices,
  value,
  disabled,
  onChange,
}: {
  voices: VoiceInfo[] | null;
  value: string;
  disabled: boolean;
  onChange: (voice: string) => void;
}) {
  const groups = voiceGroups(voices ?? []);
  return (
    <label className="choice">
      <span>{t("voice.voice")}</span>
      <select value={value} disabled={disabled || groups.length === 0} onChange={(event) => onChange(event.target.value)}>
        {groups.length === 0 && <option value="">{voices === null ? t("voice.voices.looking") : t("voice.voices.none")}</option>}
        {groups.map((group) => (
          <optgroup key={group.label} label={group.label}>
            {group.voices.map((v) => (
              <option key={v.name} value={v.name}>
                {v.installed ? voiceName(v.name) : t("voice.voice.missing", { name: voiceName(v.name) })}
              </option>
            ))}
          </optgroup>
        ))}
      </select>
    </label>
  );
}

/**
 * Text to speech (ADR-0033 item 5): written text spoken by a voice of the voice model on
 * this computer, previewed through the app's sound, then inserted at the playhead.
 */
export function Speech({ editor, state, embedded = false }: { editor: Editor; state: EditorState; embedded?: boolean }) {
  const { voices, error } = state.speech;
  // In the browser's Voice category the voices are read when it shows.
  useEffect(() => {
    if (embedded) void editor.loadVoices();
  }, [editor, embedded]);
  const [text, setText] = useState("");
  const [voice, setVoice] = useState("");
  const [speed, setSpeed] = useState(1);
  const chosen = voice || defaultVoice(voices ?? []);
  const busy = state.assist?.busy === true;
  const mine = assistFor(state.assist, ["speech", "preview"], null);
  const ready = text.trim().length > 0 && chosen !== "" && !busy;
  return (
    <section
      className={embedded ? "speech-panel embedded" : "record-panel speech-panel"}
      role={embedded ? undefined : "dialog"}
      aria-label={t("voice.title")}
    >
      <div className="record-head">
        <h2>{t("voice.title")}</h2>
        {!embedded && (
          <button className="link" onClick={() => editor.closeSpeech()}>
            {t("voice.close")}
          </button>
        )}
      </div>
      <label className="speech-text">
        <span>{t("voice.text")}</span>
        <textarea
          rows={4}
          value={text}
          placeholder={t("voice.text.placeholder")}
          aria-label={t("voice.text")}
          onChange={(event) => setText(event.target.value)}
        />
      </label>
      <VoiceSelect voices={voices} value={chosen} disabled={busy} onChange={setVoice} />
      <label className="param">
        <span />
        <span>{t("voice.speed")}</span>
        <span className="control">
          <input
            type="range"
            min={0.5}
            max={2}
            step={0.05}
            value={speed}
            disabled={busy}
            aria-label={t("voice.speed.label")}
            onChange={(event) => setSpeed(Number(event.target.value))}
          />
          <span className="value">{t("voice.speed.value", { speed: formatNumber(speed) })}</span>
        </span>
      </label>
      <p className="hint">{t("voice.english")}</p>
      <div className="buttons">
        <button disabled={!ready} title={t("voice.preview.help")} onClick={() => void editor.previewSpeech(text, chosen, speed)}>
          {mine?.busy && mine.kind === "preview" ? t("voice.busy") : t("voice.preview")}
        </button>
        <button
          className="primary"
          disabled={!ready}
          title={t("voice.insert.help")}
          onClick={() => void editor.insertSpeech(text, chosen, speed)}
        >
          {mine?.busy && mine.kind === "speech" ? t("voice.busy") : t("voice.insert")}
        </button>
      </div>
      {error && <Problem error={error} />}
      <AssistState editor={editor} assist={mine} busy={t("voice.busy")} />
    </section>
  );
}
