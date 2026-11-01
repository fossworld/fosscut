// Pictures of media for the interface: frames of video and image assets (the media grid's
// posters, the timeline's filmstrips) and the peaks of their sound (waveforms). Asked of
// the engine lazily, a few at a time, and kept in memory; the engine keeps them on disk.

import type { EngineClient } from "../engine/client";
import type { AssetView } from "../engine/types";
import { FLICKS_PER_SECOND } from "../transport/time";

/** How high the frames asked for are, in pixels: sharp at twice the timeline's rows. */
export const FRAME_HEIGHT = 96;
/** The most frames one request asks for, as the engine allows. */
export const BATCH = 48;
/** How many frames are kept in memory. */
export const KEPT_FRAMES = 800;

/** The steps of the filmstrip's time grid, in seconds: frames on it are shared by zooms. */
const STEPS = [0.125, 0.25, 0.5, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];

/**
 * The seconds between the frames of a filmstrip whose tiles each show `secondsPerTile`:
 * the smallest grid step at least that long, so zooming in and out reuses frames.
 */
export function gridStep(secondsPerTile: number): number {
  return STEPS.find((step) => step >= secondsPerTile) ?? STEPS[STEPS.length - 1]!;
}

/** The time on a grid of `step` seconds at or before `seconds`, in flicks. */
export function onGrid(seconds: number, step: number): number {
  return Math.round(Math.max(0, Math.floor(seconds / step + 1e-9) * step) * FLICKS_PER_SECOND);
}

/** How many peaks to ask for: about 120 a second, within what the engine allows. */
export function bucketsFor(flicks: number): number {
  return Math.min(65536, Math.max(256, Math.round((flicks / FLICKS_PER_SECOND) * 120)));
}

/** The lowest and highest peak between two fractions of a waveform; zeros when empty. */
export function peakBetween(peaks: Float32Array, from: number, to: number): [number, number] {
  const count = peaks.length / 2;
  if (count === 0) return [0, 0];
  const first = Math.max(0, Math.min(count - 1, Math.floor(from * count)));
  const last = Math.max(first, Math.min(count - 1, Math.ceil(to * count) - 1));
  let low = 0;
  let high = 0;
  for (let i = first; i <= last; i += 1) {
    low = Math.min(low, peaks[2 * i]!);
    high = Math.max(high, peaks[2 * i + 1]!);
  }
  return [low, high];
}

/** An asset as the pictures know it: what tells a replaced file from the one before. */
export type PictureAsset = Pick<AssetView, "id" | "kind" | "duration" | "video" | "missing">;

/** The asset's key: its id with what changes when its file is replaced. */
export function assetKey(asset: PictureAsset): string {
  return `${asset.id}:${asset.duration?.flicks ?? 0}:${asset.video?.width ?? 0}x${asset.video?.height ?? 0}`;
}

/** A waveform as kept: interleaved lowest and highest peaks, and the sound's length. */
export interface Peaks {
  duration: number;
  peaks: Float32Array;
}

type MediaEngine = Pick<EngineClient, "media">;

/** Frames and peaks of the open project's media, asked for when first wanted. */
export class Pictures {
  private frames = new Map<string, string>();
  private waveforms = new Map<string, Peaks>();
  /** Frames asked for and not answered yet, by asset key. */
  private wanted = new Map<string, { asset: PictureAsset; times: Set<number> }>();
  private asked = new Set<string>();
  private failed = new Set<string>();
  private listeners = new Set<() => void>();
  private flushing: ReturnType<typeof setTimeout> | null = null;
  private generation = 0;

  constructor(
    private readonly engine: MediaEngine,
    /** How long requests wait to be batched, in milliseconds. */
    private readonly wait = 16,
  ) {}

  /** Calls `listener` whenever pictures arrive; returns how to stop. */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private changes = 0;

  /** A number that grows whenever pictures arrive: what components re-render on. */
  version = (): number => this.changes;

  private changed(): void {
    this.changes += 1;
    for (const listener of this.listeners) listener();
  }

  /** Forgets everything, as when another project opens. */
  clear(): void {
    this.generation += 1;
    this.frames.clear();
    this.waveforms.clear();
    this.wanted.clear();
    this.asked.clear();
    this.failed.clear();
    this.changed();
  }

  /** True when the asset has a picture to show. */
  static pictured(asset: PictureAsset | undefined): asset is PictureAsset {
    return !!asset && !asset.missing && ["video", "image", "vector", "web"].includes(asset.kind);
  }

  /** True when the asset may have sound to draw. */
  static sounding(asset: PictureAsset | undefined): asset is PictureAsset {
    return !!asset && !asset.missing && (asset.kind === "audio" || asset.kind === "video");
  }

  /**
   * The frame of `asset` at `flicks` (source time) as a data URL, or null while it is asked
   * for. Images have one frame, at zero.
   */
  frame(asset: PictureAsset, flicks: number): string | null {
    if (!Pictures.pictured(asset)) return null;
    const key = assetKey(asset);
    // Images and drawings have one picture; a web composition shows its first frame, which
    // the engine renders once for all of it.
    const at = asset.kind === "video" ? Math.max(0, Math.round(flicks)) : 0;
    const id = `${key}@${at}`;
    const found = this.frames.get(id);
    if (found !== undefined) {
      // Most recently used last, so the oldest go first when memory is full.
      this.frames.delete(id);
      this.frames.set(id, found);
      return found;
    }
    if (this.failed.has(key) || this.asked.has(id)) return null;
    const entry = this.wanted.get(key) ?? { asset, times: new Set<number>() };
    entry.times.add(at);
    this.wanted.set(key, entry);
    this.asked.add(id);
    this.schedule();
    return null;
  }

  /** A frame a third of the way in: the asset's poster in the media grid. */
  poster(asset: PictureAsset): string | null {
    const length = asset.duration?.flicks ?? 0;
    return this.frame(asset, asset.kind === "video" ? onGrid(length / 3 / FLICKS_PER_SECOND, 0.5) : 0);
  }

  /** The asset's peaks, or null while they are asked for (or when it has no sound). */
  peaks(asset: PictureAsset): Peaks | null {
    if (!Pictures.sounding(asset)) return null;
    const key = assetKey(asset);
    const found = this.waveforms.get(key);
    if (found) return found;
    const id = `${key}~`;
    if (this.failed.has(id) || this.asked.has(id)) return null;
    this.asked.add(id);
    const generation = this.generation;
    const buckets = bucketsFor(asset.duration?.flicks ?? 10 * FLICKS_PER_SECOND);
    this.engine.media.waveform(asset.id, buckets).then(
      (waveform) => {
        if (generation !== this.generation) return;
        const peaks = new Float32Array(waveform.peaks.length * 2);
        waveform.peaks.forEach(([low, high], i) => {
          peaks[2 * i] = low;
          peaks[2 * i + 1] = high;
        });
        this.waveforms.set(key, { duration: waveform.duration, peaks });
        this.changed();
      },
      () => {
        if (generation === this.generation) this.failed.add(id);
      },
    );
    return null;
  }

  private schedule(): void {
    if (this.flushing !== null) return;
    this.flushing = setTimeout(() => {
      this.flushing = null;
      void this.flush();
    }, this.wait);
  }

  /** Asks for every frame wanted, a batch per request. */
  async flush(): Promise<void> {
    const wanted = [...this.wanted.values()];
    this.wanted.clear();
    const generation = this.generation;
    await Promise.all(
      wanted.flatMap(({ asset, times }) => {
        const all = [...times].sort((a, b) => a - b);
        const batches: number[][] = [];
        for (let i = 0; i < all.length; i += BATCH) batches.push(all.slice(i, i + BATCH));
        return batches.map(async (batch) => {
          const key = assetKey(asset);
          try {
            const strip = await this.engine.media.thumbnails({ asset: asset.id, times: batch, height: FRAME_HEIGHT });
            if (generation !== this.generation) return;
            strip.frames.forEach((frame, i) => {
              // The engine snaps times to frames; keep each under the time asked for.
              const at = batch[i] ?? frame.flicks;
              this.frames.set(`${key}@${at}`, frame.image);
            });
            this.trim();
            this.changed();
          } catch {
            if (generation === this.generation) this.failed.add(key);
          }
        });
      }),
    );
  }

  private trim(): void {
    while (this.frames.size > KEPT_FRAMES) {
      const oldest = this.frames.keys().next().value;
      if (oldest === undefined) break;
      this.frames.delete(oldest);
      // Asked for again when wanted again.
      this.asked.delete(oldest);
    }
  }
}
