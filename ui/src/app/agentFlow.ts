// An AI edit of a video by the user's own agent (ADR-0043): what the dialog decides
// without the engine. Which dropped files are videos, the caps the user typed, the command
// as people read it, and the agent's report in a line.

import type { AgentChoice, AgentReport, AgentSetup, ApiError, JobStatus } from "../engine/types";
import type { AspectId } from "./layout";

/** The extensions of the videos FossCut imports, as the desktop's import dialog lists them. */
export const VIDEO_EXTENSIONS = ["mp4", "m4v", "mov", "mkv", "webm", "avi", "mts", "m2ts", "ts", "mxf", "mpg", "mpeg", "wmv", "flv", "3gp", "ogv"];

/** True for a path whose extension is a video's. */
export function isVideo(path: string): boolean {
  const extension = /\.([^./\\]+)$/.exec(path)?.[1]?.toLowerCase();
  return extension !== undefined && VIDEO_EXTENSIONS.includes(extension);
}

/** The file name of a path. */
export function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** An AI edit, from choosing the video to what the agent did. */
export interface AiEditDialog {
  open: boolean;
  /**
   * `video`: the video and the canvas; `brief`: the brief and the caps, or how to run an
   * agent by hand when none is found; `agree`: the command and the notes, before anything
   * runs; `run`: the agent at work, then what it did.
   */
  step: "video" | "brief" | "agree" | "run";
  video: string | null;
  aspect: AspectId;
  /** The agent found, the suggested brief and the notes; null until read. */
  setup: AgentSetup | null;
  brief: string;
  /** The turn cap as typed. */
  maxTurns: string;
  /** The spending cap in US dollars as typed; empty sets none. */
  budget: string;
  /** The command and the notes the engine asked the user to agree to. */
  command: string[];
  notes: string[];
  job: JobStatus | null;
  /** The run panel shows only its status line. */
  collapsed: boolean;
  busy: boolean;
  error: ApiError | null;
}

export const AI_EDIT_CLOSED: AiEditDialog = {
  open: false,
  step: "video",
  video: null,
  aspect: "9:16",
  setup: null,
  brief: "",
  maxTurns: "",
  budget: "",
  command: [],
  notes: [],
  job: null,
  collapsed: false,
  busy: false,
  error: null,
};

/** What is wrong with the brief or the caps, as a message key; null when nothing is. */
export type ChoiceProblem = "agent.brief.empty" | "agent.turns.invalid" | "agent.budget.invalid";

/** The run the dialog asks for, or what is wrong with it. */
export function agentChoice(
  dialog: Pick<AiEditDialog, "brief" | "maxTurns" | "budget" | "setup">,
  confirmed: boolean,
): { choice: AgentChoice } | { problem: ChoiceProblem } {
  const brief = dialog.brief.trim();
  if (!brief) return { problem: "agent.brief.empty" };
  const limit = dialog.setup?.max_turns_limit ?? 1000;
  const turns = Number(dialog.maxTurns.trim());
  if (!/^\d+$/.test(dialog.maxTurns.trim()) || turns < 1 || turns > limit) return { problem: "agent.turns.invalid" };
  const choice: AgentChoice = { brief, max_turns: turns, confirmed };
  const budgetText = dialog.budget.trim().replace(",", ".");
  if (budgetText) {
    const budget = Number(budgetText);
    if (!Number.isFinite(budget) || budget <= 0 || budget > 1000) return { problem: "agent.budget.invalid" };
    choice.max_budget_usd = budget;
  }
  return { choice };
}

/** `text` quoted for a POSIX shell when it needs it. */
export function quoted(text: string): string {
  return text !== "" && /^[\w\-./:=@%+,*]+$/.test(text) ? text : `'${text.replace(/'/g, "'\\''")}'`;
}

/**
 * The command as people read it: quoted for a shell, with the brief (the argument after
 * `-p`, shown in full above the command) standing in as `placeholder`.
 */
export function commandLine(command: string[], placeholder: string): string {
  return command.map((arg, index) => (index > 0 && command[index - 1] === "-p" ? placeholder : quoted(arg))).join(" ");
}

/** Elapsed milliseconds as minutes and seconds, such as 3:07. */
export function clock(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = String(seconds % 60).padStart(2, "0");
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}` : `${minutes}:${rest}`;
}

/** How a run stands, for its status chip. */
export type RunState = "starting" | "working" | "finished" | "limit" | "stopped" | "failed";

/** How the run of `job` stands. */
export function runState(job: JobStatus): RunState {
  if (job.state === "succeeded") return "finished";
  if (job.state === "cancelled") return "stopped";
  if (job.state === "failed") return job.error?.code === "fosscut.agent.limit_reached" ? "limit" : "failed";
  const report: AgentReport | undefined = job.agent;
  return report && (report.turns > 0 || report.connected !== undefined) ? "working" : "starting";
}
