import { describe, expect, it } from "vitest";

import { FakeEngine } from "../engine/fake";
import type { CaptureDevice, JobStatus } from "../engine/types";
import { entry, moment, timeline } from "../test/fixtures";
import { Editor } from "./editor";
import { DEVICE_UNAVAILABLE, defaultForm, devicesOf, recordChoice, recordProblem, recordedTime } from "./recorder";

const DEVICES: CaptureDevice[] = [
  { kind: "camera", name: "FaceTime HD Camera", format: "avfoundation", input: "0:" },
  { kind: "screen", name: "Capture screen 0", format: "avfoundation", input: "2:" },
  { kind: "microphone", name: "Studio Mic", format: "avfoundation", input: ":0" },
  { kind: "microphone", name: "Built-in", format: "avfoundation", input: ":1" },
];

const at = (done: number): JobStatus => ({ job: "job_0001", kind: "recording", state: "running", progress: { done, total: 0 }, elapsed_ms: 0 });

function engine(): FakeEngine {
  const fake = new FakeEngine();
  fake.revision = 4;
  fake.opened = {
    bundle: "/work/Demo.fosscut",
    info: { id: "project_1", name: "Demo", revision: 4, can_undo: false, can_redo: false, primary_timeline: "timeline_0000", timelines: [], assets: 0 },
  };
  fake.answers["project.info"] = () => fake.opened?.info;
  fake.answers["timeline.read"] = () => timeline(fake.revision);
  fake.answers["history.list"] = { entries: [entry(4, "human", ["clip.add"])] };
  fake.answers["media.list"] = { assets: [] };
  fake.captureDevices = DEVICES;
  return fake;
}

async function started(fake = engine()) {
  const editor = new Editor(fake);
  await editor.start();
  return { editor, fake };
}

describe("recording", () => {
  it("defaults to the first device, a camera to the first microphone and a screen to no sound", () => {
    expect(devicesOf(DEVICES, "microphone").map((d) => d.name)).toEqual(["Studio Mic", "Built-in"]);
    expect(defaultForm("microphone", DEVICES)).toEqual({ what: "microphone", device: "Studio Mic", microphone: "" });
    expect(defaultForm("camera", DEVICES)).toEqual({ what: "camera", device: "FaceTime HD Camera", microphone: "Studio Mic" });
    expect(defaultForm("screen", DEVICES)).toEqual({ what: "screen", device: "Capture screen 0", microphone: "none" });
    expect(defaultForm("camera", DEVICES.slice(0, 2)), "a camera without microphones records no sound").toEqual({
      what: "camera",
      device: "FaceTime HD Camera",
      microphone: "none",
    });
    expect(defaultForm("microphone", [])).toEqual({ what: "microphone", device: "", microphone: "" });
  });

  it("asks for the device and microphone chosen", () => {
    expect(recordChoice({ what: "microphone", device: "Built-in", microphone: "" })).toEqual({ what: "microphone", device: "Built-in" });
    expect(recordChoice({ what: "microphone", device: "", microphone: "Studio Mic" }), "a microphone has no second microphone").toEqual({
      what: "microphone",
    });
    expect(recordChoice({ what: "screen", device: "Capture screen 0", microphone: "none" })).toEqual({
      what: "screen",
      device: "Capture screen 0",
      microphone: "none",
    });
  });

  it("shows the time recorded in minutes and seconds", () => {
    expect([recordedTime(null), recordedTime(at(0)), recordedTime(at(9)), recordedTime(at(75)), recordedTime(at(3_725))]).toEqual([
      "0:00",
      "0:00",
      "0:09",
      "1:15",
      "1:02:05",
    ]);
  });

  it("points to the privacy settings when a device cannot be opened", () => {
    expect(recordProblem({ code: "fosscut.job.not_found", message: "there is no job job_0009" })).toBe("there is no job job_0009.");
    const refused = recordProblem({ code: DEVICE_UNAVAILABLE, message: "cannot open the camera", hint: "Check that it is connected" });
    expect(refused).toContain("cannot open the camera. Check that it is connected.");
    expect(refused).toContain("System Settings, under Privacy and Security");
    expect(recordProblem({ code: "x", message: "stopped.", hint: "Try again." }), "no doubled stops").toBe("stopped. Try again.");
  });

  it("reads the devices when the panel opens, and records from the playhead until stopped", async () => {
    const { editor, fake } = await started();
    await editor.openRecorder();
    expect(editor.getState().recorder).toMatchObject({ open: true, devices: DEVICES, job: null, error: null });
    await editor.seek(moment(2).flicks);
    expect(await editor.startRecording({ what: "microphone", device: "Studio Mic" })).toBe(true);
    expect(fake.callsOf("record.start")).toEqual([[{ what: "microphone", device: "Studio Mic", at: moment(2).flicks }]]);
    expect(await editor.startRecording({ what: "camera" }), "one recording at a time").toBe(false);
    fake.recordStep = 0.5;
    for (let i = 0; i < 5; i += 1) await editor.pollRecording();
    expect(recordedTime(editor.getState().recorder.job)).toBe("0:02");
    expect(editor.getState().recorder.job?.state).toBe("running");
  });

  it("finishes once on stop: reads the project again and selects the clip the recording became", async () => {
    const { editor, fake } = await started();
    await editor.openRecorder();
    await editor.seek(moment(3).flicks);
    await editor.startRecording({ what: "camera", device: "FaceTime HD Camera", microphone: "Studio Mic" });
    await editor.pollRecording();
    const reads = () => fake.callsOf("query").filter(([name]) => name === "timeline.read").length;
    const before = reads();
    await editor.stopRecording();
    const job = editor.getState().recorder.job;
    expect(job).toMatchObject({ state: "succeeded", asset: "asset_0062", clip: "clip_0063", recording: { what: "camera", stopped: true } });
    expect(reads(), "the project is read again once").toBe(before + 1);
    expect(editor.getState().selection).toBe("clip_0063");
    expect(editor.getState().media.map((a) => a.id), "the bin shows the recording").toEqual(["asset_0062"]);
    await editor.pollRecording();
    await editor.stopRecording();
    expect(fake.callsOf("record.status")).toHaveLength(1);
    expect(fake.callsOf("record.stop"), "nothing to stop or finish again").toHaveLength(1);
    expect(reads()).toBe(before + 1);
  });

  it("shows refusals, failures and files that could not be imported in the panel", async () => {
    const fake = engine();
    fake.recordRefusal = { code: DEVICE_UNAVAILABLE, message: "cannot open the microphone" };
    const { editor } = await started(fake);
    await editor.openRecorder();
    expect(await editor.startRecording({ what: "microphone" })).toBe(false);
    expect(editor.getState().recorder.error?.code).toBe(DEVICE_UNAVAILABLE);
    fake.recordRefusal = null;
    await editor.startRecording({ what: "microphone" });
    expect(editor.getState().recorder.error, "a new start clears it").toBeNull();
    const original = fake.record.stop;
    fake.record.stop = async (job) => ({
      ...(await original(job)),
      import_error: { code: "fosscut.request.invalid_arguments", message: "it was recorded for a project that is no longer open" },
    });
    await editor.stopRecording();
    expect(editor.getState().recorder.error?.message).toContain("no longer open");

    fake.record.devices = async () => {
      throw { code: "fosscut.media.capture_failed", message: "no capture devices" };
    };
    editor.closeRecorder();
    await editor.openRecorder();
    expect(editor.getState().recorder).toMatchObject({ open: true, error: { message: "no capture devices" } });
  });
});
