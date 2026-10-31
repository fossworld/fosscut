import { afterEach, describe as group, expect, it } from "vitest";

import { setLanguage, t } from "../i18n";
import { entry } from "../test/fixtures";
import { clock, describe, isAgent, runs, who } from "./runs";

group("activity", () => {
  afterEach(() => setLanguage("en"));

  it("names actors as people read them", () => {
    expect(who("human")).toBe(t("activity.who.you"));
    expect(who("cli")).toBe(t("activity.who.cli"));
    expect(who("agent:claude-code")).toBe("claude-code");
    expect(who("agent:")).toBe(t("activity.who.agent"));
    expect(who("")).toBe(t("activity.who.unknown"));
    expect(who("human")).toBe("You");
    setLanguage("hu");
    expect([who("human"), who("cli")]).toEqual(["Ön", "Parancssor"]);
    expect(isAgent("agent:codex") && !isAgent("human")).toBe(true);
  });

  it("describes changes by their commands", () => {
    expect(describe(entry(1, "human", ["text.add"]))).toBe(t("activity.verb.text_add"));
    expect(describe(entry(2, "agent:x", ["clip.add", "text.add"]))).toBe(
      `${t("activity.verb.clip_add")}, ${t("activity.verb.text_add")}`,
    );
    expect(describe(entry(3, "agent:x", ["clip.split", "clip.split", "clip.split"]))).toBe(`${t("activity.verb.clip_split")} (3)`);
    expect(describe(entry(4, "agent:x", ["effect.new"])), "commands without words show as they are").toBe("effect.new");
    setLanguage("hu");
    expect(describe(entry(3, "agent:x", ["clip.split", "clip.split"]))).toBe("Klip kettévágva (2)");
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
      [t("activity.who.you"), false, [3]],
      ["claude-code", true, [2]],
    ]);
    expect(runs([])).toEqual([]);
  });

  it("shows times only when the log has them", () => {
    expect(clock(undefined)).toBe("");
    expect(clock(0)).toMatch(/\d/);
  });
});
