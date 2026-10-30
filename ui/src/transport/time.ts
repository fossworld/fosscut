// Frame-exact time on the UI side, mirroring fosscut-time (ADR-0005): flicks, rational
// rates, frame indices and SMPTE timecode. BigInt keeps the products exact.

export const FLICKS_PER_SECOND = 705_600_000;

export interface Rate {
  num: number;
  den: number;
}

/** "30", "30000/1001" or "29.97"-free rationals as the engine writes them. */
export function parseRate(text: string): Rate {
  const [num, den = "1"] = text.split("/");
  const rate = { num: Number(num), den: Number(den) };
  if (!Number.isInteger(rate.num) || !Number.isInteger(rate.den) || rate.num <= 0 || rate.den <= 0) {
    throw new Error(`not a frame rate: ${text}`);
  }
  return rate;
}

const big = BigInt;

/** Floor division that rounds toward negative infinity, as Rust's div_euclid does here. */
function floorDiv(a: bigint, b: bigint): bigint {
  const q = a / b;
  return a % b !== 0n && a < 0n !== b < 0n ? q - 1n : q;
}

/** The frame containing `flicks`. */
export function frameAt(flicks: number, rate: Rate): number {
  return Number(floorDiv(big(Math.round(flicks)) * big(rate.num), big(FLICKS_PER_SECOND) * big(rate.den)));
}

/** The start of frame `frames`, rounded to the nearest flick. */
export function framesToFlicks(frames: number, rate: Rate): number {
  const numerator = big(frames) * big(FLICKS_PER_SECOND) * big(rate.den);
  const denominator = big(rate.num);
  return Number(floorDiv(2n * numerator + denominator, 2n * denominator));
}

/** The start of the frame containing `flicks`. */
export function frameStart(flicks: number, rate: Rate): number {
  return framesToFlicks(frameAt(flicks, rate), rate);
}

function isDropFrame(rate: Rate): boolean {
  return rate.den === 1001 && (rate.num === 30000 || rate.num === 60000);
}

/** SMPTE timecode of frame `frames`: drop frame (with `;`) at 29.97 and 59.94. */
export function timecode(frames: number, rate: Rate): string {
  const sign = frames < 0 ? "-" : "";
  let frame = Math.abs(frames);
  const fps = Math.ceil(rate.num / rate.den);
  const drop = isDropFrame(rate);
  if (drop) {
    const dropped = Math.floor(fps / 15);
    const perTenMinutes = fps * 600 - dropped * 9;
    const perMinute = fps * 60 - dropped;
    const tens = Math.floor(frame / perTenMinutes);
    const remainder = frame % perTenMinutes;
    frame += dropped * 9 * tens;
    if (remainder > dropped) frame += dropped * Math.floor((remainder - dropped) / perMinute);
  }
  const pad = (n: number) => String(n).padStart(2, "0");
  const ff = frame % fps;
  const total = Math.floor(frame / fps);
  const separator = drop ? ";" : ":";
  return `${sign}${pad(Math.floor(total / 3600))}:${pad(Math.floor(total / 60) % 60)}:${pad(total % 60)}${separator}${pad(ff)}`;
}
