// The caches for the stand-in engines (tests and `pnpm dev` in a plain browser), as the
// desktop's `cache_info` and `cache_clear` answer (ADR-0032): sizes in memory, which
// clearing empties. Without sizes (null), they are unknown and clearing is refused.

import type { ApiError, CacheKind, CacheUsage } from "./types";

export const CACHE_KINDS: CacheKind[] = ["analysis", "proxies", "remux", "frames"];

export class CacheStandIn {
  private bytes: Record<CacheKind, number | null>;

  constructor(sizes: Partial<Record<CacheKind, number>> | null) {
    this.bytes = Object.fromEntries(CACHE_KINDS.map((kind) => [kind, sizes ? (sizes[kind] ?? 0) : null])) as Record<
      CacheKind,
      number | null
    >;
  }

  async usage(): Promise<CacheUsage> {
    return {
      caches: CACHE_KINDS.map((kind) => ({ kind, bytes: this.bytes[kind], path: `/stand-in/cache/${kind}` })),
    };
  }

  async clear(kind: CacheKind | "all"): Promise<CacheUsage> {
    if (CACHE_KINDS.some((k) => this.bytes[k] === null)) {
      throw {
        code: "fosscut.cache.unavailable",
        message: "clearing caches from the app is not available yet",
      } satisfies ApiError;
    }
    let freed = 0;
    for (const k of CACHE_KINDS) {
      if (kind !== "all" && kind !== k) continue;
      freed += this.bytes[k] ?? 0;
      this.bytes[k] = 0;
    }
    return { ...(await this.usage()), freed_bytes: freed };
  }
}
