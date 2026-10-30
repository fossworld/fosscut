import { describe as group, expect, it } from "vitest";

import { entry } from "../test/fixtures";
import { clock, describe, isAgent, runs, who } from "./runs";

group("activity", () => {
  it("names actors as people read them", () => {
    expect(who("human")).toBe("You");
    expect(who("cli")).toBe("Command line");
    expect(who("agent:claude-code")).toBe("claude-code");
    expect(who("agent:")).toBe("Agent");
    expect(who("")).toBe("Unknown");
    expect(isAgent("agent:codex") && !isAgent("human")).toBe(true);
  });

  it("describes changes by their commands", () => {
    expect(describe(entry(1, "human", ["text.add"]))).toBe("Added a title");
    expect(describe(entry(2, "agent:x", ["clip.add", "text.add"]))).toBe("Added a clip, Added a title");
    expect(describe(entry(3, "agent:x", ["clip.split", "clip.split", "clip.split"]))).toBe("Split a clip (3)");
    expect(describe(entry(4, "agent:x", ["effect.new"]))).toBe("effect.new");
  });

  it("groups consecutive changes by the same actor", () => {
    const grouped = runs([
      entry(5, "agent:claude-code", ["history.undo"], "undo"),
      entry(4, "agent:claude-code", ["clip.split"]),
      entry(3, "human", ["text.add"]),
      entry(2, "agent:claude-code", ["media.add"]),
    ]);
    expect(grouped.map((r) => [r.who, r.agent, r.entries.map((e) => e.seq)])).toEqual([
      ["claude-code", true, [5, 4]],
      ["You", false, [3]],
      ["claude-code", true, [2]],
    ]);
    expect(runs([])).toEqual([]);
  });

  it("shows times only when the log has them", () => {
    expect(clock(undefined)).toBe("");
    expect(clock(0)).toMatch(/\d/);
  });
});
