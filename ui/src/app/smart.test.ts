import { afterEach, describe, expect, it } from "vitest";

import { FakeEngine } from "../engine/fake";
import { standInListing } from "../engine/models";
import type { ApiError, EditRequest } from "../engine/types";
import { setLanguage } from "../i18n";
import { clip, entry, timeline, track } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import { BODY_NEEDS } from "./assist";
import { Editor } from "./editor";
import {
  analysisNeeds,
  analyzerName,
  beatsCut,
  beatsMarked,
  candidateRange,
  candidateReasons,
  candidateSeconds,
  clipShowing,
  missingAnalysis,
  reframed,
  shortsFound,
  verticalMade,
} from "./smart";

function engine(): FakeEngine {
  const fake = new FakeEngine();
  fake.revision = 4;
  fake.opened = {
    bundle: "/work/Demo.fosscut",
    info: { id: "project_1", name: "Demo", revision: 4, can_undo: false, can_redo: false, primary_timeline: "timeline_0000", timelines: [], assets: 1 },
  };
  fake.answers["project.info"] = () => ({ ...fake.opened?.info, revision: fake.revision });
  fake.answers["timeline.read"] = () => timeline(fake.revision);
  fake.answers["history.list"] = { entries: [entry(4, "human", ["clip.add"])] };
  fake.modelListing = standInListing(["onnxruntime", "mediapipe-pose-full", "birefnet-lite"]);
  fake.analysisReads = 2;
  return fake;
}

async function started(fake = engine()) {
  const editor = new Editor(fake);
  await editor.start();
  return { editor, fake };
}

async function analyzeAll(editor: Editor, times = 10) {
  for (let i = 0; i < times && editor.getState().assist?.busy; i++) await editor.pollAnalysis();
}

const missing = (analyzer: string, options: Record<string, unknown> = {}): ApiError => ({
  code: "fosscut.analysis.missing",
  message: `the ${analyzer} analysis is not cached`,
  details: { asset: "asset_0001", analyzer, options, effect: "reframe" },
});

describe("edits planned from analyses", () => {
  afterEach(() => setLanguage("en"));

  it("reads which analysis a refusal asks for, and what it needs to run", () => {
    expect(missingAnalysis(missing("pose"))).toEqual({ asset: "asset_0001", analyzer: "pose", options: null, model: null, needsModel: false });
    const transcript = missingAnalysis({ ...missing("transcript"), details: { asset: "a", analyzer: "transcript", options: {}, needs_model: true, model: "whisper-base.en" } });
    expect(analysisNeeds(transcript!), "the speech model the desktop names").toEqual(["whisper-base.en"]);
    const matte = missingAnalysis(missing("matte", { matte: { model: "person" } }));
    expect(matte?.options).toEqual({ model: "person" });
    expect(analysisNeeds(matte!)).toEqual(["onnxruntime", "mediapipe-pose-full"]);
    expect(analysisNeeds(missingAnalysis(missing("matte"))!), "a subject matte unless told otherwise").toEqual(["onnxruntime", "birefnet-lite"]);
    expect(analysisNeeds(missingAnalysis(missing("pose"))!)).toEqual(BODY_NEEDS);
    expect(analysisNeeds(missingAnalysis(missing("beats"))!)).toEqual([]);
    expect(missingAnalysis({ code: "fosscut.analysis.missing", message: "the transcript analysis is missing" })?.analyzer, "from the message when no details").toBe("transcript");
    expect(missingAnalysis({ code: "fosscut.clip.not_found", message: "no" })).toBeNull();
    expect(missingAnalysis(null)).toBeNull();
    const shown = { ...timeline(), tracks: [track("track_0002", "video", 0, [clip("clip_0003", "video", 0, 4, { asset: "asset_0001" })])] };
    expect(clipShowing(shown, "asset_0001")).toBe("clip_0003");
    expect(clipShowing(shown, "asset_9999")).toBeNull();
    expect(clipShowing(null, "asset_0001")).toBeNull();
    setLanguage("hu");
    expect(analyzerName("pose")).toBe("testtartás");
    expect(analyzerName("loudness"), "others as the engine names them").toBe("loudness");
  });

  it("says what each planned edit did", () => {
    const response = null;
    expect(reframed({ reframe: { clip: "c", frames: 200, subject_frames: 150, shots: [] }, response })).toBe("Reframed as one shot; the subject was found in 75% of the frames.");
    expect(verticalMade({ vertical: { source: "t", timeline: "t2", clips: 3, centred: [{ clip: "c", reason: "none" }] }, response })).toBe(
      "Made a vertical version; 3 clips follow their subject. One clip is centred: no subject was found in it.",
    );
    expect(beatsMarked({ beats: { timeline: "t", tempo_bpm: 119.6, beats_per_bar: 4, confidence: 0.9, markers: 24, existing: 0 }, response })).toBe("Added 24 beat markers, at 120 BPM.");
    const cut = (snapped: number, unsnapped: number) => ({
      cuts: {
        track: "t",
        music: "m",
        tempo_bpm: 120,
        beats: 10,
        tolerance: 1,
        snapped: Array.from({ length: snapped }, (_, i) => ({ before: `a${i}`, after: `b${i}`, from: 0, to: 0 })),
        unsnapped: Array.from({ length: unsnapped }, (_, i) => ({ before: null, after: `c${i}`, at: 0, reason: "far" })),
      },
      response,
    });
    expect(beatsCut(cut(2, 1))).toBe("Moved 2 cuts onto beats. One stayed where it was.");
    expect(beatsCut(cut(0, 3))).toBe("No cut was near enough to a beat to move.");
    expect(shortsFound({ clip: "c", asset: "a", measured: [], missing: [], sentences: 40, considered: 9, candidates: [] })).toBe("No moment of that length stands alone.");
    const candidate = {
      span: { start: { flicks: 5, seconds: "0" }, end: { flicks: 9, seconds: "0" } },
      text: "Hi",
      score: 0.5,
      seconds: 0,
      reasons: { words_per_second: 2.84, pause_share: 0.123, loudness_variation_lu: null, typical_words_per_second: 2.5 },
      timeline_start: 0,
      timeline_end: 12.34 * FLICKS_PER_SECOND,
    };
    expect(candidateSeconds(candidate), "from its span when the length is not given").toBe(12.3);
    expect(candidateRange(candidate), "made from the media's own time").toEqual([5, 9]);
    expect(candidateReasons(candidate), "the measurements people read, those measured only").toEqual(["2.8 words a second", "12% pauses"]);
  });

  it("reframes a clip, and offers to run the pose analysis it needs, then tries again", async () => {
    const { editor, fake } = await started();
    await editor.reframe("clip_0003", "person");
    expect(fake.callsOf("reframe.clip")).toEqual([[{ clip: "clip_0003", source: "person", shots: false, expected_revision: 4 }]]);
    expect(editor.getState().measurement?.error?.code).toBe("fosscut.analysis.missing");
    expect(editor.getState().missingAnalysis).toEqual({ kind: "reframe", analyzer: "pose" });
    await editor.runMissingAnalysis();
    expect(fake.callsOf("subject.analyze")).toEqual([[{ clip: "clip_0003", kind: "pose" }]]);
    expect(editor.getState().analysisFor, "the reframe panel shows the analysis running").toBe("reframe");
    await analyzeAll(editor);
    expect(fake.callsOf("reframe.clip"), "tried again once the analysis ran").toHaveLength(2);
    expect(editor.getState().measurement?.report).toBe("Reframed in 2 shots; the subject was found in 94% of the frames.");
    expect(editor.getState().missingAnalysis).toBeNull();
  });

  it("asks before installing the models a missing analysis needs", async () => {
    const fake = engine();
    fake.modelListing = standInListing([]);
    const { editor } = await started(fake);
    await editor.cameraTrack("clip_0003", "subject", 2);
    expect(editor.getState().missingAnalysis?.analyzer).toBe("matte");
    await editor.runMissingAnalysis();
    expect(editor.getState().install?.models.map((m) => m.name)).toEqual(["onnxruntime", "birefnet-lite"]);
    expect(fake.callsOf("subject.analyze"), "nothing runs before the person agrees").toEqual([]);
  });

  it("makes a vertical version and shows it", async () => {
    const { editor, fake } = await started();
    await editor.verticalVersion("person");
    expect(fake.callsOf("reframe.vertical")).toEqual([[{ timeline: "timeline_0000", source: "person", expected_revision: 4 }]]);
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toContainEqual({ command: "timeline.set_primary", args: { timeline: "timeline_0090" } });
    expect(editor.getState().measurement?.report).toMatch(/^Made a vertical version/);
  });

  it("marks beats and cuts to them once the beats are analyzed", async () => {
    const fake = engine();
    const { editor } = await started(fake);
    await editor.beatMarkers("clip_0008", 2);
    expect(editor.getState().missingAnalysis).toEqual({ kind: "beats", analyzer: "beats" });
    await editor.runMissingAnalysis();
    expect(fake.callsOf("subject.analyze"), "through the clip itself when no other shows the asset").toEqual([[{ clip: "clip_0008", kind: "beats" }]]);
    fake.analyzed.add("clip_0008:beats:");
    await editor.beatMarkers("clip_0008", 2);
    expect(editor.getState().measurement?.report).toBe("Added 24 beat markers, at 120 BPM.");
    const cuts = await editor.cutToBeats("clip_0008", "track_0002", 1);
    expect(fake.callsOf("beats.cut")).toEqual([["track_0002", "clip_0008", 1, fake.revision - 1]]);
    expect(cuts?.cuts.snapped).toHaveLength(2);
    expect(editor.getState().measurement?.report).toBe("Moved 2 cuts onto beats. One stayed where it was.");
  });

  it("finds moments for shorts after the transcript, and makes the chosen one", async () => {
    const fake = engine();
    fake.shortCandidates = [
      { span: { start: { flicks: 0, seconds: "0" }, end: { flicks: 30 * FLICKS_PER_SECOND, seconds: "30" } }, text: "A moment", score: 0.8, seconds: 30, timeline_start: 0, timeline_end: 30 * FLICKS_PER_SECOND },
    ];
    const { editor } = await started(fake);
    await editor.findShorts("clip_0003", { min_seconds: 20, max_seconds: 60, count: 3 });
    expect(editor.getState().missingAnalysis?.analyzer).toBe("transcript");
    expect(editor.getState().shortList).toBeNull();
    await editor.runMissingAnalysis();
    await analyzeAll(editor);
    expect(fake.callsOf("shorts.candidates")).toHaveLength(2);
    expect(editor.getState().shortList?.candidates.map((c) => c.text)).toEqual(["A moment"]);
    await editor.makeShort({ clip: "clip_0003", start: 0, end: 30 * FLICKS_PER_SECOND, follow: "person", captions: true, width: 1080, height: 1920 });
    expect(fake.callsOf("shorts.make")[0]?.[0]).toMatchObject({ clip: "clip_0003", follow: "person", expected_revision: fake.revision - 2 });
    expect(editor.getState().measurement?.report).toBe("Made Short 1, 30 s long, as a new timeline.");
  });

  it("applies voice presets as one edit", async () => {
    const { editor, fake } = await started();
    await editor.applyVoice("clip_0008", "robot", false);
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([{ command: "voice.apply", args: { clip: "clip_0008", preset: "robot", replace: false } }]);
  });
});
