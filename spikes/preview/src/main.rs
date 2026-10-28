//! FossCut preview-path spike for ADR-0010 (milestone 2). Throwaway code.
//!
//! The scene (4K H.264 decoded with VideoToolbox, transformed, masked and graded with a LUT
//! by `fosscut-render`, previewed at 1920x1080) is rendered with wgpu into the window behind
//! a transparent webview; the UI leaves a hole where the preview shows through. This is
//! option A, the native surface, which the spike chose. Option B (frames sent to a webview
//! canvas through a `frame://` URI scheme) was measured and removed; it is in the first
//! commit of the spike's pull request.
//!
//! The UI (`ui/`) drives the engine and measures it. With `FOSSCUT_SPIKE_BENCH=1` it runs
//! the benchmark, saves JSON to `results/` and quits.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fosscut_media::ffmpeg::{FfmpegVideoDecoder, probe_video};
use fosscut_media::{Acceleration, DecoderOptions, VideoDecoder, VideoFrame};
use fosscut_model::{
    Asset, AssetKind, Clip, ClipContent, Id, IdKind, MediaInfo, PathHints, Project,
    TimelineFormat, Track, TrackKind, Transform, VideoInfo,
};
use fosscut_render::color::TransferFunction;
use fosscut_render::lut::Lut3d;
use fosscut_render::plan::{Mask, MaskShape, RenderPlan, SourceRef, plan_frame};
use fosscut_render::{DecoderFrames, FrameProvider, RenderError, Renderer};
use fosscut_time::{Flicks, Rate};
use serde::Serialize;
use tauri::{Manager, State};

const PREVIEW: (u32, u32) = (1920, 1080);

/// Per-frame costs in milliseconds.
#[derive(Clone, Copy, Debug, Default, Serialize)]
struct Timing {
    decode_ms: f64,
    render_ms: f64,
    present_ms: f64,
    total_ms: f64,
    skipped: bool,
}

#[derive(Default)]
struct Samples {
    presented: u64,
    decode: Vec<f64>,
    render: Vec<f64>,
    present: Vec<f64>,
    total: Vec<f64>,
    hardware: bool,
}

#[derive(Serialize)]
struct Summary {
    presented: u64,
    decode_ms: [f64; 2],
    render_ms: [f64; 2],
    present_ms: [f64; 2],
    total_ms: [f64; 2],
    hardware_decode: bool,
}

fn percentiles(values: &[f64]) -> [f64; 2] {
    if values.is_empty() {
        return [0.0, 0.0];
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    [at(0.5), at(0.95)]
}

impl Samples {
    fn record(&mut self, timing: &Timing) {
        self.presented += 1;
        self.decode.push(timing.decode_ms);
        self.render.push(timing.render_ms);
        self.present.push(timing.present_ms);
        self.total.push(timing.total_ms);
    }

    fn summary(&self) -> Summary {
        Summary {
            presented: self.presented,
            decode_ms: percentiles(&self.decode),
            render_ms: percentiles(&self.render),
            present_ms: percentiles(&self.present),
            total_ms: percentiles(&self.total),
            hardware_decode: self.hardware,
        }
    }
}

enum Job {
    /// Render the frame at `time` into the window.
    Show {
        time: Flicks,
        reply: Option<Sender<Timing>>,
    },
    /// Where the preview hole is, in physical pixels of the window.
    Viewport { rect: [f32; 4] },
    /// Play from `from` in real time, or stop.
    Play { from: Option<Flicks> },
}

/// The benchmark scene: one 4K clip with a transform, a feathered mask and a LUT.
struct Scene {
    project: Project,
    timeline: Id,
    mask: Mask,
    lut: Arc<Lut3d>,
    duration: Flicks,
}

impl Scene {
    fn load(path: &str) -> Scene {
        let info = probe_video(path.as_ref()).expect("cannot probe the spike source");
        let rate = info.rate.unwrap_or(Rate::FPS_30);
        let duration = info.duration.unwrap_or(Flicks::from_seconds(10).unwrap_or(Flicks::ZERO));
        let format = TimelineFormat {
            width: PREVIEW.0,
            height: PREVIEW.1,
            rate,
            sample_rate: 48_000,
        };
        let mut project = Project::new(Id::new(IdKind::Project, 1), "Preview spike", format);
        let asset = Id::new(IdKind::Asset, 1);
        project.media.assets.push(Asset {
            id: asset.clone(),
            kind: AssetKind::Video,
            name: "source".into(),
            path: PathHints::default(),
            identity: None,
            info: MediaInfo {
                duration: Some(duration),
                container: None,
                video: Some(VideoInfo {
                    width: info.width,
                    height: info.height,
                    rate: Some(rate),
                    codec: Some(info.codec.clone()),
                    rotation: 0,
                }),
                audio: None,
            },
        });
        let timeline = project.manifest.primary_timeline.clone();
        let clip = Clip {
            id: Id::new(IdKind::Clip, 3),
            start: Flicks::ZERO,
            duration,
            content: ClipContent::Video {
                asset,
                source_in: Flicks::ZERO,
            },
            transform: Transform {
                position: [0.0, 0.0],
                scale: 0.92,
                rotation: 2.0,
            },
            opacity: 1.0,
            volume: 1.0,
        };
        if let Some(doc) = project.timelines.get_mut(&timeline) {
            doc.tracks.push(Track {
                id: Id::new(IdKind::Track, 2),
                kind: TrackKind::Video,
                name: None,
                muted: false,
                hidden: false,
                locked: false,
                clips: vec![clip],
            });
        }
        // A warm grade with lifted blacks, as a real look would be.
        let mut lut = Lut3d::identity(33);
        for rgb in &mut lut.data {
            rgb[0] = (rgb[0] * 1.06 + 0.02).min(1.0);
            rgb[1] = rgb[1] * 0.98 + 0.015;
            rgb[2] = (rgb[2] * 0.9 + 0.03).min(1.0);
        }
        Scene {
            project,
            timeline,
            mask: Mask {
                shape: MaskShape::Ellipse,
                center: [0.5, 0.5],
                size: [1.15, 1.25],
                feather: 0.35,
                invert: false,
            },
            lut: Arc::new(lut),
            duration,
        }
    }

    fn plan(&self, time: Flicks) -> Result<RenderPlan, RenderError> {
        let mut plan = plan_frame(&self.project, &self.timeline, time)?;
        for layer in &mut plan.layers {
            layer.mask = Some(self.mask);
            layer.lut = Some(self.lut.clone());
        }
        Ok(plan)
    }
}

/// Measures how long frame decoding takes.
struct TimedFrames {
    inner: DecoderFrames,
    spent: Duration,
}

impl FrameProvider for TimedFrames {
    fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError> {
        let start = Instant::now();
        let frame = self.inner.frame(source);
        self.spent += start.elapsed();
        frame
    }
}

/// The window surface and the pipeline that blits the preview into the hole.
struct Window {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    blit: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    viewport: [f32; 4],
    window: tauri::WebviewWindow,
}

const BLIT: &str = r"
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
struct Out { @builtin(position) position: vec4f, @location(0) uv: vec2f }
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.position = vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}
@fragment fn fs_main(in: Out) -> @location(0) vec4f {
    return textureSampleLevel(image, image_sampler, in.uv, 0.0);
}
";

struct Engine {
    scene: Scene,
    renderer: Renderer,
    frames: TimedFrames,
    offscreen: wgpu::Texture,
    window: Option<Window>,
    samples: Arc<Mutex<Samples>>,
    playing: Option<(Instant, Flicks)>,
}

impl Engine {
    fn draw(&mut self, time: Flicks) -> Result<Timing, String> {
        let start = Instant::now();
        self.frames.spent = Duration::ZERO;
        let plan = self.scene.plan(time).map_err(|e| e.to_string())?;
        let view = self.offscreen.create_view(&Default::default());
        self.renderer
            .render_to(
                &plan,
                &mut self.frames,
                &view,
                wgpu::TextureFormat::Rgba8Unorm,
                PREVIEW,
                TransferFunction::Bt709,
            )
            .map_err(|e| e.to_string())?;
        let decode = self.frames.spent;
        let mut timing = Timing {
            decode_ms: ms(decode),
            render_ms: ms(start.elapsed().saturating_sub(decode)),
            ..Timing::default()
        };
        if let Some(window) = &mut self.window {
            let presenting = Instant::now();
            present(&self.renderer, window, &view);
            timing.present_ms = ms(presenting.elapsed());
        }
        timing.total_ms = ms(start.elapsed());
        Ok(timing)
    }

    fn record(&self, timing: &Timing) {
        if let Ok(mut samples) = self.samples.lock() {
            samples.record(timing);
        }
    }

    fn run(mut self, jobs: Receiver<Job>) {
        loop {
            let first = if self.playing.is_some() {
                match jobs.try_recv() {
                    Ok(job) => Some(job),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            } else {
                match jobs.recv() {
                    Ok(job) => Some(job),
                    Err(_) => return,
                }
            };
            // Coalesce: only the newest Show is drawn; older ones are answered as skipped.
            let mut pending: Vec<Job> = first.into_iter().collect();
            while let Ok(job) = jobs.try_recv() {
                pending.push(job);
            }
            let mut latest_show: Option<(Flicks, Vec<Sender<Timing>>)> = None;
            for job in pending {
                match job {
                    Job::Show { time, reply } => {
                        let replies = latest_show.take().map(|(_, r)| r).unwrap_or_default();
                        let mut replies = replies;
                        replies.extend(reply);
                        latest_show = Some((time, replies));
                    }
                    Job::Viewport { rect } => {
                        if let Some(window) = &mut self.window {
                            window.viewport = rect;
                            resize(&self.renderer, window);
                        }
                    }
                    Job::Play { from } => {
                        self.playing = from.map(|t| (Instant::now(), t));
                    }
                }
            }
            if let Some((time, replies)) = latest_show {
                let timing = self.draw(time).unwrap_or(Timing {
                    skipped: true,
                    ..Timing::default()
                });
                self.record(&timing);
                let last = replies.len().saturating_sub(1);
                for (i, reply) in replies.into_iter().enumerate() {
                    let mut answer = timing;
                    answer.skipped = i != last;
                    let _ = reply.send(answer);
                }
            } else if let Some((started, from)) = self.playing {
                let elapsed = Flicks::new((started.elapsed().as_secs_f64() * 705_600_000.0) as i64);
                let mut time = from + elapsed;
                if time >= self.scene.duration {
                    time = from;
                    self.playing = Some((Instant::now(), from));
                }
                if let Ok(timing) = self.draw(time) {
                    self.record(&timing);
                }
            }
            if let Ok(mut samples) = self.samples.lock() {
                samples.hardware = self.frames_hardware();
            }
        }
    }

    fn frames_hardware(&self) -> bool {
        HARDWARE.lock().map(|h| *h).unwrap_or(false)
    }
}

/// Whether the decoder used hardware, reported by the decoder wrapper.
static HARDWARE: Mutex<bool> = Mutex::new(false);

/// Wraps the FFmpeg decoder to report its acceleration.
struct ReportingDecoder(FfmpegVideoDecoder);

impl VideoDecoder for ReportingDecoder {
    fn info(&self) -> &fosscut_media::VideoStreamInfo {
        self.0.info()
    }
    fn acceleration(&self) -> Acceleration {
        self.0.acceleration()
    }
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, fosscut_media::MediaError> {
        self.0.next_frame()
    }
    fn frame_at(&mut self, time: Flicks) -> Result<VideoFrame, fosscut_media::MediaError> {
        let frame = self.0.frame_at(time);
        if let Ok(mut hardware) = HARDWARE.lock() {
            *hardware = self.0.acceleration() == Acceleration::VideoToolbox;
        }
        frame
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn present(renderer: &Renderer, window: &mut Window, image: &wgpu::TextureView) {
    let frame = match window.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame)
        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
        _ => {
            resize(renderer, window);
            return;
        }
    };
    let target = frame.texture.create_view(&Default::default());
    let device = renderer.device();
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("blit"),
        layout: &window.blit.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(image),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&window.sampler),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.08,
                        g: 0.08,
                        b: 0.09,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let [x, y, w, h] = window.viewport;
        let (sw, sh) = (window.config.width as f32, window.config.height as f32);
        let (x, y) = (x.clamp(0.0, sw - 1.0), y.clamp(0.0, sh - 1.0));
        let (w, h) = (w.clamp(1.0, sw - x), h.clamp(1.0, sh - y));
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_pipeline(&window.blit);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    renderer.queue().submit([encoder.finish()]);
    renderer.queue().present(frame);
}

fn resize(renderer: &Renderer, window: &mut Window) {
    if let Ok(size) = window.window.inner_size() {
        window.config.width = size.width.max(1);
        window.config.height = size.height.max(1);
        window.surface.configure(renderer.device(), &window.config);
    }
}

struct Shared {
    jobs: Mutex<Sender<Job>>,
    samples: Arc<Mutex<Samples>>,
    info: serde_json::Value,
}

#[tauri::command]
fn info(shared: State<'_, Shared>) -> serde_json::Value {
    shared.info.clone()
}

#[tauri::command]
fn viewport(shared: State<'_, Shared>, x: f32, y: f32, width: f32, height: f32) {
    if let Ok(jobs) = shared.jobs.lock() {
        let _ = jobs.send(Job::Viewport {
            rect: [x, y, width, height],
        });
    }
}

#[tauri::command]
async fn show(shared: State<'_, Shared>, ms_time: f64) -> Result<Timing, String> {
    let (reply, answer) = channel();
    shared
        .jobs
        .lock()
        .map_err(|e| e.to_string())?
        .send(Job::Show {
            time: Flicks::new((ms_time * 705_600.0) as i64),
            reply: Some(reply),
        })
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || answer.recv())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn play(shared: State<'_, Shared>, from_ms: Option<f64>) {
    if let Ok(jobs) = shared.jobs.lock() {
        let _ = jobs.send(Job::Play {
            from: from_ms.map(|ms| Flicks::new((ms * 705_600.0) as i64)),
        });
    }
}

#[tauri::command]
fn stats(shared: State<'_, Shared>, reset: bool) -> Option<serde_json::Value> {
    let mut samples = shared.samples.lock().ok()?;
    let summary = serde_json::to_value(samples.summary()).ok();
    if reset {
        let hardware = samples.hardware;
        *samples = Samples {
            hardware,
            ..Samples::default()
        };
    }
    summary
}

#[tauri::command]
fn save_results(app: tauri::AppHandle, name: String, json: String, quit: bool) -> Result<String, String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("results");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    if quit {
        app.exit(0);
    }
    Ok(path.display().to_string())
}

fn main() {
    let source = std::env::var("FOSSCUT_SPIKE_SOURCE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/generated/test-pattern-4k.mp4").into()
    });
    let bench = std::env::var("FOSSCUT_SPIKE_BENCH").is_ok_and(|v| v == "1");
    let samples = Arc::new(Mutex::new(Samples::default()));
    let (jobs, receiver) = channel::<Job>();
    let receiver = Mutex::new(Some(receiver));

    tauri::Builder::default()
        .setup(move |app| {
            let window = app
                .get_webview_window("main")
                .ok_or("the main window is missing")?;
            let scene = Scene::load(&source);
            let instance = wgpu::Instance::new(
                wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
            );
            let surface = Some(instance.create_surface(window.clone())?);
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: surface.as_ref(),
                force_fallback_adapter: false,
                ..Default::default()
            }))?;
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("spike"),
                required_limits: adapter.limits(),
                ..Default::default()
            }))?;
            let adapter_info = adapter.get_info();
            let window_state = surface.map(|surface| {
                let caps = surface.get_capabilities(&adapter);
                let format = caps
                    .formats
                    .iter()
                    .copied()
                    .find(|f| !f.is_srgb())
                    .unwrap_or(caps.formats[0]);
                let size = window.inner_size().unwrap_or(tauri::PhysicalSize::new(1280, 900));
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format,
                    width: size.width.max(1),
                    height: size.height.max(1),
                    present_mode: wgpu::PresentMode::Fifo,
                    desired_maximum_frame_latency: 1,
                    alpha_mode: caps.alpha_modes[0],
                    view_formats: vec![],
                    color_space: Default::default(),
                };
                surface.configure(&device, &config);
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("blit"),
                    source: wgpu::ShaderSource::Wgsl(BLIT.into()),
                });
                let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("blit"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(format.into())],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                });
                Window {
                    surface,
                    config,
                    blit,
                    sampler,
                    viewport: [0.0, 0.0, 1.0, 1.0],
                    window: window.clone(),
                }
            });
            let renderer = Renderer::with_device(adapter_info.clone(), device, queue);
            let offscreen = renderer.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("preview"),
                size: wgpu::Extent3d {
                    width: PREVIEW.0,
                    height: PREVIEW.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let mut frames = DecoderFrames::new();
            let decoder = FfmpegVideoDecoder::open(source.as_ref(), DecoderOptions::default())?;
            let source_info = decoder.info().clone();
            frames.insert(Id::new(IdKind::Asset, 1), Box::new(ReportingDecoder(decoder)));
            let duration_ms = scene.duration.get() as f64 / 705_600.0;
            let info = serde_json::json!({
                "mode": "a-native-surface",
                "bench": bench,
                "adapter": adapter_info.name,
                "backend": format!("{:?}", adapter_info.backend),
                "preview": [PREVIEW.0, PREVIEW.1],
                "source": {
                    "codec": source_info.codec,
                    "size": [source_info.width, source_info.height],
                    "rate": source_info.rate.map(|r| r.to_string()),
                    "duration_ms": duration_ms,
                },
                "scale_factor": window.scale_factor().unwrap_or(1.0),
            });
            let engine = Engine {
                scene,
                renderer,
                frames: TimedFrames {
                    inner: frames,
                    spent: Duration::ZERO,
                },
                offscreen,
                window: window_state,
                samples: samples.clone(),
                playing: None,
            };
            let receiver = receiver
                .lock()
                .ok()
                .and_then(|mut r| r.take())
                .ok_or("the engine was already started")?;
            std::thread::Builder::new()
                .name("engine".into())
                .spawn(move || engine.run(receiver))?;
            app.manage(Shared {
                jobs: Mutex::new(jobs.clone()),
                samples: samples.clone(),
                info,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            info,
            viewport,
            show,
            play,
            stats,
            save_results
        ])
        .run(tauri::generate_context!())
        .expect("the spike failed to start");
}

