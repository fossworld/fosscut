// The agent-aware activity view (ADR-0010, item 5): who changed the project, grouped into
// runs of consecutive changes by the same actor, in words a person reads at a glance.

import type { LogEntry } from "../engine/types";
import { type MessageKey, language, t } from "../i18n";

/** Consecutive changes by one actor, newest first. */
export interface Run {
  actor: string;
  who: string;
  agent: boolean;
  entries: LogEntry[];
}

/** The commands said in words, by the key of their message; others show as commands. */
const VERBS: Record<string, MessageKey> = {
  "media.add": "activity.verb.media_add",
  "track.add": "activity.verb.track_add",
  "clip.add": "activity.verb.clip_add",
  "clip.split": "activity.verb.clip_split",
  "clip.move": "activity.verb.clip_move",
  "clip.remove": "activity.verb.clip_remove",
  "clip.set": "activity.verb.clip_set",
  "text.add": "activity.verb.text_add",
  "captions.add": "activity.verb.captions_add",
  "captions.edit": "activity.verb.captions_edit",
  "captions.style": "activity.verb.captions_style",
  "timeline.remove_ranges": "activity.verb.timeline_remove_ranges",
  "timeline.duplicate": "activity.verb.timeline_duplicate",
  "history.undo": "activity.verb.history_undo",
  "history.redo": "activity.verb.history_redo",
};

/** The actor as a person reads it: you, the command line, or the agent's name. */
export function who(actor: string): string {
  if (actor === "human") return t("activity.who.you");
  if (actor === "cli") return t("activity.who.cli");
  if (actor.startsWith("agent:")) return actor.slice("agent:".length) || t("activity.who.agent");
  return actor || t("activity.who.unknown");
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
      const key = VERBS[command];
      const verb = key ? t(key) : command;
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

/** "12:04:31" for a change's time, in the viewer's time zone and the interface's language. */
export function clock(atMs: number | undefined): string {
  if (atMs === undefined) return "";
  return new Date(atMs).toLocaleTimeString(language(), { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}
