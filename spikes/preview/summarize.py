"""Median of each benchmark metric across runs, per preview mode (ADR-0010 spike).

Usage: python3 summarize.py [results-dir]
"""

import json
import statistics
import sys
from pathlib import Path

METRICS = [
    ("Playback, frames per second", "playback_5s", ["fps"]),
    ("Playback, render per frame (ms, p50)", "playback_5s", ["engine", "render_ms", 0], ["render_ms", "p50"]),
    ("Forward scrub, new frames per second", "scrub_forward_4s", ["fps"]),
    ("Forward scrub, latency p50 (ms)", "scrub_forward_4s", ["latency_ms", "p50"]),
    ("Forward scrub, latency p95 (ms)", "scrub_forward_4s", ["latency_ms", "p95"]),
    ("Backward scrub, new frames per second", "scrub_backward_4s", ["fps"]),
    ("Backward scrub, latency p50 (ms)", "scrub_backward_4s", ["latency_ms", "p50"]),
    ("Random seek, latency p50 (ms)", "random_seeks_30", ["latency_ms", "p50"]),
    ("Random seek, latency p95 (ms)", "random_seeks_30", ["latency_ms", "p95"]),
    ("Timeline scroll, frames per second", "timeline_scroll_4s", ["fps"]),
    ("Timeline scroll, frame time p95 (ms)", "timeline_scroll_4s", ["frame_ms", "p95"]),
    ("Timeline scroll during playback, frames per second", "timeline_scroll_while_playing_4s", ["fps"]),
    ("Timeline scroll during playback, frame time p95 (ms)", "timeline_scroll_while_playing_4s", ["frame_ms", "p95"]),
    ("Playback UI refresh p50 (ms)", "playback_5s", ["ui", "p50"]),
]


def lookup(run, step, *paths):
    for path in paths:
        value = run.get(step)
        for key in path:
            if isinstance(value, dict) and key in value:
                value = value[key]
            elif isinstance(value, list) and isinstance(key, int) and key < len(value):
                value = value[key]
            else:
                value = None
                break
        if isinstance(value, (int, float)):
            return float(value)
    return None


def main():
    directory = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).parent / "results")
    runs = {"a": [], "b": []}
    for path in sorted(directory.glob("[ab]-*.json")):
        runs[path.name[0]].append(json.loads(path.read_text()))
    print(f"| Metric | A: native surface ({len(runs['a'])} runs) | B: frame transport ({len(runs['b'])} runs) |")
    print("|---|---|---|")
    for label, step, *paths in METRICS:
        cells = []
        for mode in "ab":
            values = [v for v in (lookup(run, step, *paths) for run in runs[mode]) if v is not None]
            cells.append(f"{statistics.median(values):.1f}" if values else "n/a")
        print(f"| {label} | {cells[0]} | {cells[1]} |")


if __name__ == "__main__":
    main()
