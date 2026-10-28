# Preview-path spike (ADR-0010)

Throwaway Tauri 2 app from milestone 2 that measured how preview frames should reach the webview. Results and the decision: [milestone 2 spike results](../../docs/development/milestone-2-spike-results.md). The desktop app (milestone 5) replaces this code; it is not part of the workspace, tests or coverage.

The scene: the 4K test pattern from `cargo xtask fixtures`, transformed, masked and graded with a LUT by `fosscut-render`, previewed at 1920x1080. Mode A renders with wgpu into the window behind a transparent webview, which leaves a hole over the preview.

## Run

Needs FFmpeg 8 development libraries as described in [CONTRIBUTING.md](../../CONTRIBUTING.md#ffmpeg-for-development).

```bash
cargo xtask fixtures
```

```bash
cd spikes/preview && cargo run --release
```

Environment variables:

| Variable | Effect |
|---|---|
| `FOSSCUT_SPIKE_BENCH=1` | Run the scripted benchmark, save JSON to `results/`, quit |
| `FOSSCUT_SPIKE_SOURCE=path` | Use another video instead of the 4K test pattern |

`cargo run --release --bin decode_bench` measures the decode budget, and `python3 summarize.py` prints the medians of all saved runs.
