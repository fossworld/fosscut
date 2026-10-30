// The agent-aware activity view (ADR-0010, item 5): who changed the project, grouped into
// runs of consecutive changes by the same actor, in words a person reads at a glance.

import type { LogEntry } from "../engine/types";

/** Consecutive changes by one actor, newest first. */
export interface Run {
  actor: string;
  who: string;
  agent: boolean;
  entries: LogEntry[];
}

const VERBS: Record<string, string> = {
  "media.add": "Imported media",
  "track.add": "Added a track",
  "clip.add": "Added a clip",
  "clip.split": "Split a clip",
  "clip.move": "Moved a clip",
  "clip.remove": "Removed a clip",
  "clip.set": "Changed a clip",
  "text.add": "Added a title",
  "captions.add": "Added captions",
  "captions.edit": "Edited captions",
  "captions.style": "Styled captions",
  "timeline.remove_ranges": "Cut time ranges",
  "timeline.duplicate": "Copied the timeline",
  "history.undo": "Undid a change",
  "history.redo": "Redid a change",
};

/** The actor as a person reads it: you, the command line, or the agent's name. */
export function who(actor: string): string {
  if (actor === "human") return "You";
  if (actor === "cli") return "Command line";
  if (actor.startsWith("agent:")) return actor.slice("agent:".length) || "Agent";
  return actor || "Unknown";
}

/** True for changes an AI agent made. */
export function isAgent(actor: string): boolean {
  return actor.startsWith("agent:");
}

/** What a change did: its commands in words, repeated commands counted once with a count. */
export function describe(entry: LogEntry): string {
  const counts = new Map<string, number>();
  for (const command of entry.commands) counts.set(command, (counts.get(command) ?? 0) + 1);
  return [...counts]
    .map(([command, count]) => {
      const verb = VERBS[command] ?? command;
      return count > 1 ? `${verb} (${count})` : verb;
    })
    .join(", ");
}

/** Groups entries (newest first) into runs of consecutive changes by the same actor. */
export function runs(entries: LogEntry[]): Run[] {
  const grouped: Run[] = [];
  for (const entry of entries) {
    const last = grouped[grouped.length - 1];
    if (last && last.actor === entry.actor) {
      last.entries.push(entry);
    } else {
      grouped.push({
        actor: entry.actor,
        who: who(entry.actor),
        agent: isAgent(entry.actor),
        entries: [entry],
      });
    }
  }
  return grouped;
}

/** "12:04:31" for a change's time, in the viewer's time zone. */
export function clock(atMs: number | undefined): string {
  if (atMs === undefined) return "";
  return new Date(atMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}
