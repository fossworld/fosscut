// Recording from the window: which devices a recording can use and what it defaults to,
// the request it makes, the time recorded and what to say when the system refuses.

import type { ApiError, CaptureDevice, DeviceKind, JobStatus, RecordChoice } from "../engine/types";

/** No sound with a camera or screen recording, as the engine takes it. */
export const NO_MICROPHONE = "none";

/** The engine's code for a device that cannot be found or opened. */
export const DEVICE_UNAVAILABLE = "fosscut.media.device_unavailable";

/** What the record panel holds. */
export interface RecordForm {
  what: DeviceKind;
  /** The device's name; empty for the first of its kind. */
  device: string;
  /** For a camera or screen, the microphone's name or `none`. */
  microphone: string;
}

/** The devices of a kind, in the order the system lists them. */
export function devicesOf(devices: CaptureDevice[], kind: DeviceKind): CaptureDevice[] {
  return devices.filter((d) => d.kind === kind);
}

/**
 * The form for recording `what`: its first device, and for a camera the first
 * microphone (no sound without one), for a screen no sound.
 */
export function defaultForm(what: DeviceKind, devices: CaptureDevice[]): RecordForm {
  const first = (kind: DeviceKind) => devicesOf(devices, kind)[0]?.name ?? "";
  const microphone = what === "camera" ? first("microphone") || NO_MICROPHONE : what === "screen" ? NO_MICROPHONE : "";
  return { what, device: first(what), microphone };
}

/** The request the form makes; the device and microphone only when there is a choice. */
export function recordChoice(form: RecordForm): RecordChoice {
  const choice: RecordChoice = { what: form.what };
  if (form.device) choice.device = form.device;
  if (form.what !== "microphone" && form.microphone) choice.microphone = form.microphone;
  return choice;
}

/** The time a recording has recorded, as `m:ss` (or `h:mm:ss`), from its whole seconds. */
export function recordedTime(status: JobStatus | null): string {
  const total = Math.max(0, Math.floor(status?.progress.done ?? 0));
  const [hours, minutes, seconds] = [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60];
  const pad = (n: number) => String(n).padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${minutes}:${pad(seconds)}`;
}

/**
 * A recording's error with its hint; a device that cannot be opened is usually one the
 * system has not let FossCut use, so that says where to allow it.
 */
export function recordProblem(error: ApiError): string {
  const sentence = (part: string) => (/[.!?]$/.test(part.trim()) ? part.trim() : `${part.trim()}.`);
  const text = error.hint ? `${sentence(error.message)} ${sentence(error.hint)}` : sentence(error.message);
  if (error.code !== DEVICE_UNAVAILABLE) return text;
  return `${text} The system may not have allowed FossCut to use the microphone, the camera or screen recording: allow it in System Settings, under Privacy and Security, then try again.`;
}
