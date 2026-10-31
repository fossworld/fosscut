import { CircleAlert } from "lucide-react";

import type { ApiError } from "../engine/types";
import { Glyph } from "../ui/controls";

/** A FossCut error with its hint, as panels show one. */
export function Problem({ error }: { error: ApiError }) {
  return (
    <p className="problem" role="alert">
      <Glyph icon={CircleAlert} size="sm" />
      <span>{problemText(error)}</span>
    </p>
  );
}

/**
 * The sentence a panel shows for `error`: the engine's message, which starts in lower case
 * as Rust errors do, as a sentence, followed by its hint.
 */
export function problemText(error: ApiError): string {
  const message = error.message.charAt(0).toUpperCase() + error.message.slice(1);
  if (!error.hint) return message;
  const ended = /[.!?]$/.test(message) ? message : `${message}.`;
  return `${ended} ${error.hint}`;
}
