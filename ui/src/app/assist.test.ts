import { afterEach, describe, expect, it } from "vitest";

import { FakeEngine } from "../engine/fake";
import { STAND_IN_VOICES, standInListing } from "../engine/models";
import type { ApiError, EditRequest, TimelineView } from "../engine/types";
import { setLanguage, t } from "../i18n";
import { clip, entry, moment, timeline, track } from "../test/fixtures";
import {
  BODY_NEEDS,
  backgroundNeeds,
  defaultVoice,
  dubbedReport,
  megabytes,
  missingModels,
  modelsFor,
  translateNeeds,
  translatedReport,
  voiceGroups,
  voiceName,
} from "./assist";
import { Editor } from "./editor";

/** A video clip with its asset, the fixture's title, and a caption track of two pages. */
function view(revision: number, effects: TimelineView["tracks"][number]["clips"][number]["effects"] = []): TimelineView {
  const base = timeline(revision);
  return {
    ...base,
    tracks: [
      track("track_0002", "video", 0, [clip("clip_0003", "video", 0, 6, { asset: "asset_0001", asset_name: "talk.mp4", effects })]),
      ...base.tracks.slice(1),
      track("track_0010", "caption", 2, [
        clip("clip_0011", "caption", 0, 1.5, { text: "Good morning" }),
        clip("clip_0012", "caption", 1.5, 3, { text: "Thank you" }),
      ]),
    ],
  };
}

function engine(installed: string[] = []): FakeEngine {
  const fake = new FakeEngine();
  fake.revision = 4;
  fake.opened = {
    bundle: "/work/Demo.fosscut",
    info: { id: "project_1", name: "Demo", revision: 4, can_undo: false, can_redo: false, primary_timeline: "timeline_0000", timelines: [], assets: 1 },
  };
  let effects: TimelineView["tracks"][number]["clips"][number]["effects"] = [];
  fake.answers["project.info"] = () => fake.opened?.info;
  fake.answers["timeline.read"] = () => view(fake.revision, effects);
  fake.answers["history.list"] = { entries: [entry(4, "human", ["clip.add"])] };
  fake.answers["media.list"] = { assets: [] };
  fake.modelListing = standInListing(installed);
  fake.voices = STAND_IN_VOICES;
  fake.captionPages = [
    { clip: "clip_0011", text: "Good morning" },
    { clip: "clip_0012", text: "Thank you" },
  ];
  fake.translations = { "Good morning": "Jó reggelt", "Thank you": "Köszönöm" };
  // Adding the background effect shows it on the clip, as the engine would.
  const execute = fake.onExecute;
  fake.onExecute = (request: EditRequest) => {
    for (const call of request.commands) {
      const args = call.args as { type?: string; params?: Record<string, unknown> };
      if (call.command === "effect.add" && args.type === "key.subject") {
        effects = [{ id: "effect_0040", type: "key.subject", params: args.params }];
      }
    }
    return execute(request);
  };
  return fake;
}

async function started(fake = engine()) {
  const editor = new Editor(fake);
  await editor.start();
  return { editor, fake };
}

/** The `execute` requests' commands, in order. */
function edits(fake: FakeEngine): EditRequest[] {
  return fake.callsOf("execute").map(([request]) => request as EditRequest);
}

/** Reads the install until the dialog is done with it, at most `times` times. */
async function installAll(editor: Editor, times = 10) {
  for (let i = 0; i < times && editor.getState().install; i++) await editor.pollInstall();
}

/** Reads the analysis until it finished, at most `times` times. */
async function analyzeAll(editor: Editor, times = 10) {
  for (let i = 0; i < times && editor.getState().assist?.busy; i++) await editor.pollAnalysis();
}

const ALL = ["onnxruntime", "birefnet-lite", "mediapipe-pose-full", "kokoro-82m", "opus-mt-en-hu", "opus-mt-hu-en"];

describe("removing a background", () => {
  it("shows what to install when the models are missing, and installs nothing unless the person agrees", async () => {
    const { editor, fake } = await started();
    await editor.removeBackground("clip_0003");
    const prompt = editor.getState().install;
    expect(prompt?.purpose).toBe("background");
    expect(prompt?.models.map((m) => [m.name, m.license, m.size_bytes > 0, Boolean(m.attribution)])).toEqual([
      ["onnxruntime", "MIT", true, true],
      ["mediapipe-pose-full", "Apache-2.0", true, true],
    ]);
    expect(prompt?.installing).toBe(false);
    expect(fake.callsOf("models.install"), "nothing downloads before the person agrees").toEqual([]);
    expect(fake.callsOf("subject.analyze")).toEqual([]);
    editor.cancelInstall();
    expect(editor.getState().install).toBeNull();
    expect(fake.callsOf("models.install")).toEqual([]);
    expect(edits(fake)).toEqual([]);
  });

  it("installs the models one after the other once agreed, then analyzes and adds the effect", async () => {
    const { editor, fake } = await started();
    await editor.removeBackground("clip_0003");
    await editor.confirmInstall();
    expect(fake.callsOf("models.install")).toEqual([["onnxruntime", true]]);
    expect(editor.getState().install?.installing).toBe(true);
    expect(editor.getState().install?.current).toMatchObject({ name: "onnxruntime", state: "running", done_bytes: 0 });
    await editor.pollInstall();
    const halfway = editor.getState().install?.current;
    expect(halfway?.done_bytes, "progress in bytes").toBe(Math.round((halfway?.total_bytes ?? 0) / 2));
    editor.cancelInstall();
    expect(editor.getState().install, "an install that runs cannot be put away").not.toBeNull();
    await installAll(editor);
    expect(fake.callsOf("models.install")).toEqual([
      ["onnxruntime", true],
      ["mediapipe-pose-full", true],
    ]);
    expect(editor.getState().install).toBeNull();
    expect(editor.getState().models?.runtime).not.toBeNull();
    // Then the analysis runs as a job, with progress, and the effect follows it.
    expect(fake.callsOf("subject.analyze")).toEqual([[{ clip: "clip_0003", kind: "matte", model: "person" }]]);
    const assist = editor.getState().assist;
    expect(assist).toMatchObject({ kind: "background", clip: "clip_0003", busy: true, error: null });
    expect(assist?.job?.state).toBe("running");
    expect(edits(fake), "nothing is added before the matte is there").toEqual([]);
    await editor.pollAnalysis();
    expect(editor.getState().assist?.job?.progress).toEqual({ done: 60, total: 120 });
    await analyzeAll(editor);
    expect(edits(fake)).toEqual([
      {
        commands: [{ command: "effect.add", args: { clip: "clip_0003", type: "key.subject", params: { source: "person" } } }],
        expected_revision: 4,
        reason: t("models.background.reason"),
      },
    ]);
    expect(editor.getState().assist).toMatchObject({ busy: false, report: t("models.background.done"), error: null });
    // A second poll of the finished job adds nothing more.
    await editor.pollAnalysis();
    expect(edits(fake)).toHaveLength(1);
    editor.dismissAssist();
    expect(editor.getState().assist).toBeNull();
  });

  it("adds the effect at once when the models are there and the matte is cached", async () => {
    const fake = engine(ALL);
    fake.analyzed.add("clip_0003:matte:person");
    const { editor } = await started(fake);
    await editor.removeBackground("clip_0003");
    expect(editor.getState().install).toBeNull();
    expect(edits(fake).map((r) => r.commands[0]?.command)).toEqual(["effect.add"]);
    expect(editor.getState().assist?.report).toBe(t("models.background.done"));
  });

  it("points the clip's effect at another matte, analyzing any subject first", async () => {
    const fake = engine(ALL);
    fake.analyzed.add("clip_0003:matte:person");
    const { editor } = await started(fake);
    await editor.removeBackground("clip_0003");
    await editor.removeBackground("clip_0003", "subject");
    expect(fake.callsOf("subject.analyze").at(-1)).toEqual([{ clip: "clip_0003", kind: "matte", model: "subject" }]);
    await analyzeAll(editor);
    expect(edits(fake).at(-1)?.commands).toEqual([
      { command: "effect.set", args: { clip: "clip_0003", effect: "effect_0040", params: { source: "subject" } } },
    ]);
    // The same matte again changes nothing.
    await editor.removeBackground("clip_0003", "person");
    await analyzeAll(editor);
    const before = edits(fake).length;
    fake.analyzed.add("clip_0003:matte:subject");
    await editor.removeBackground("clip_0003", "subject");
    expect(edits(fake).length, "already keyed with that matte").toBe(before + 1);
  });

  it("asks for the models the engine says are missing, then tries again", async () => {
    const fake = engine(["onnxruntime"]);
    const { editor } = await started(fake);
    // The listing cannot be read at first, so the engine's refusal says what is missing.
    const list = fake.models.list;
    let unreadable = true;
    fake.models.list = async () => {
      if (unreadable) {
        unreadable = false;
        throw { code: "fosscut.internal", message: "busy" } satisfies ApiError;
      }
      return list();
    };
    await editor.addBodyEffect("clip_0003", "body.glow");
    expect(fake.callsOf("subject.analyze")).toEqual([[{ clip: "clip_0003", kind: "pose" }]]);
    expect(editor.getState().assist).toBeNull();
    expect(editor.getState().install).toMatchObject({ purpose: "body" });
    expect(editor.getState().install?.models.map((m) => m.name)).toEqual(["mediapipe-pose-full"]);
    await editor.confirmInstall();
    await installAll(editor);
    expect(fake.callsOf("subject.analyze"), "tried again once installed").toHaveLength(2);
    await analyzeAll(editor);
    expect(edits(fake).at(-1)?.commands).toEqual([{ command: "effect.add", args: { clip: "clip_0003", type: "body.glow" } }]);
  });

  it("shows the engine's refusal as it is when the model it names is installed", async () => {
    const fake = engine(ALL);
    const { editor } = await started(fake);
    fake.subject.analyze = async () => {
      throw { code: "fosscut.model.not_installed", message: "mediapipe-pose-full is not installed", details: { what: "mediapipe-pose-full" } } satisfies ApiError;
    };
    await editor.removeBackground("clip_0003");
    expect(editor.getState().install, "an installed model is not offered again").toBeNull();
    expect(editor.getState().assist?.error?.code).toBe("fosscut.model.not_installed");
    expect(edits(fake)).toEqual([]);
  });

  it("stops an analysis without adding anything, and shows why one failed", async () => {
    const { editor, fake } = await started(engine(ALL));
    await editor.removeBackground("clip_0003", "subject");
    await editor.cancelAnalysis();
    expect(fake.callsOf("subject.cancel")).toHaveLength(1);
    expect(editor.getState().assist).toMatchObject({ busy: false, report: t("models.analysis.cancelled") });
    expect(edits(fake)).toEqual([]);
    fake.analysisFailure = { code: "fosscut.model.inference_failed", message: "the model failed: out of memory" };
    await editor.removeBackground("clip_0003", "subject");
    await analyzeAll(editor);
    expect(editor.getState().assist?.error?.message).toBe("the model failed: out of memory");
    expect(edits(fake)).toEqual([]);
  });

  it("shows a failed install with a retry that resumes", async () => {
    const fake = engine(["onnxruntime"]);
    const { editor } = await started(fake);
    await editor.removeBackground("clip_0003", "subject");
    fake.installFailure = { code: "fosscut.model.download_failed", message: "cannot install birefnet-lite: timed out" };
    await editor.confirmInstall();
    await installAll(editor, 3);
    expect(editor.getState().install).toMatchObject({ installing: false, error: { code: "fosscut.model.download_failed" } });
    fake.installFailure = null;
    fake.installs.clear();
    await editor.confirmInstall();
    await installAll(editor);
    expect(editor.getState().install).toBeNull();
    expect(fake.callsOf("subject.analyze")).toEqual([[{ clip: "clip_0003", kind: "matte", model: "subject" }]]);
  });

  it("runs one piece of work at a time", async () => {
    const { editor, fake } = await started(engine(ALL));
    await editor.removeBackground("clip_0003");
    await editor.addBodyEffect("clip_0003", "body.glow");
    expect(editor.getState().notice?.text).toBe(t("app.assist.busy"));
    expect(fake.callsOf("subject.analyze")).toHaveLength(1);
  });
});

describe("body effects", () => {
  it("analyze the pose before the effect is added, and other effects add at once", async () => {
    const { editor, fake } = await started(engine(ALL));
    await editor.addEffect("clip_0003", "color.adjust");
    expect(edits(fake).map((r) => r.commands)).toEqual([[{ command: "effect.add", args: { clip: "clip_0003", type: "color.adjust" } }]]);
    await editor.addEffect("clip_0003", "body.sparkle");
    expect(fake.callsOf("subject.analyze")).toEqual([[{ clip: "clip_0003", kind: "pose" }]]);
    expect(edits(fake)).toHaveLength(1);
    await analyzeAll(editor);
    expect(edits(fake).at(-1)?.commands).toEqual([{ command: "effect.add", args: { clip: "clip_0003", type: "body.sparkle" } }]);
    expect(editor.getState().assist?.report).toBe(t("models.body.done", { effect: "body.sparkle" }));
    // Adding the background effect from the browser removes the background.
    fake.analyzed.add("clip_0003:matte:person");
    await editor.addEffect("clip_0003", "key.subject");
    expect(fake.callsOf("subject.analyze").at(-1)).toEqual([{ clip: "clip_0003", kind: "matte", model: "person" }]);
  });

  it("offer the pose model first when it is missing", async () => {
    const { editor, fake } = await started(engine(["onnxruntime"]));
    await editor.addEffect("clip_0003", "body.outline");
    expect(editor.getState().install).toMatchObject({ purpose: "body" });
    expect(editor.getState().install?.models.map((m) => m.name)).toEqual(["mediapipe-pose-full"]);
    expect(fake.callsOf("subject.analyze")).toEqual([]);
  });
});

describe("text to speech", () => {
  it("lists the voices and inserts speech at the playhead against the revision on screen", async () => {
    const { editor, fake } = await started(engine(ALL));
    await editor.openSpeech();
    expect(editor.getState().speech).toMatchObject({ open: true, error: null });
    expect(editor.getState().speech.voices?.map((v) => v.name)).toContain("af_heart");
    await editor.seek(moment(2).flicks);
    await editor.insertSpeech("Welcome to the show.", "am_adam", 1.25);
    expect(fake.callsOf("voice.add")).toEqual([
      [{ text: "Welcome to the show.", voice: "am_adam", speed: 1.25, at: moment(2).flicks, expected_revision: 4 }],
    ]);
    const state = editor.getState();
    expect(state.selection, "the clip it became is selected").toBe("clip_0079");
    expect(state.assist?.report).toBe(t("voice.speech.added", { seconds: 1.1 }));
    expect(state.timeline?.revision).toBe(5);
    await editor.previewSpeech("Welcome to the show.", "bf_emma", 0.9);
    expect(fake.callsOf("voice.preview")).toEqual([["Welcome to the show.", "bf_emma", 0.9]]);
    expect(editor.getState().assist).toMatchObject({ kind: "preview", busy: false, report: null, error: null });
    await editor.insertSpeech("   ", "am_adam", 1);
    expect(fake.callsOf("voice.add"), "nothing to say asks nothing").toHaveLength(1);
    editor.closeSpeech();
    expect(editor.getState().speech.open).toBe(false);
  });

  it("asks to install the voice model first, then inserts", async () => {
    const { editor, fake } = await started(engine(["onnxruntime"]));
    await editor.insertSpeech("Hello.", "af_heart", 1);
    expect(editor.getState().install?.models.map((m) => m.name)).toEqual(["kokoro-82m"]);
    expect(fake.callsOf("voice.add")).toEqual([]);
    await editor.confirmInstall();
    await installAll(editor);
    expect(fake.callsOf("voice.add")).toEqual([[{ text: "Hello.", voice: "af_heart", speed: 1, at: 0, expected_revision: 4 }]]);
  });

  it("reloads on a stale revision instead of inserting", async () => {
    const fake = engine(ALL);
    const { editor } = await started(fake);
    fake.revision = 6;
    await editor.insertSpeech("Hello.", "af_heart", 1);
    expect(editor.getState().notice?.text).toBe(t("app.conflict"));
    expect(editor.getState().timeline?.revision).toBe(6);
    expect(editor.getState().assist).toBeNull();
  });

  it("keeps the record and speech panels apart", async () => {
    const { editor } = await started(engine(ALL));
    await editor.openRecorder();
    await editor.openSpeech();
    expect([editor.getState().recorder.open, editor.getState().speech.open]).toEqual([false, true]);
    await editor.openRecorder();
    expect([editor.getState().recorder.open, editor.getState().speech.open]).toEqual([true, false]);
  });
});

describe("caption translation and dubbing", () => {
  it("translates the selected clip's caption track into Hungarian as one step", async () => {
    const { editor, fake } = await started(engine(ALL));
    await editor.translateCaptions("clip_0012", "hu", "bilingual");
    expect(fake.callsOf("captions.translate")).toEqual([[{ track: "track_0010", to: "hu", mode: "bilingual", expected_revision: 4 }]]);
    expect(editor.getState().assist).toMatchObject({
      kind: "translate",
      clip: "clip_0012",
      busy: false,
      report: t("voice.translated.bilingual.hu", { count: 2 }),
    });
    expect(editor.getState().timeline?.revision, "read again after the engine applied it").toBe(5);
  });

  it("asks for the Hungarian to English model before translating into English", async () => {
    const { editor, fake } = await started(engine(["onnxruntime", "opus-mt-en-hu"]));
    await editor.translateCaptions("clip_0011", "en", "replace");
    expect(editor.getState().install).toMatchObject({ purpose: "translate" });
    expect(editor.getState().install?.models.map((m) => m.name)).toEqual(["opus-mt-hu-en"]);
    expect(fake.callsOf("captions.translate")).toEqual([]);
  });

  it("dubs the track with the voice chosen, speaking its translations", async () => {
    const { editor, fake } = await started(engine(ALL));
    await editor.translateCaptions("clip_0011", "hu", "bilingual");
    await editor.dubCaptions("clip_0011", "bf_emma", 1.1, true);
    expect(fake.callsOf("captions.dub")).toEqual([[{ track: "track_0010", voice: "bf_emma", speed: 1.1, translation: true, expected_revision: 5 }]]);
    expect(editor.getState().assist?.report).toBe(t("voice.dubbed", { count: 2, voice: "Emma" }));
    await editor.dubCaptions("clip_0011", "am_adam", 1, false);
    expect(fake.callsOf("captions.dub").at(-1)).toEqual([{ track: "track_0010", voice: "am_adam", speed: 1, translation: false, expected_revision: 6 }]);
  });

  it("shows a refusal with its message", async () => {
    const fake = engine(ALL);
    fake.captionPages = [];
    const { editor } = await started(fake);
    await editor.translateCaptions("clip_0011", "hu", "bilingual");
    expect(editor.getState().assist?.error?.code).toBe("fosscut.captions.none");
    expect(editor.getState().install).toBeNull();
  });
});

describe("local model helpers", () => {
  afterEach(() => setLanguage("en"));

  it("know what each feature needs and what of it is missing, the runtime first", () => {
    expect(backgroundNeeds("person")).toEqual(["onnxruntime", "mediapipe-pose-full"]);
    expect(backgroundNeeds("subject")).toEqual(["onnxruntime", "birefnet-lite"]);
    expect(BODY_NEEDS).toEqual(["onnxruntime", "mediapipe-pose-full"]);
    expect(translateNeeds("hu")).toEqual(["onnxruntime", "opus-mt-en-hu"]);
    expect(translateNeeds("en")).toEqual(["onnxruntime", "opus-mt-hu-en"]);
    const listing = standInListing(["mediapipe-pose-full"]);
    expect(missingModels(listing, backgroundNeeds("person")).map((m) => m.name)).toEqual(["onnxruntime"]);
    expect(missingModels({ ...listing, runtime: "/opt/libonnxruntime.dylib" }, backgroundNeeds("person")), "a runtime found elsewhere counts").toEqual([]);
  });

  it("read which models the engine's refusal names", () => {
    const listing = standInListing([]);
    const names = (error: ApiError, needs: string[] = []) => modelsFor(error, listing, needs).map((m) => m.name);
    expect(names({ code: "fosscut.model.not_installed", message: "", details: { what: "birefnet-lite and onnxruntime" } })).toEqual([
      "onnxruntime",
      "birefnet-lite",
    ]);
    expect(names({ code: "fosscut.model.not_installed", message: "", details: { what: "kokoro-82m/voices/af_heart.bin" } })).toEqual(["kokoro-82m"]);
    expect(names({ code: "fosscut.model.runtime_missing", message: "" })).toEqual(["onnxruntime"]);
    expect(names({ code: "fosscut.model.not_installed", message: "", details: { what: "a multilingual speech model" } }, ["onnxruntime", "kokoro-82m"]), "the feature's needs when it names none").toEqual([
      "onnxruntime",
      "kokoro-82m",
    ]);
    expect(names({ code: "fosscut.analysis.missing", message: "" }, ["onnxruntime"])).toEqual([]);
    expect(modelsFor({ code: "fosscut.model.not_installed", message: "", details: { what: "kokoro-82m" } }, standInListing(["kokoro-82m"]), []), "nothing installed is offered again").toEqual([]);
  });

  it("group voices by accent and gender, and name them", () => {
    expect(voiceGroups(STAND_IN_VOICES).map((g) => [g.label, g.voices.map((v) => voiceName(v.name))])).toEqual([
      ["American English, female", ["Heart", "Bella"]],
      ["American English, male", ["Adam", "Michael"]],
      ["British English, female", ["Emma"]],
      ["British English, male", ["George"]],
    ]);
    expect(voiceGroups([])).toEqual([]);
    expect(defaultVoice([{ name: "am_adam", accent: "american", gender: "male", installed: false }, { name: "bf_emma", accent: "british", gender: "female", installed: true }])).toBe("bf_emma");
    expect(defaultVoice([])).toBe("");
    setLanguage("hu");
    expect(voiceGroups(STAND_IN_VOICES)[0]?.label).toBe("Amerikai angol, női");
  });

  it("write sizes and what translating and dubbing did", () => {
    expect(megabytes(224_005_088)).toBe("224");
    expect(megabytes(24_691_287)).toBe("24.7");
    const translated = {
      revision: 5,
      timeline: "timeline_0000",
      track: "track_0010",
      pair: "en>hu",
      mode: "bilingual" as const,
      model: "opus-mt-en-hu",
      pages: [{ clip: "clip_0011", text: "Good morning", translation: "Jó reggelt" }],
      skipped: ["clip_0012"],
    };
    expect(translatedReport({ translation: translated, response: null })).toBe(
      t("voice.translated.skipped", { done: t("voice.translated.bilingual.hu", { count: 1 }), count: 1 }),
    );
    expect(translatedReport({ translation: { ...translated, pages: [], skipped: [] }, response: null })).toBe(t("voice.translated.none"));
    expect(translatedReport({ translation: { ...translated, pair: "hu>en", mode: "replace", skipped: [] }, response: null })).toBe(
      "Translated 1 page into English, instead of its words.",
    );
    const page = { clip: "clip_0011", start: 0, text: "Hi", speech: 1, speed: 1, asset: "asset_0080" };
    const dub = { revision: 6, timeline: "timeline_0000", track: "track_0010", voice: "bm_george", pages: [page, { ...page, warning: "runs on" }], skipped: ["clip_0013"] };
    expect(dubbedReport({ dub, response: null })).toBe("Dubbed 2 pages with George. 1 runs past the next page. 1 page had nothing to say.");
    expect(dubbedReport({ dub: { ...dub, pages: [] }, response: null })).toBe(t("voice.dubbed.none"));
    setLanguage("hu");
    expect(megabytes(24_691_287)).toBe("24,7");
    expect(dubbedReport({ dub: { ...dub, skipped: [] }, response: null })).toBe("2 oldal szinkronizálva, hang: George. 1 túlnyúlik a következő oldalon.");
  });
});
