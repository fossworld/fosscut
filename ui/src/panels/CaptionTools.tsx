import { useEffect, useState } from "react";

import { defaultVoice } from "../app/assist";
import type { Editor } from "../app/editor";
import type { ClipView } from "../engine/types";
import { formatNumber, t } from "../i18n";
import { VoiceSelect } from "./Speech";
import { AssistState, assistFor } from "./Subject";

/**
 * Translating and dubbing the caption track of the selected caption clip on this computer
 * (ADR-0033 item 6): into Hungarian or English, under each page or instead of its words;
 * and a voice speaking each page, its words or its translation, at the page's start.
 */
export function CaptionTools({ editor, clip }: { editor: Editor; clip: ClipView }) {
  const { assist, speech } = editor.getState();
  const [to, setTo] = useState<"hu" | "en">("hu");
  const [mode, setMode] = useState<"bilingual" | "replace">("bilingual");
  const [voice, setVoice] = useState("");
  const [speed, setSpeed] = useState(1);
  const [translation, setTranslation] = useState(false);
  const busy = assist?.busy === true;
  const mine = assistFor(assist, ["translate", "dub"], clip.id);
  const voices = speech.voices;
  const chosen = voice || defaultVoice(voices ?? []);

  useEffect(() => {
    void editor.loadVoices();
  }, [editor]);

  return (
    <section className="effects captions-tools">
      <h2>{t("voice.captions.title")}</h2>
      <label className="choice">
        <span>{t("voice.translate.to")}</span>
        <select value={to} disabled={busy} onChange={(event) => setTo(event.target.value as "hu" | "en")}>
          <option value="hu">{t("voice.translate.hu")}</option>
          <option value="en">{t("voice.translate.en")}</option>
        </select>
      </label>
      <label className="choice">
        <span>{t("voice.translate.mode")}</span>
        <select value={mode} disabled={busy} onChange={(event) => setMode(event.target.value as "bilingual" | "replace")}>
          <option value="bilingual">{t("voice.translate.bilingual")}</option>
          <option value="replace">{t("voice.translate.replace")}</option>
        </select>
      </label>
      <div className="buttons">
        <button disabled={busy} title={t("voice.translate.help")} onClick={() => void editor.translateCaptions(clip.id, to, mode)}>
          {mine?.busy && mine.kind === "translate" ? t("voice.translate.busy") : t("voice.translate")}
        </button>
      </div>

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
      <label className="choice">
        <span>{t("voice.dub.text")}</span>
        <select
          value={translation ? "translation" : "words"}
          disabled={busy}
          onChange={(event) => setTranslation(event.target.value === "translation")}
        >
          <option value="words">{t("voice.dub.words")}</option>
          <option value="translation">{t("voice.dub.translation")}</option>
        </select>
      </label>
      <p className="hint">{t("voice.dub.english")}</p>
      <div className="buttons">
        <button
          disabled={busy || !chosen}
          title={t("voice.dub.help")}
          onClick={() => void editor.dubCaptions(clip.id, chosen, speed, translation)}
        >
          {mine?.busy && mine.kind === "dub" ? t("voice.dub.busy") : t("voice.dub")}
        </button>
      </div>
      <AssistState
        editor={editor}
        assist={mine}
        busy={mine?.kind === "dub" ? t("voice.dub.busy") : t("voice.translate.busy")}
      />
    </section>
  );
}
