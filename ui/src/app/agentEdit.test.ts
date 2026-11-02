// The AI edit flow (ADR-0043) through the editor against the stand-in engine: a video
// dropped or chosen on the start screen, the project made from it, the brief, agreeing to
// the command, the run and cancelling it, and the hand-off when no agent is installed.

import { describe, expect, it } from "vitest";

import { FakeEngine } from "../engine/fake";
import type { AgentChoice, Opened } from "../engine/types";
import { timeline } from "../test/fixtures";
import { Editor } from "./editor";

const VIDEO = "/Users/me/Downloads/test_raw.mp4";

const DEMO: Opened = {
  bundle: "/work/Demo.fosscut",
  info: { id: "project_1", name: "Demo", revision: 4, can_undo: true, can_redo: false, primary_timeline: "timeline_0000", timelines: [], assets: 1 },
};

/** An engine with no project open, whose project made from a video answers like Demo. */
function engine(): FakeEngine {
  const fake = new FakeEngine();
  fake.answers["project.info"] = () => fake.opened?.info;
  fake.answers["timeline.read"] = () => timeline(fake.revision);
  fake.answers["history.list"] = { entries: [] };
  fake.answers["media.list"] = { assets: [], folders: [] };
  fake.answers["capabilities"] = {};
  fake.agentTurns = 4;
  fake.agentStep = 2;
  return fake;
}

async function started(fake = engine()) {
  const editor = new Editor(fake, () => false);
  await editor.start();
  return { editor, fake };
}

/** Drops the video, makes the project and reads the agent: the brief step. */
async function atBrief(fake = engine()) {
  const { editor } = await started(fake);
  await editor.dropped([VIDEO]);
  await editor.createAiProject();
  return { editor, fake };
}

describe("an AI edit from a video", () => {
  it("starts when a video is dropped on the start screen, and only then", async () => {
    const { editor, fake } = await started();
    await editor.dropped(["/Users/me/notes.txt"]);
    expect(editor.getState().aiEdit.open).toBe(false);
    await editor.dropped(["/Users/me/notes.txt", VIDEO]);
    const dialog = editor.getState().aiEdit;
    expect(dialog).toMatchObject({ open: true, step: "video", video: VIDEO, aspect: "9:16" });
    expect(fake.callsOf("media.import"), "nothing is imported without a project").toEqual([]);

    // With a project open, a dropped video is imported as before.
    const other = engine();
    other.opened = DEMO;
    const { editor: open } = await started(other);
    await open.dropped([VIDEO]);
    expect(open.getState().aiEdit.open).toBe(false);
    expect(other.callsOf("media.import")).toEqual([[[VIDEO]]]);
  });

  it("starts from the card with a chosen video, and not when the choice is cancelled", async () => {
    const { editor, fake } = await started();
    await editor.pickAiVideo();
    expect(editor.getState().aiEdit.open).toBe(false);
    fake.videoPick = VIDEO;
    await editor.pickAiVideo();
    expect(editor.getState().aiEdit).toMatchObject({ open: true, step: "video", video: VIDEO });
    // Choosing another video keeps the dialog and its canvas.
    editor.setAiAspect("1:1");
    fake.videoPick = "/Users/me/Movies/take2.mov";
    await editor.pickAiVideo();
    expect(editor.getState().aiEdit).toMatchObject({ video: "/Users/me/Movies/take2.mov", aspect: "1:1" });
  });

  it("makes a vertical project from the video, then offers the agent's brief", async () => {
    const { editor, fake } = await atBrief();
    expect(fake.callsOf("agent.project")).toEqual([[VIDEO, 1080, 1920]]);
    const state = editor.getState();
    expect(state.opened?.bundle).toBe("/Users/me/Downloads/test_raw.fosscut");
    expect(state.aiEdit).toMatchObject({ open: true, step: "brief", busy: false, maxTurns: "200" });
    expect(state.aiEdit.setup?.agent?.name).toBe("Claude Code");
    expect(state.aiEdit.brief).toContain("talking-head-reel");
    expect(fake.callsOf("agent.start"), "nothing ran").toEqual([]);

    // The brief can be changed and put back.
    editor.editAiBrief({ brief: "Only add captions." });
    expect(editor.getState().aiEdit.brief).toBe("Only add captions.");
    editor.resetAiBrief();
    expect(editor.getState().aiEdit.brief).toBe(state.aiEdit.setup?.brief);
  });

  it("stays at the first step when saving the project is cancelled", async () => {
    const fake = engine();
    fake.videoProjectCancelled = true;
    const { editor } = await started(fake);
    await editor.dropped([VIDEO]);
    await editor.createAiProject();
    expect(editor.getState().opened).toBeNull();
    expect(editor.getState().aiEdit).toMatchObject({ open: true, step: "video", busy: false });
  });

  it("asks the user to agree to the command and notes, then runs and reports the agent", async () => {
    const { editor, fake } = await atBrief();
    editor.editAiBrief({ maxTurns: "40", budget: "3" });
    await editor.checkAgent();
    let dialog = editor.getState().aiEdit;
    expect(dialog.step).toBe("agree");
    expect(dialog.command.slice(0, 2)).toEqual(["/Users/demo/.local/bin/claude", "-p"]);
    expect(dialog.command).toContain("--max-budget-usd");
    expect(dialog.notes.at(-1)).toContain("40 turns");
    expect(fake.agentRuns.size, "nothing ran before the user agreed").toBe(0);

    await editor.confirmAgent();
    dialog = editor.getState().aiEdit;
    expect(dialog.step).toBe("run");
    expect(dialog.job?.state).toBe("running");
    const asked = fake.callsOf("agent.start").map(([choice]) => choice as AgentChoice);
    expect(asked.map((c) => c.confirmed)).toEqual([false, true]);
    expect(asked[1]).toMatchObject({ max_turns: 40, max_budget_usd: 3 });

    // A running agent's panel does not close.
    editor.closeAiEdit();
    expect(editor.getState().aiEdit.open).toBe(true);
    await editor.pollAgent();
    expect(editor.getState().aiEdit.job?.agent?.turns).toBe(2);
    await editor.pollAgent();
    const finished = editor.getState().aiEdit.job;
    expect(finished?.state).toBe("succeeded");
    expect(finished?.agent?.summary).toMatch(/pauses/);
    editor.closeAiEdit();
    expect(editor.getState().aiEdit.open).toBe(false);
  });

  it("says when a hidden run ends, and cancels a run", async () => {
    const { editor, fake } = await atBrief();
    await editor.checkAgent();
    await editor.confirmAgent();
    editor.collapseAiRun(true);
    await editor.cancelAgent();
    expect(editor.getState().aiEdit.job?.state).toBe("cancelled");
    expect(fake.callsOf("agent.cancel")).toHaveLength(1);
    editor.backToBrief();
    expect(editor.getState().aiEdit.step).toBe("brief");

    await editor.checkAgent();
    await editor.confirmAgent();
    editor.collapseAiRun(true);
    await editor.pollAgent();
    await editor.pollAgent();
    expect(editor.getState().notice?.kind).toBe("info");
  });

  it("does not ask for a run with an empty brief or a bad cap", async () => {
    const { editor, fake } = await atBrief();
    editor.editAiBrief({ brief: "   " });
    await editor.checkAgent();
    editor.editAiBrief({ brief: "Go.", maxTurns: "zero" });
    await editor.checkAgent();
    expect(fake.callsOf("agent.start")).toEqual([]);
    expect(editor.getState().aiEdit.step).toBe("brief");
  });

  it("hands off the brief and the setup line when no agent is installed", async () => {
    const fake = engine();
    fake.agentFound = null;
    const { editor } = await atBrief(fake);
    const dialog = editor.getState().aiEdit;
    expect(dialog.step).toBe("brief");
    expect(dialog.setup?.agent).toBeNull();
    expect(dialog.setup?.tried.length).toBeGreaterThan(0);
    expect(dialog.setup?.setup[0]).toMatch(/^claude mcp add fosscut -- /);
    expect(dialog.brief).not.toBe("");
    // Once installed, looking again finds it and keeps the brief as typed.
    editor.editAiBrief({ brief: "My own brief." });
    fake.agentFound = { path: "/opt/homebrew/bin/claude", name: "Claude Code", via: "path" };
    await editor.loadAgent();
    expect(editor.getState().aiEdit.setup?.agent?.path).toBe("/opt/homebrew/bin/claude");
    expect(editor.getState().aiEdit.brief).toBe("My own brief.");
  });

  it("opens again from the editor at the brief, or shows the agent that runs", async () => {
    const { editor, fake } = await atBrief();
    editor.closeAiEdit();
    expect(editor.getState().aiEdit.open).toBe(false);
    await editor.openAgent();
    expect(editor.getState().aiEdit).toMatchObject({ open: true, step: "brief", maxTurns: "200" });
    await editor.checkAgent();
    await editor.confirmAgent();
    editor.collapseAiRun(true);
    await editor.openAgent();
    expect(editor.getState().aiEdit).toMatchObject({ step: "run", collapsed: false });
    expect(fake.callsOf("agent.start")).toHaveLength(2);
  });

  it("shows why the agent refused to start", async () => {
    const fake = engine();
    const { editor } = await atBrief(fake);
    fake.agentRefusal = { code: "fosscut.agent.not_found", message: "no agent command line (Claude Code) was found on this computer" };
    await editor.checkAgent();
    expect(editor.getState().aiEdit).toMatchObject({ step: "brief", busy: false });
    expect(editor.getState().aiEdit.error?.code).toBe("fosscut.agent.not_found");
  });
});
