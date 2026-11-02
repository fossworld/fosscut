import { describe, expect, it } from "vitest";

import type { AgentSetup, JobStatus } from "../engine/types";
import { agentChoice, clock, commandLine, fileName, isVideo, quoted, runState } from "./agentFlow";

const setup = { max_turns_limit: 1000 } as AgentSetup;

describe("AI edits", () => {
  it("know a video by its extension", () => {
    expect(isVideo("/Users/me/Downloads/test_raw.mp4")).toBe(true);
    expect(isVideo("C:\\Clips\\Take 2.MOV")).toBe(true);
    expect(isVideo("/work/voice.wav")).toBe(false);
    expect(isVideo("/work/Show.fosscut")).toBe(false);
    expect(isVideo("/work/mp4")).toBe(false);
    expect(fileName("/Users/me/Downloads/test_raw.mp4")).toBe("test_raw.mp4");
  });

  it("check the brief and the caps before asking for a run", () => {
    const dialog = { brief: "  Cut the pauses.  ", maxTurns: "120", budget: "", setup };
    expect(agentChoice(dialog, false)).toEqual({ choice: { brief: "Cut the pauses.", max_turns: 120, confirmed: false } });
    expect(agentChoice({ ...dialog, budget: "2,5" }, true)).toEqual({
      choice: { brief: "Cut the pauses.", max_turns: 120, max_budget_usd: 2.5, confirmed: true },
    });
    expect(agentChoice({ ...dialog, brief: " \n " }, true)).toEqual({ problem: "agent.brief.empty" });
    expect(agentChoice({ ...dialog, maxTurns: "0" }, true)).toEqual({ problem: "agent.turns.invalid" });
    expect(agentChoice({ ...dialog, maxTurns: "1001" }, true)).toEqual({ problem: "agent.turns.invalid" });
    expect(agentChoice({ ...dialog, maxTurns: "12.5" }, true)).toEqual({ problem: "agent.turns.invalid" });
    expect(agentChoice({ ...dialog, budget: "-1" }, true)).toEqual({ problem: "agent.budget.invalid" });
    expect(agentChoice({ ...dialog, budget: "lots" }, true)).toEqual({ problem: "agent.budget.invalid" });
  });

  it("show the command as a shell reads it, with the brief standing in", () => {
    expect(quoted("mcp__fosscut__*,Skill")).toBe("mcp__fosscut__*,Skill");
    expect(quoted("/Users/me/My Apps/claude")).toBe("'/Users/me/My Apps/claude'");
    expect(quoted("it's")).toBe("'it'\\''s'");
    expect(quoted("")).toBe("''");
    const command = ["/Users/me/.local/bin/claude", "-p", "Edit it.\nAll of it.", "--mcp-config", "/cache/agent/1/mcp.json", "--max-turns", "200"];
    expect(commandLine(command, "<the brief above>")).toBe(
      "/Users/me/.local/bin/claude -p <the brief above> --mcp-config /cache/agent/1/mcp.json --max-turns 200",
    );
  });

  it("read elapsed time and how a run stands", () => {
    expect(clock(0)).toBe("0:00");
    expect(clock(67_500)).toBe("1:07");
    expect(clock(3_725_000)).toBe("1:02:05");
    const job = (patch: Partial<JobStatus>): JobStatus => ({ job: "job_0001", kind: "agent", state: "running", progress: { done: 0, total: 200 }, elapsed_ms: 0, ...patch });
    const report = { agent: "claude", turns: 0, max_turns: 200, tool_calls: 0, tool_errors: 0 };
    expect(runState(job({ agent: report }))).toBe("starting");
    expect(runState(job({ agent: { ...report, connected: true } }))).toBe("working");
    expect(runState(job({ agent: { ...report, turns: 3 } }))).toBe("working");
    expect(runState(job({ state: "succeeded" }))).toBe("finished");
    expect(runState(job({ state: "cancelled" }))).toBe("stopped");
    expect(runState(job({ state: "failed", error: { code: "fosscut.agent.limit_reached", message: "cap" } }))).toBe("limit");
    expect(runState(job({ state: "failed", error: { code: "fosscut.agent.failed", message: "no" } }))).toBe("failed");
  });
});
