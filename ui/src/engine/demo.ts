// A sample project for working on the interface in a plain browser (`pnpm dev`, outside
// the desktop app). Development builds only; edits are simulated and nothing is saved.

import { clip, entry, moment, track } from "../test/fixtures";
import { FakeEngine } from "./fake";
import type { TimelineView } from "./types";

export function demoEngine(): FakeEngine {
  const engine = new FakeEngine();
  engine.revision = 6;
  engine.opened = {
    bundle: "/demo/Launch.fosscut",
    info: {
      id: "project_demo",
      name: "Launch video (demo)",
      revision: 6,
      can_undo: true,
      can_redo: false,
      primary_timeline: "timeline_0000",
      timelines: [],
      assets: 3,
    },
  };
  engine.recent = ["/demo/Launch.fosscut"];
  engine.connected = [{ id: 1, name: "agent:claude-code", since_ms: Date.now() - 60_000 }];
  const timeline = (): TimelineView => ({
    timeline: "timeline_0000",
    name: "Timeline 1",
    revision: engine.revision,
    format: { width: 1920, height: 1080, rate: "30" },
    duration: moment(24),
    tracks: [
      track("track_0002", "video", 0, [
        clip("clip_0003", "video", 0, 8, { asset_name: "interview.mp4" }),
        clip("clip_0006", "video", 8, 15, { asset_name: "b-roll.mp4" }),
        clip("clip_0007", "video", 15, 24, { asset_name: "interview.mp4" }),
      ]),
      track("track_0004", "audio", 1, [clip("clip_0008", "audio", 0, 24, { asset_name: "music.m4a" })]),
      track("track_0005", "text", 2, [clip("clip_0009", "text", 0.5, 3.5, { text: "Meet FossCut" })]),
      track("track_0010", "caption", 3, [
        clip("clip_0011", "caption", 1, 2.2, { text: "Edit by talking" }),
        clip("clip_0012", "caption", 2.2, 3.6, { text: "to your agent" }),
      ]),
    ],
  });
  engine.answers["project.info"] = () => ({ ...engine.opened?.info, revision: engine.revision });
  engine.answers["timeline.read"] = timeline;
  engine.answers["history.list"] = {
    entries: [
      entry(6, "agent:claude-code", ["captions.add"]),
      entry(5, "agent:claude-code", ["text.add"]),
      entry(4, "human", ["clip.split"]),
      entry(3, "agent:claude-code", ["clip.add", "clip.add", "clip.add"]),
      entry(2, "agent:claude-code", ["media.add"]),
    ].map((e) => ({ ...e, at_ms: Date.now() - (7 - e.seq) * 45_000 })),
  };
  return engine;
}
