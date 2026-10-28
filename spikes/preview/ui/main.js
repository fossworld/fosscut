// FossCut preview spike UI (throwaway, ADR-0010). Drives the engine in either mode and
// measures playback, scrubbing, seeking and timeline scrolling.
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const state = { info: null, mode: "a", durationMs: 10000, playing: false };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const nextFrame = () => new Promise((r) => requestAnimationFrame(r));
const log = (text) => { $("log").textContent += text + "\n"; $("log").scrollTop = 1e9; };

function percentile(values, q) {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.round((sorted.length - 1) * q)];
}
const summary = (values) => ({
  count: values.length,
  p50: +percentile(values, 0.5).toFixed(2),
  p95: +percentile(values, 0.95).toFixed(2),
  max: +percentile(values, 1).toFixed(2),
});

// ---- Preview: mode A tells the engine where the hole is; mode B draws fetched frames. ----

function reportViewport() {
  if (state.mode !== "a") return;
  const r = $("preview").getBoundingClientRect();
  const s = window.devicePixelRatio;
  invoke("viewport", { x: r.left * s, y: r.top * s, width: r.width * s, height: r.height * s });
}

const ctx = $("frame").getContext("2d");
const image = new ImageData(1920, 1080);

// Mode B: fetch one frame and draw it. Returns timings in milliseconds.
async function fetchFrame(ms) {
  const start = performance.now();
  const response = await fetch(`frame://localhost/${ms.toFixed(3)}`);
  const buffer = await response.arrayBuffer();
  const received = performance.now();
  image.data.set(new Uint8ClampedArray(buffer));
  ctx.putImageData(image, 0, 0);
  const drawn = performance.now();
  return {
    total: drawn - start,
    transfer: received - start - (+response.headers.get("x-decode-ms") || 0) - (+response.headers.get("x-render-ms") || 0),
    draw: drawn - received,
    decode: +response.headers.get("x-decode-ms") || 0,
    render: +response.headers.get("x-render-ms") || 0,
    bytes: buffer.byteLength,
  };
}

// Show the frame at `ms` and resolve once it should be on screen: after the engine
// presented (A) or the canvas was drawn (B), plus one display refresh.
async function showAt(ms) {
  const start = performance.now();
  let detail;
  if (state.mode === "a") detail = await invoke("show", { msTime: ms });
  else detail = await fetchFrame(ms);
  await nextFrame();
  return { latency: performance.now() - start, detail };
}

// ---- Timeline: 2,000 clips on 8 tracks, drawn on a 2D canvas, visible clips only. ----

const timeline = { canvas: $("timeline"), scroll: 0, clips: [], pxPerSecond: 40, contentWidth: 0 };
(function buildClips() {
  let seed = 7;
  const random = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  const tracks = 8;
  const ends = new Array(tracks).fill(0);
  for (let i = 0; i < 2000; i++) {
    const track = i % tracks;
    const start = ends[track] + random() * 1.5;
    const length = 1 + random() * 6;
    ends[track] = start + length;
    timeline.clips.push({ track, start, length, hue: Math.floor(random() * 360), name: `clip ${i + 1}` });
  }
  timeline.contentWidth = Math.max(...ends) * timeline.pxPerSecond;
})();

function drawTimeline() {
  const { canvas } = timeline;
  const s = window.devicePixelRatio;
  const w = canvas.clientWidth, h = canvas.clientHeight;
  if (canvas.width !== w * s || canvas.height !== h * s) { canvas.width = w * s; canvas.height = h * s; }
  const g = canvas.getContext("2d");
  g.setTransform(s, 0, 0, s, 0, 0);
  g.fillStyle = "#16171a"; g.fillRect(0, 0, w, h);
  const trackHeight = (h - 20) / 8;
  const left = timeline.scroll / timeline.pxPerSecond, right = (timeline.scroll + w) / timeline.pxPerSecond;
  g.fillStyle = "#6b6d75"; g.font = "10px system-ui";
  for (let sec = Math.floor(left); sec <= right; sec++) {
    const x = sec * timeline.pxPerSecond - timeline.scroll;
    g.fillRect(x, 0, 1, sec % 5 === 0 ? 10 : 5);
    if (sec % 5 === 0) g.fillText(`${sec}s`, x + 3, 10);
  }
  let drawn = 0;
  for (const clip of timeline.clips) {
    if (clip.start > right || clip.start + clip.length < left) continue;
    const x = clip.start * timeline.pxPerSecond - timeline.scroll;
    const y = 20 + clip.track * trackHeight;
    g.fillStyle = `hsl(${clip.hue} 45% 38%)`;
    g.fillRect(x, y + 2, clip.length * timeline.pxPerSecond - 2, trackHeight - 4);
    g.fillStyle = "#f0f0f2";
    g.fillText(clip.name, x + 4, y + 14);
    drawn++;
  }
  return drawn;
}
timeline.canvas.addEventListener("wheel", (e) => {
  timeline.scroll = Math.max(0, Math.min(timeline.contentWidth, timeline.scroll + e.deltaX + e.deltaY));
  drawTimeline();
});

// ---- Interactive controls. ----

$("scrub").addEventListener("input", async (e) => {
  const ms = (+e.target.value / 10000) * state.durationMs;
  $("time").textContent = `${(ms / 1000).toFixed(3)} s`;
  if (state.mode === "a") invoke("show", { msTime: ms });
  else if (!state.scrubBusy) { state.scrubBusy = true; await fetchFrame(ms); state.scrubBusy = false; }
});
$("play").addEventListener("click", () => {
  state.playing = !state.playing;
  $("play").textContent = state.playing ? "Pause" : "Play";
  if (state.mode === "a") invoke("play", { fromMs: state.playing ? 0 : null });
  else if (state.playing) playTransport(1e9);
});
$("bench").addEventListener("click", () => runBenchmark());

// Mode B playback: request frames back to back for `durationMs`, following a real-time clock.
async function playTransport(durationMs) {
  const start = performance.now();
  const frames = [];
  while (performance.now() - start < durationMs && (state.playing || durationMs < 1e9)) {
    const ms = (performance.now() - start) % state.durationMs;
    frames.push(await fetchFrame(ms));
  }
  return { seconds: (performance.now() - start) / 1000, frames };
}

// ---- Benchmark. ----

// Counts display refreshes and their intervals while `work` runs.
async function measureRefresh(work) {
  const intervals = [];
  let last = performance.now(), running = true;
  (async () => { while (running) { await nextFrame(); const now = performance.now(); intervals.push(now - last); last = now; } })();
  const result = await work();
  running = false;
  return { result, refresh: summary(intervals), refreshFps: +(1000 / (percentile(intervals, 0.5) || 1)).toFixed(1) };
}

async function benchPlayback(seconds) {
  if (state.mode === "a") {
    await invoke("stats", { reset: true });
    await invoke("play", { fromMs: 0 });
    const measured = await measureRefresh(() => sleep(seconds * 1000));
    await invoke("play", { fromMs: null });
    const stats = await invoke("stats", { reset: true });
    return { fps: +(stats.presented / seconds).toFixed(1), engine: stats, ui: measured.refresh };
  }
  const measured = await measureRefresh(() => playTransport(seconds * 1000));
  const { frames } = measured.result;
  return {
    fps: +(frames.length / seconds).toFixed(1),
    total_ms: summary(frames.map((f) => f.total)),
    transfer_ms: summary(frames.map((f) => f.transfer)),
    draw_ms: summary(frames.map((f) => f.draw)),
    decode_ms: summary(frames.map((f) => f.decode)),
    render_ms: summary(frames.map((f) => f.render)),
    megabytes_per_second: +(frames.reduce((a, f) => a + f.bytes, 0) / seconds / 1e6).toFixed(1),
    ui: measured.refresh,
  };
}

// Scrub across the clip at 2.5x speed (forward or backward), one target per display refresh.
async function benchScrub(direction, seconds) {
  const latencies = [];
  let shown = 0, inFlight = false;
  const start = performance.now();
  const measured = await measureRefresh(async () => {
    while (performance.now() - start < seconds * 1000) {
      const progress = (performance.now() - start) / (seconds * 1000);
      const ms = (direction === "forward" ? progress : 1 - progress) * (state.durationMs - 40);
      if (state.mode === "a") {
        const sent = performance.now();
        invoke("show", { msTime: ms }).then((t) => { if (!t.skipped) { shown++; latencies.push(performance.now() - sent); } });
      } else if (!inFlight) {
        inFlight = true;
        const sent = performance.now();
        fetchFrame(ms).then(() => { shown++; latencies.push(performance.now() - sent); inFlight = false; });
      }
      await nextFrame();
    }
    await sleep(300);
  });
  return { fps: +(shown / seconds).toFixed(1), latency_ms: summary(latencies), ui: measured.refresh };
}

// Jump to random times and wait for each frame to be on screen.
async function benchSeeks(count) {
  let seed = 42;
  const random = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  const latencies = [];
  for (let i = 0; i < count; i++) {
    const { latency } = await showAt(random() * (state.durationMs - 40));
    latencies.push(latency);
  }
  return { latency_ms: summary(latencies) };
}

// Scroll the timeline by 12 px per refresh and record refresh intervals.
async function benchTimeline(seconds, whilePlaying) {
  if (whilePlaying && state.mode === "a") await invoke("play", { fromMs: 0 });
  let playback = null;
  if (whilePlaying && state.mode === "b") { state.playing = true; playback = playTransport(seconds * 1000); }
  const intervals = [];
  let drawn = 0, last = performance.now();
  const start = last;
  while (performance.now() - start < seconds * 1000) {
    timeline.scroll = (timeline.scroll + 12) % Math.max(1, timeline.contentWidth - 2000);
    drawn = drawTimeline();
    await nextFrame();
    const now = performance.now();
    intervals.push(now - last);
    last = now;
  }
  if (whilePlaying && state.mode === "a") await invoke("play", { fromMs: null });
  if (playback) { state.playing = false; await playback; }
  return {
    fps: +(intervals.length / seconds).toFixed(1),
    frame_ms: summary(intervals),
    clips_total: timeline.clips.length,
    clips_drawn_last_frame: drawn,
  };
}

async function runBenchmark() {
  $("bench").disabled = true;
  const results = { info: state.info, started: new Date().toISOString(), user_agent: navigator.userAgent };
  const steps = [
    ["playback_5s", () => benchPlayback(5)],
    ["scrub_forward_4s", () => benchScrub("forward", 4)],
    ["scrub_backward_4s", () => benchScrub("backward", 4)],
    ["random_seeks_30", () => benchSeeks(30)],
    ["timeline_scroll_4s", () => benchTimeline(4, false)],
    ["timeline_scroll_while_playing_4s", () => benchTimeline(4, true)],
  ];
  await sleep(1500);
  await showAt(0);
  for (const [name, step] of steps) {
    log(`running ${name}...`);
    results[name] = await step();
    log(`${name}: ${JSON.stringify(results[name])}`);
    await sleep(500);
  }
  const name = `${state.mode}-${Date.now()}`;
  const path = await invoke("save_results", { name, json: JSON.stringify(results, null, 2), quit: !!state.info.bench });
  log(`saved ${path}`);
  $("bench").disabled = false;
}

// ---- Start. ----

(async function start() {
  state.info = await invoke("info");
  state.mode = state.info.mode.startsWith("a") ? "a" : "b";
  state.durationMs = state.info.source.duration_ms;
  $("mode").textContent = `${state.info.mode} on ${state.info.adapter} (${state.info.backend})`;
  if (state.mode === "b") $("preview").classList.add("transport");
  log(JSON.stringify(state.info, null, 2));
  drawTimeline();
  reportViewport();
  window.addEventListener("resize", () => { reportViewport(); drawTimeline(); });
  await showAt(0);
  setInterval(async () => {
    const stats = await invoke("stats", { reset: false });
    if (stats) $("meter").textContent = `presented ${stats.presented}, decode p50 ${stats.decode_ms[0].toFixed(1)} ms, render p50 ${stats.render_ms[0].toFixed(1)} ms, hw decode ${stats.hardware_decode}`;
  }, 1000);
  if (state.info.bench) runBenchmark();
})();
