//! The preview (ADR-0010, path A): frames rendered with wgpu straight into the window,
//! behind a transparent webview that leaves a hole where the video shows. One thread owns
//! the GPU: it draws when the playhead, the project or the hole changes, and paces playback
//! by the wall clock (audio, and the audio clock, come later).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use fosscut_engine::preview::Previewer;
use fosscut_model::{Id, Project};
use fosscut_render::Renderer;
use fosscut_time::{Flicks, TimeReport};
use serde::Serialize;

/// The project the preview shows.
pub struct Scene {
    /// The bundle, to find media.
    pub bundle: PathBuf,
    /// The project as it is now.
    pub project: Project,
    /// The timeline shown.
    pub timeline: Id,
}

/// Where preview frames go: the window's surface ([`crate::surface::Surface`]), or a
/// texture in tests.
pub trait Screen: Send + 'static {
    /// Follows the window's size.
    fn resize(&mut self, renderer: &Renderer);
    /// The texture a frame of `size` renders into, in [`crate::surface::FRAME_FORMAT`].
    fn frame_view(&mut self, renderer: &Renderer, size: (u32, u32)) -> wgpu::TextureView;
    /// Shows `frame` in `rect` (physical pixels), or only the background.
    fn present(&mut self, renderer: &Renderer, frame: Option<&wgpu::TextureView>, rect: [f32; 4]);
}

/// What the preview thread is asked to do.
pub enum Job {
    /// Show this project, or nothing.
    Scene(Option<Arc<Scene>>),
    /// The hole in the webview, in physical pixels: x, y, width, height.
    Viewport([f32; 4]),
    /// The window changed size or scale.
    Resized,
    /// Show the frame at this time, stopping playback.
    Show(Flicks),
    /// Play from the playhead.
    Play,
    /// Stop playing.
    Pause,
}

/// Where the playhead is, sent to the window each time a frame is shown.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Playhead {
    /// The frame shown.
    pub time: TimeReport,
    /// Whether playback goes on.
    pub playing: bool,
}

/// The preview thread's mailbox. Without a GPU the preview is off and jobs are dropped.
#[derive(Clone)]
pub struct Preview {
    jobs: Option<Sender<Job>>,
}

impl Preview {
    /// A preview that shows nothing, when the window has no GPU surface.
    pub fn off() -> Preview {
        Preview { jobs: None }
    }

    /// Starts the preview thread drawing onto `surface`. `playhead` hears every frame
    /// shown; `failed` hears why a frame could not be shown.
    pub fn start(
        surface: impl Screen,
        renderer: Renderer,
        playhead: impl Fn(Playhead) + Send + 'static,
        failed: impl Fn(String) + Send + 'static,
    ) -> std::io::Result<Preview> {
        let (jobs, receiver) = channel();
        let engine = Engine {
            surface: Box::new(surface) as Box<dyn Screen>,
            renderer,
            previewer: Previewer::new(),
            scene: None,
            viewport: [0.0, 0.0, 0.0, 0.0],
            time: Flicks::ZERO,
            clock: None,
            playhead: Box::new(playhead),
            failed: Box::new(failed),
        };
        std::thread::Builder::new()
            .name("fosscut-preview".into())
            .spawn(move || engine.run(&receiver))?;
        Ok(Preview { jobs: Some(jobs) })
    }

    /// Asks the preview thread to do `job`.
    pub fn send(&self, job: Job) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(job);
        }
    }
}

/// Playback pacing: the time shown is where playback started plus the wall-clock time
/// since.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    started: Instant,
    from: Flicks,
}

impl Clock {
    /// Starts at `from` now.
    pub fn start(from: Flicks, now: Instant) -> Clock {
        Clock { started: now, from }
    }

    /// The time to show at `now`.
    pub fn at(&self, now: Instant) -> Flicks {
        let elapsed = now.saturating_duration_since(self.started);
        let elapsed = Flicks::from_seconds_f64(elapsed.as_secs_f64()).unwrap_or(Flicks::ZERO);
        self.from + elapsed
    }
}

/// The largest rectangle with the canvas's `aspect` (width over height) centred in `hole`.
pub fn fit(hole: [f32; 4], aspect: f32) -> [f32; 4] {
    let [x, y, width, height] = hole;
    if width <= 0.0 || height <= 0.0 || aspect <= 0.0 {
        return [x, y, 0.0, 0.0];
    }
    let (w, h) = if width / height > aspect {
        (height * aspect, height)
    } else {
        (width, width / aspect)
    };
    [x + (width - w) / 2.0, y + (height - h) / 2.0, w, h]
}

/// The end of a timeline: playback stops on its last frame.
fn duration(scene: &Scene) -> Option<(Flicks, Flicks)> {
    let doc = scene.project.timelines.get(&scene.timeline)?;
    Some((doc.duration(), doc.format.rate.frame_duration()))
}

struct Engine {
    surface: Box<dyn Screen>,
    renderer: Renderer,
    previewer: Previewer,
    scene: Option<Arc<Scene>>,
    viewport: [f32; 4],
    time: Flicks,
    clock: Option<Clock>,
    playhead: Box<dyn Fn(Playhead) + Send>,
    failed: Box<dyn Fn(String) + Send>,
}

impl Engine {
    fn run(mut self, jobs: &Receiver<Job>) {
        loop {
            let first = if self.clock.is_some() {
                let interval = self
                    .scene
                    .as_deref()
                    .and_then(duration)
                    .map_or(Duration::from_millis(33), |(_, frame)| {
                        Duration::from_secs_f64(frame.as_seconds_f64())
                    });
                match jobs.recv_timeout(interval) {
                    Ok(job) => Some(job),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match jobs.recv() {
                    Ok(job) => Some(job),
                    Err(_) => return,
                }
            };
            let mut changed = false;
            for job in first.into_iter().chain(jobs.try_iter()) {
                changed |= self.apply(job);
            }
            if let Some(clock) = self.clock {
                self.time = clock.at(Instant::now());
                changed = true;
                if let Some((end, frame)) = self.scene.as_deref().and_then(duration)
                    && self.time >= end
                {
                    self.time = (end - frame).max(Flicks::ZERO);
                    self.clock = None;
                }
            }
            if changed {
                self.draw();
            }
        }
    }

    /// Applies one job; true when the picture may have changed.
    fn apply(&mut self, job: Job) -> bool {
        match job {
            Job::Scene(scene) => {
                if scene.is_none() {
                    self.clock = None;
                    self.time = Flicks::ZERO;
                }
                self.scene = scene;
            }
            Job::Viewport(rect) => self.viewport = rect,
            Job::Resized => self.surface.resize(&self.renderer),
            Job::Show(time) => {
                self.clock = None;
                self.time = time.max(Flicks::ZERO);
            }
            Job::Play => {
                let at_end = self
                    .scene
                    .as_deref()
                    .and_then(duration)
                    .is_some_and(|(end, frame)| self.time + frame >= end);
                if at_end {
                    self.time = Flicks::ZERO;
                }
                self.clock = Some(Clock::start(self.time, Instant::now()));
            }
            Job::Pause => self.clock = None,
        }
        true
    }

    fn draw(&mut self) {
        let Some(scene) = self.scene.clone() else {
            self.surface.present(&self.renderer, None, [0.0; 4]);
            return;
        };
        let aspect = scene
            .project
            .timelines
            .get(&scene.timeline)
            .map_or(16.0 / 9.0, |doc| {
                doc.format.width as f32 / doc.format.height.max(1) as f32
            });
        let rect = fit(self.viewport, aspect);
        if rect[2] < 1.0 || rect[3] < 1.0 {
            self.surface.present(&self.renderer, None, [0.0; 4]);
            return;
        }
        let max = (rect[2].round() as u32, rect[3].round() as u32);
        let shown = Previewer::size(&scene.project, &scene.timeline, max).and_then(|size| {
            let view = self.surface.frame_view(&self.renderer, size);
            let at = self.previewer.render_to(
                &mut self.renderer,
                &scene.project,
                &scene.bundle,
                &scene.timeline,
                self.time,
                max,
                &view,
                crate::surface::FRAME_FORMAT,
            )?;
            Ok((at, view))
        });
        match shown {
            Ok((at, view)) => {
                self.surface.present(&self.renderer, Some(&view), rect);
                (self.playhead)(Playhead {
                    time: at,
                    playing: self.clock.is_some(),
                });
            }
            Err(error) => {
                self.clock = None;
                self.surface.present(&self.renderer, None, [0.0; 4]);
                (self.failed)(error.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fit_the_hole_keeping_their_aspect() {
        assert_eq!(
            fit([10.0, 20.0, 1600.0, 900.0], 16.0 / 9.0),
            [10.0, 20.0, 1600.0, 900.0]
        );
        // A tall canvas in a wide hole: bars at the sides.
        assert_eq!(
            fit([0.0, 0.0, 1600.0, 900.0], 0.5625),
            [546.875, 0.0, 506.25, 900.0]
        );
        // A wide canvas in a tall hole: bars above and below.
        assert_eq!(
            fit([0.0, 0.0, 400.0, 900.0], 2.0),
            [0.0, 350.0, 400.0, 200.0]
        );
        assert_eq!(fit([5.0, 5.0, 0.0, 900.0], 2.0), [5.0, 5.0, 0.0, 0.0]);
        assert_eq!(fit([5.0, 5.0, 100.0, 100.0], 0.0)[2], 0.0);
    }

    #[test]
    fn the_clock_adds_wall_time_to_where_playback_started() {
        let now = Instant::now();
        let clock = Clock::start(Flicks::from_millis(500), now);
        assert_eq!(clock.at(now), Flicks::from_millis(500));
        assert_eq!(
            clock.at(now + Duration::from_millis(250)),
            Flicks::from_millis(750)
        );
        assert_eq!(
            clock.at(now - Duration::from_secs(1)),
            Flicks::from_millis(500),
            "never before the start"
        );
    }
}

/// The preview thread on a headless GPU, drawing into a texture instead of a window.
#[cfg(test)]
mod thread_tests {
    use std::sync::mpsc::{Receiver, Sender, channel};

    use fosscut_core::{Request, Session};
    use fosscut_model::{
        Asset, AssetKind, Clip, ClipContent, DirStore, IdKind, MediaInfo, PathHints,
        TimelineFormat, Track, TrackKind, Transform, VideoInfo,
    };
    use fosscut_time::Rate;
    use serde_json::json;

    use super::*;

    /// Records what the thread shows: the frame's rectangle, or `None` for background only.
    struct Offscreen {
        frame: Option<(wgpu::Texture, (u32, u32))>,
        shown: Sender<Option<[f32; 4]>>,
        resized: Sender<()>,
    }

    impl Screen for Offscreen {
        fn resize(&mut self, _: &Renderer) {
            let _ = self.resized.send(());
        }

        fn frame_view(&mut self, renderer: &Renderer, size: (u32, u32)) -> wgpu::TextureView {
            let texture = renderer.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("test frame"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: crate::surface::FRAME_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.frame = Some((texture, size));
            view
        }

        fn present(&mut self, _: &Renderer, frame: Option<&wgpu::TextureView>, rect: [f32; 4]) {
            let _ = self.shown.send(frame.map(|_| rect));
        }
    }

    struct Running {
        preview: Preview,
        shown: Receiver<Option<[f32; 4]>>,
        resized: Receiver<()>,
        heads: Receiver<Playhead>,
        failures: Receiver<String>,
    }

    const WAIT: Duration = Duration::from_secs(20);

    /// A preview thread on a headless renderer, or `None` without a GPU (CI requires one).
    fn running() -> Option<Running> {
        let renderer = match Renderer::headless() {
            Ok(renderer) => renderer,
            Err(error) => {
                let required = std::env::var_os("FOSSCUT_REQUIRE_GPU")
                    .is_some_and(|v| !v.is_empty() && v != "0");
                assert!(
                    !required,
                    "FOSSCUT_REQUIRE_GPU is set but there is no adapter: {error}"
                );
                return None;
            }
        };
        let (shown_tx, shown) = channel();
        let (resized_tx, resized) = channel();
        let (heads_tx, heads) = channel();
        let (failures_tx, failures) = channel();
        let screen = Offscreen {
            frame: None,
            shown: shown_tx,
            resized: resized_tx,
        };
        let preview = Preview::start(
            screen,
            renderer,
            move |head| {
                let _ = heads_tx.send(head);
            },
            move |message| {
                let _ = failures_tx.send(message);
            },
        )
        .unwrap();
        Some(Running {
            preview,
            shown,
            resized,
            heads,
            failures,
        })
    }

    /// A 16:9 project at 30 fps with one title lasting `frames` frames.
    fn titled(name: &str, frames: i64) -> Arc<Scene> {
        let dir =
            std::env::temp_dir().join(format!("fosscut-preview-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bundle = dir.join("Show.fosscut");
        let format = TimelineFormat {
            width: 320,
            height: 180,
            rate: Rate::FPS_30,
            sample_rate: 48_000,
        };
        fosscut_engine::project::create(&bundle, "Show", format).unwrap();
        let mut session = Session::open(DirStore::new(&bundle)).unwrap();
        session
            .execute(Request::single(
                "text.add",
                json!({ "text": "Preview", "start": "0f", "duration": format!("{frames}f") }),
            ))
            .unwrap();
        let project = session.project().clone();
        let timeline = project.manifest.primary_timeline.clone();
        Arc::new(Scene {
            bundle,
            project,
            timeline,
        })
    }

    /// The titled project with a video clip whose file does not exist.
    fn missing_media(name: &str) -> Arc<Scene> {
        let scene = titled(name, 30);
        let mut project = scene.project.clone();
        let asset = Id::new(IdKind::Asset, 90);
        project.media.assets.push(Asset {
            id: asset.clone(),
            kind: AssetKind::Video,
            name: "gone.mp4".into(),
            path: PathHints {
                relative: Some("../gone.mp4".into()),
                absolute: None,
            },
            identity: None,
            info: MediaInfo {
                duration: Some(Flicks::from_seconds(2).unwrap()),
                container: None,
                video: Some(VideoInfo {
                    width: 320,
                    height: 180,
                    rate: Some(Rate::FPS_30),
                    codec: None,
                    rotation: 0,
                }),
                audio: None,
            },
        });
        let doc = project.timelines.get_mut(&scene.timeline).unwrap();
        doc.tracks.insert(
            0,
            Track {
                id: Id::new(IdKind::Track, 91),
                kind: TrackKind::Video,
                name: None,
                muted: false,
                hidden: false,
                locked: false,
                caption_style: None,
                clips: vec![Clip {
                    id: Id::new(IdKind::Clip, 92),
                    start: Flicks::ZERO,
                    duration: Flicks::from_seconds(1).unwrap(),
                    content: ClipContent::Video {
                        asset,
                        source_in: Flicks::ZERO,
                    },
                    transform: Transform::default(),
                    opacity: 1.0,
                    volume: 1.0,
                }],
            },
        );
        Arc::new(Scene {
            bundle: scene.bundle.clone(),
            project,
            timeline: scene.timeline.clone(),
        })
    }

    fn frame(n: i64) -> Flicks {
        Rate::FPS_30.frames_to_flicks(n)
    }

    #[test]
    fn frames_are_drawn_where_the_hole_is_and_the_playhead_follows() {
        let Some(run) = running() else {
            return;
        };
        run.preview.send(Job::Viewport([0.0, 0.0, 640.0, 480.0]));
        run.preview.send(Job::Scene(Some(titled("drawn", 30))));
        let head = run.heads.recv_timeout(WAIT).unwrap();
        assert_eq!((head.time.frames, head.playing), (0, false));
        let drawn = std::iter::from_fn(|| run.shown.recv_timeout(WAIT).ok())
            .find(Option::is_some)
            .flatten();
        assert_eq!(
            drawn,
            Some([0.0, 60.0, 640.0, 360.0]),
            "letterboxed into the hole"
        );

        run.preview.send(Job::Show(frame(10) + Flicks::new(5)));
        assert_eq!(run.heads.recv_timeout(WAIT).unwrap().time.frames, 10);
        run.preview.send(Job::Show(frame(500)));
        assert_eq!(
            run.heads.recv_timeout(WAIT).unwrap().time.frames,
            29,
            "held on the last frame"
        );
        run.preview.send(Job::Resized);
        run.resized.recv_timeout(WAIT).unwrap();
        assert!(run.failures.try_recv().is_err());
    }

    #[test]
    fn playback_runs_to_the_last_frame_and_stops_then_starts_over() {
        let Some(run) = running() else {
            return;
        };
        run.preview.send(Job::Viewport([0.0, 0.0, 320.0, 180.0]));
        run.preview.send(Job::Scene(Some(titled("play", 8))));
        run.heads.recv_timeout(WAIT).unwrap();
        run.preview.send(Job::Play);
        let heads: Vec<Playhead> = std::iter::from_fn(|| run.heads.recv_timeout(WAIT).ok())
            .take_while(|head| head.playing)
            .collect();
        assert!(!heads.is_empty(), "frames were shown while playing");
        let frames: Vec<i64> = heads.iter().map(|h| h.time.frames).collect();
        assert!(
            frames.windows(2).all(|w| w[0] <= w[1]),
            "time only moves forward: {frames:?}"
        );
        // The head that ended the run (playing: false) sits on the last frame.
        run.preview.send(Job::Pause);
        let stopped = run.heads.recv_timeout(WAIT).unwrap();
        assert_eq!((stopped.time.frames, stopped.playing), (7, false));

        // Playing from the end starts over.
        run.preview.send(Job::Play);
        let restarted = run.heads.recv_timeout(WAIT).unwrap();
        assert!(restarted.time.frames < 7, "{restarted:?}");
        run.preview.send(Job::Pause);
    }

    #[test]
    fn without_a_project_or_a_hole_only_the_background_shows_and_failures_are_reported() {
        let Some(run) = running() else {
            return;
        };
        run.preview.send(Job::Scene(Some(titled("empty", 30))));
        assert_eq!(run.shown.recv_timeout(WAIT).unwrap(), None, "no hole yet");
        run.preview.send(Job::Viewport([0.0, 0.0, 320.0, 180.0]));
        run.preview.send(Job::Scene(None));
        assert_eq!(run.shown.recv_timeout(WAIT).unwrap(), None, "nothing open");
        run.preview.send(Job::Scene(Some(missing_media("missing"))));
        let failure = run.failures.recv_timeout(WAIT).unwrap();
        assert!(failure.contains("is missing"), "{failure}");
        assert!(run.heads.try_recv().is_err(), "no frame was shown");
        Preview::off().send(Job::Play);
    }
}
