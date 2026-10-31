//! Caches that make the preview fast without changing results (ADR-0032): proxies are made
//! once per content and shown only by the preview, with the original's framing; remuxed
//! copies decode the same frames as the original; rendered frames are kept and shown
//! without rendering, and keyed so edits change only the frames they touch; exports,
//! frame grabs and contact sheets keep reading the originals.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use common::renderer;
use fosscut_engine::cache::{CacheKind, CachePolicy, cache_info};
use fosscut_engine::frame::{contact_sheet, frame};
use fosscut_engine::jobs::{JobState, Jobs};
use fosscut_engine::preview::{PreviewOptions, PreviewSource, Previewer};
use fosscut_engine::proxy::{ProxySize, existing_proxy, make_proxy, proxy_path};
use fosscut_engine::remux::{existing_remux, remux, remux_path};
use fosscut_engine::{EngineError, ExportRequest, Preset, export};
use fosscut_media::ffmpeg::FfmpegVideoDecoder;
use fosscut_media::{DecoderOptions, HardwareDecoding, VideoDecoder};
use fosscut_model::{
    Asset, AssetKind, Clip, ClipContent, ContentIdentity, Id, IdKind, MediaInfo, PathHints,
    Project, TimelineFormat, Track, TrackKind, VideoInfo,
};
use fosscut_render::{Image, Renderer};
use fosscut_time::{Flicks, Rate};

const WAIT: Duration = Duration::from_secs(120);

/// The folder of this test binary: media files beside `Show.fosscut`, and caches.
fn dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-caches-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Show.fosscut")).unwrap();
        dir
    })
}

fn bundle() -> PathBuf {
    dir().join("Show.fosscut")
}

/// A fresh cache folder for one test.
fn cache(name: &str) -> PathBuf {
    let cache = dir().join(format!("cache-{name}"));
    let _ = std::fs::remove_dir_all(&cache);
    cache
}

/// Makes `name` beside the bundle with FFmpeg from `input` (a lavfi source) and `codec`
/// arguments, once.
fn media(name: &str, input: &str, codec: &[&str]) -> PathBuf {
    // Tests run as threads of one process: one makes each file while the others wait.
    static MAKING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _making = MAKING.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir().join(name);
    if !path.exists() {
        let partial = dir().join(format!("partial-{name}"));
        let status = Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", input])
            .args(codec)
            .args(["-f", format_of(name)])
            .arg(&partial)
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg could not make {name}");
        std::fs::rename(&partial, &path).unwrap();
    }
    path
}

fn format_of(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("ts") => "mpegts",
        Some("avi") => "avi",
        Some("mov") => "mov",
        _ => "mp4",
    }
}

/// A 1 s, 30 fps test pattern of 1000x562, whose half-size proxy (500x282) has a slightly
/// different aspect.
fn big() -> PathBuf {
    media(
        "big.mp4",
        "testsrc2=size=1000x562:rate=30:duration=1",
        &["-c:v", "mpeg4", "-q:v", "3"],
    )
}

/// A 320x180 timeline showing `file` (`width` x `height`, `frames` frames at 30 fps) as
/// asset 1 from its start, moved and scaled so framing matters.
fn showing(file: &str, size: (u32, u32), frames: i64, container: Option<&str>) -> Project {
    let rate = Rate::FPS_30;
    let format = TimelineFormat {
        width: 320,
        height: 180,
        rate,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Caches", format);
    let asset = Id::new(IdKind::Asset, 1);
    project.media.assets.push(Asset {
        folder: None,
        id: asset.clone(),
        kind: AssetKind::Video,
        name: file.into(),
        path: PathHints {
            relative: Some(format!("../{file}")),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            duration: Some(rate.frames_to_flicks(frames)),
            container: container.map(str::to_string),
            video: Some(VideoInfo {
                width: size.0,
                height: size.1,
                rate: Some(rate),
                codec: None,
                rotation: 0,
            }),
            ..Default::default()
        },
    });
    project.manifest.next_id = 10;
    let timeline = project.manifest.primary_timeline.clone();
    let mut clip = Clip::new(
        Id::new(IdKind::Clip, 3),
        Flicks::ZERO,
        rate.frames_to_flicks(frames),
        ClipContent::Video {
            asset,
            source_in: Flicks::ZERO,
        },
    );
    clip.transform.position = [0.1, -0.05];
    clip.transform.scale = 0.8;
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 2),
            kind: TrackKind::Video,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![clip],
        });
    project
}

fn asset() -> Id {
    Id::new(IdKind::Asset, 1)
}

fn timeline(project: &Project) -> Id {
    project.manifest.primary_timeline.clone()
}

fn identity(file: &Path) -> ContentIdentity {
    fosscut_engine::import::identity(file).unwrap()
}

fn at(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

/// Mean absolute difference over RGB after averaging `block` x `block` pixel blocks.
fn mad(a: &Image, b: &Image, block: u32) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let average = |image: &Image, bx: u32, by: u32, c: usize| {
        let mut sum = 0.0;
        for y in by * block..(by + 1) * block {
            for x in bx * block..(bx + 1) * block {
                sum += f64::from(image.pixel(x, y)[c]);
            }
        }
        sum / f64::from(block * block)
    };
    let (mut total, mut count) = (0.0, 0.0);
    for by in 0..a.height / block {
        for bx in 0..a.width / block {
            for c in 0..3 {
                total += (average(a, bx, by, c) - average(b, bx, by, c)).abs();
                count += 1.0;
            }
        }
    }
    total / count
}

/// How much redder than green an image is on average, -255 to 255.
fn redness(image: &Image) -> f64 {
    let pixels = image.rgba.chunks(4);
    let n = (image.width * image.height) as f64;
    pixels
        .map(|p| f64::from(p[0]) - f64::from(p[1]))
        .sum::<f64>()
        / n
}

fn grab(previewer: &mut Previewer, renderer: &mut Renderer, project: &Project, n: i64) -> Image {
    previewer
        .grab(
            renderer,
            project,
            &bundle(),
            &timeline(project),
            at(n),
            (320, 180),
        )
        .unwrap()
        .image
}

fn options(cache: &Path) -> PreviewOptions {
    PreviewOptions {
        cache: Some(cache.to_path_buf()),
        ..PreviewOptions::default()
    }
}

#[test]
fn proxies_are_made_once_per_content_and_show_the_same_picture() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let cache = cache("proxies");
    let source = big();
    let project = showing("big.mp4", (1000, 562), 30, None);
    let mut seen = Vec::new();
    let made = make_proxy(
        &project,
        &bundle(),
        &asset(),
        ProxySize::Half,
        &cache,
        &mut |p| {
            seen.push(p);
            true
        },
    )
    .unwrap();
    assert!(!made.reused);
    assert_eq!((made.width, made.height), (500, 282), "even, half size");
    assert_eq!((made.frames, made.rate), (30, Some(Rate::FPS_30)));
    let video = made.verified.video.as_ref().unwrap();
    assert_eq!(
        (video.codec.as_str(), video.profile.as_deref()),
        ("prores", Some("Proxy"))
    );
    assert!(made.verified.audio.is_none(), "proxies have no sound");
    assert_eq!(seen.last().map(|p| (p.done, p.total)), Some((30, 30)));
    let path = proxy_path(&cache, &identity(&source), ProxySize::Half);
    assert_eq!(made.path, path.display().to_string());
    assert_eq!(
        existing_proxy(&project, &bundle(), &asset(), ProxySize::Half, &cache).unwrap(),
        Some(path.clone())
    );
    assert_eq!(
        existing_proxy(&project, &bundle(), &asset(), ProxySize::Quarter, &cache).unwrap(),
        None
    );

    // Another project with the same file under another id finds the same proxy.
    let mut other = project.clone();
    other.media.assets[0].id = Id::new(IdKind::Asset, 7);
    let again = make_proxy(
        &other,
        &bundle(),
        &Id::new(IdKind::Asset, 7),
        ProxySize::Half,
        &cache,
        &mut |_| true,
    )
    .unwrap();
    assert!(again.reused, "made once per content and size");
    assert_eq!(again.path, made.path);
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".partial"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // The proxy decodes, in hardware where the machine can and in software otherwise; a
    // failure here names the reason the preview would fall back to the original.
    for hardware in [HardwareDecoding::Auto, HardwareDecoding::Off] {
        let decoded = FfmpegVideoDecoder::open(&path, DecoderOptions { hardware })
            .and_then(|mut decoder| decoder.next_frame());
        println!(
            "proxy decoded with {hardware:?}: {:?}",
            decoded.as_ref().map(|f| f.is_some())
        );
        if hardware == HardwareDecoding::Off {
            assert!(decoded.unwrap().is_some(), "the proxy decodes in software");
        }
    }

    // The preview shows the proxy with the original's picture and framing.
    let mut original = Previewer::with_options(options(&cache));
    let mut proxied = Previewer::with_options(PreviewOptions {
        proxies: Some(ProxySize::Half),
        ..options(&cache)
    });
    for n in [0, 11, 29] {
        let expected = grab(&mut original, &mut renderer, &project, n);
        let shown = grab(&mut proxied, &mut renderer, &project, n);
        let difference = mad(&expected, &shown, 4);
        assert!(difference < 3.0, "frame {n}: {difference}");
    }
    assert_eq!(original.source(&asset()), Some(PreviewSource::Original));
    assert_eq!(
        proxied.source(&asset()),
        Some(PreviewSource::Proxy(ProxySize::Half))
    );
    let mut quarter = Previewer::with_options(PreviewOptions {
        proxies: Some(ProxySize::Quarter),
        ..options(&cache)
    });
    grab(&mut quarter, &mut renderer, &project, 0);
    assert_eq!(
        quarter.source(&asset()),
        Some(PreviewSource::Original),
        "no quarter proxy was made"
    );

    // Cancelling keeps nothing.
    let cancelled = make_proxy(
        &project,
        &bundle(),
        &asset(),
        ProxySize::Quarter,
        &cache,
        &mut |_| false,
    )
    .unwrap_err();
    assert_eq!(cancelled.code(), "fosscut.job.cancelled");
    assert!(
        std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .all(|e| e.file_name() == path.file_name().unwrap()),
        "only the half proxy is there"
    );
}

/// The picture's area in pixels: every pixel's red over the red in the middle of the
/// layer, so a slightly darker or brighter copy does not count.
fn covered(image: &Image, middle: (u32, u32)) -> f64 {
    let full = f64::from(image.pixel(middle.0, middle.1)[0]);
    image.rgba.chunks(4).map(|p| f64::from(p[0])).sum::<f64>() / full
}

#[test]
fn proxies_cover_exactly_the_area_of_the_original() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let cache = cache("framing");
    // A white picture whose half-size proxy (500x282) is a little less wide for its height.
    media(
        "white.mp4",
        "color=c=white:size=1000x562:rate=30:duration=1",
        &["-c:v", "mpeg4", "-q:v", "2"],
    );
    let project = showing("white.mp4", (1000, 562), 30, None);
    make_proxy(
        &project,
        &bundle(),
        &asset(),
        ProxySize::Half,
        &cache,
        &mut |_| true,
    )
    .unwrap();
    let original = grab(
        &mut Previewer::with_options(options(&cache)),
        &mut renderer,
        &project,
        5,
    );
    let mut proxied = Previewer::with_options(PreviewOptions {
        proxies: Some(ProxySize::Half),
        ..options(&cache)
    });
    let shown = grab(&mut proxied, &mut renderer, &project, 5);
    assert_eq!(
        proxied.source(&asset()),
        Some(PreviewSource::Proxy(ProxySize::Half))
    );
    // The layer is moved right by a tenth and up by a twentieth of the canvas.
    let middle = (160 + 32, 90 - 9);
    let (a, b) = (covered(&original, middle), covered(&shown, middle));
    // Fitting the proxy's own aspect would cover about 0.2% less.
    assert!(
        (a - b).abs() / a < 0.0005,
        "the proxy covers {b} pixels, the original {a}"
    );
}

#[test]
fn exports_frame_grabs_and_contact_sheets_read_the_originals() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let cache = cache("originals");
    let source = big();
    let project = showing("big.mp4", (1000, 562), 30, None);
    // A "proxy" that is plainly not the original: solid red.
    let proxy = proxy_path(&cache, &identity(&source), ProxySize::Half);
    std::fs::create_dir_all(proxy.parent().unwrap()).unwrap();
    let red = media(
        "red.mov",
        "color=c=red:size=500x282:rate=30:duration=1",
        &["-c:v", "mpeg4", "-q:v", "2"],
    );
    std::fs::copy(&red, &proxy).unwrap();

    let mut previewer = Previewer::with_options(PreviewOptions {
        proxies: Some(ProxySize::Half),
        ..options(&cache)
    });
    let shown = grab(&mut previewer, &mut renderer, &project, 10);
    let grabbed = frame(
        &project,
        &bundle(),
        &timeline(&project),
        at(10),
        None,
        &mut renderer,
    )
    .unwrap();
    let original = grab(&mut Previewer::new(), &mut renderer, &project, 10);
    assert!(grabbed.image == original, "frame grabs read the original");
    assert!(
        redness(&shown) > redness(&grabbed.image) + 60.0,
        "the preview shows the proxy: {} against {}",
        redness(&shown),
        redness(&grabbed.image)
    );
    let sheet = contact_sheet(
        &project,
        &bundle(),
        &timeline(&project),
        4,
        160,
        &mut renderer,
    )
    .unwrap();
    assert!(
        redness(&sheet.image) < redness(&shown) - 60.0,
        "contact sheets read the original"
    );

    // An export with the proxy there is the export without it.
    let exported = |name: &str, renderer: &mut Renderer| {
        let output = dir().join(name);
        let _ = std::fs::remove_file(&output);
        let request = ExportRequest {
            bundle: bundle(),
            settings: Preset::MovProRes.settings(&project.timelines[&timeline(&project)].format),
            timeline: timeline(&project),
            output: output.clone(),
            allow_overwrite: false,
            expected_revision: None,
            request_id: None,
            cache: cache.clone(),
            options: Default::default(),
        };
        export(&project, &request, renderer, &mut |_| true).unwrap();
        let mut decoder = FfmpegVideoDecoder::open(
            &output,
            DecoderOptions {
                hardware: HardwareDecoding::Off,
            },
        )
        .unwrap();
        [0, 10, 29]
            .map(|n| decoder.frame_at(at(n)).unwrap().planes)
            .to_vec()
    };
    let with_proxy = exported("with-proxy.mov", &mut renderer);
    // The preview holds the proxy open, which Windows will not delete.
    drop(previewer);
    std::fs::remove_dir_all(&cache).unwrap();
    let without = exported("without-proxy.mov", &mut renderer);
    assert!(with_proxy == without, "the export read the original");
}

#[test]
fn remuxed_copies_show_the_same_frames_as_the_original() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let cache = cache("remux");
    for (file, codec) in [("clip.ts", "mpeg2video"), ("clip.avi", "mpeg4")] {
        let source = media(
            file,
            "testsrc2=size=320x180:rate=30:duration=1",
            &["-c:v", codec, "-q:v", "3", "-g", "10", "-bf", "2"],
        );
        let container = if file.ends_with(".ts") {
            "mpegts"
        } else {
            "avi"
        };
        let project = showing(file, (320, 180), 30, Some(container));
        let made = remux(&project, &bundle(), &asset(), &cache, &mut |_| true).unwrap();
        assert!(!made.reused, "{file}");
        assert_eq!(made.verified.brand.as_deref(), Some("qt  "));
        let path = remux_path(&cache, &identity(&source));
        assert_eq!(made.path, path.display().to_string());
        assert_eq!(
            existing_remux(&project, &bundle(), &asset(), &cache).unwrap(),
            Some(path)
        );
        assert!(
            remux(&project, &bundle(), &asset(), &cache, &mut |_| true)
                .unwrap()
                .reused,
            "made once per content"
        );

        // One previewer seeks back and forth in the copy; the original is read from its
        // start for every frame, which is exact even where seeking in it is not.
        let mut remuxed = Previewer::with_options(PreviewOptions {
            remuxed: true,
            ..options(&cache)
        });
        for n in [0, 1, 17, 9, 29, 3] {
            let mut original = Previewer::with_options(options(&cache));
            let expected = grab(&mut original, &mut renderer, &project, n);
            let shown = grab(&mut remuxed, &mut renderer, &project, n);
            assert!(
                shown == expected,
                "{file}: frame {n} differs by {}",
                mad(&shown, &expected, 1)
            );
        }
        assert_eq!(remuxed.source(&asset()), Some(PreviewSource::Remux));
    }

    // Files that seek well are not remuxed, and only video is.
    big();
    let mp4 = showing("big.mp4", (1000, 562), 30, Some("mov,mp4,m4a,3gp,3g2,mj2"));
    let error = remux(&mp4, &bundle(), &asset(), &cache, &mut |_| true).unwrap_err();
    assert_eq!(error.code(), "fosscut.cache.remux_not_needed");
    let mut image = mp4.clone();
    image.media.assets[0].kind = AssetKind::Image;
    let error = remux(&image, &bundle(), &asset(), &cache, &mut |_| true).unwrap_err();
    assert_eq!(error.code(), "fosscut.cache.not_video");
}

#[test]
fn rendered_frames_are_kept_shown_without_rendering_and_keyed_by_content() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let cache = cache("frames");
    big();
    let project = showing("big.mp4", (1000, 562), 30, None);
    let mut plain = Previewer::new();
    let expected = grab(&mut plain, &mut renderer, &project, 12);

    let with_cache = PreviewOptions {
        render_cache_bytes: Some(64 << 20),
        ..options(&cache)
    };
    let mut previewer = Previewer::with_options(with_cache.clone());
    assert!(grab(&mut previewer, &mut renderer, &project, 12) == expected);
    assert_eq!(previewer.render_cache().unwrap().len(), 1, "kept");
    assert!(grab(&mut previewer, &mut renderer, &project, 12) == expected);
    assert_eq!(
        previewer.render_cache().unwrap().len(),
        1,
        "shown from the cache"
    );

    // The window path draws a kept frame exactly as it would have rendered it.
    let shown = render_to(&mut previewer, &mut renderer, &project, 12);
    assert!(shown == expected.rgba, "a kept frame drawn into a window");
    let other = render_to(&mut previewer, &mut renderer, &project, 20);
    assert_eq!(previewer.render_cache().unwrap().len(), 2);
    assert!(
        other == grab(&mut plain, &mut renderer, &project, 20).rgba,
        "a new frame drawn into a window"
    );

    // A kept frame is shown without rendering: mark the one on disk, and it comes back.
    let files = frame_files(&cache);
    assert_eq!(files.len(), 2);
    for file in &files {
        let mut bytes = std::fs::read(file).unwrap();
        for byte in &mut bytes[16..] {
            *byte = 200;
        }
        std::fs::write(file, bytes).unwrap();
    }
    let mut reopened = Previewer::with_options(with_cache.clone());
    let marked = grab(&mut reopened, &mut renderer, &project, 12);
    assert!(marked.rgba.iter().all(|&b| b == 200), "from the cache");

    // An edit changes the key of the frames it touches.
    let mut edited = project.clone();
    edited
        .timelines
        .get_mut(&timeline(&project))
        .unwrap()
        .tracks[0]
        .clips[0]
        .opacity = 0.5;
    let fresh = grab(&mut reopened, &mut renderer, &edited, 12);
    assert!(fresh != marked, "rendered again");
    assert!(fresh == grab(&mut plain, &mut renderer, &edited, 12));
    assert_eq!(reopened.render_cache().unwrap().len(), 3);

    // Rendering ahead keeps frames that are not kept yet.
    let times: Vec<Flicks> = (0..6).map(at).collect();
    let timeline = timeline(&project);
    let ahead = reopened
        .prerender(
            &mut renderer,
            &project,
            &bundle(),
            &timeline,
            &times,
            (320, 180),
        )
        .unwrap();
    assert_eq!(ahead, 6);
    assert_eq!(
        reopened
            .prerender(
                &mut renderer,
                &project,
                &bundle(),
                &timeline,
                &times,
                (320, 180)
            )
            .unwrap(),
        0,
        "all kept"
    );

    // The limit holds, the least recently used going first.
    let frame_bytes = (16 + 320 * 180 * 4) as u64;
    reopened.set_options(PreviewOptions {
        render_cache_bytes: Some(4 * frame_bytes),
        ..with_cache.clone()
    });
    let kept = reopened.render_cache().unwrap();
    assert_eq!(kept.len(), 4);
    assert!(kept.bytes() <= 4 * frame_bytes);
    assert_eq!(frame_files(&cache).len(), 4);
    let usage = cache_info(&cache);
    let frames = usage.iter().find(|u| u.kind == CacheKind::Frames).unwrap();
    assert_eq!((frames.entries, frames.bytes), (4, 4 * frame_bytes));

    // Without a frame cache nothing is kept.
    reopened.set_options(options(&cache));
    assert!(reopened.render_cache().is_none());
}

fn frame_files(cache: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for shard in std::fs::read_dir(cache.join("frames")).unwrap().flatten() {
        for file in std::fs::read_dir(shard.path()).unwrap().flatten() {
            if file.file_name().to_string_lossy().ends_with(".rgba") {
                files.push(file.path());
            }
        }
    }
    files
}

/// Draws frame `n` with [`Previewer::render_to`] into a texture and reads it back.
fn render_to(
    previewer: &mut Previewer,
    renderer: &mut Renderer,
    project: &Project,
    n: i64,
) -> Vec<u8> {
    let (width, height) = (320u32, 180u32);
    let texture = renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("window"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    previewer
        .render_to(
            renderer,
            project,
            &bundle(),
            &timeline(project),
            at(n),
            (width, height),
            &view,
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .unwrap();
    let padded = (width * 4).div_ceil(256) * 256;
    let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue().submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    renderer
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let mut rgba = Vec::new();
    for row in mapped.chunks(padded as usize).take(height as usize) {
        rgba.extend_from_slice(&row[..(width * 4) as usize]);
    }
    rgba
}

#[test]
fn jobs_make_proxies_and_copies_join_cancel_and_apply_the_cache_policy() {
    if renderer().is_none() {
        return;
    }
    let cache = cache("jobs");
    let long = media(
        "long.mp4",
        "testsrc2=size=1280x720:rate=30:duration=10",
        &["-c:v", "mpeg4", "-q:v", "5"],
    );
    let project = showing("long.mp4", (1280, 720), 300, None);
    let jobs = Jobs::new();

    // An old rendered frame, which the policy removes once a job finishes.
    let old = cache.join("frames/00/old.rgba");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, [0u8; 64]).unwrap();
    let long_ago = std::time::SystemTime::now() - Duration::from_secs(90 * 86_400);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_accessed(long_ago)
                .set_modified(long_ago),
        )
        .unwrap();
    jobs.set_cache_policy(Some(CachePolicy {
        cache: cache.clone(),
        max_age_days: 30,
        limit_bytes: 0,
    }));

    let first = jobs
        .start_proxy(
            project.clone(),
            bundle(),
            asset(),
            ProxySize::Quarter,
            cache.clone(),
        )
        .unwrap();
    assert_eq!(
        (first.kind.as_str(), first.state),
        ("proxy", JobState::Running)
    );
    let path = proxy_path(&cache, &identity(&long), ProxySize::Quarter);
    assert_eq!(first.output.as_deref(), Some(path.to_str().unwrap()));
    assert_eq!(first.asset.as_deref(), Some("asset_0001"));
    let joined = jobs
        .start_proxy(
            project.clone(),
            bundle(),
            asset(),
            ProxySize::Quarter,
            cache.clone(),
        )
        .unwrap();
    assert_eq!(
        joined.job, first.job,
        "the same proxy joins the running job"
    );
    let done = jobs.status(&first.job, WAIT).unwrap();
    assert_eq!(done.state, JobState::Succeeded, "{:?}", done.error);
    let report = done.proxy.unwrap();
    assert_eq!(
        (report.width, report.height, report.frames),
        (320, 180, 300)
    );
    assert!(path.is_file());
    assert!(!old.exists(), "the cache policy ran after the job");

    // A cancelled proxy keeps nothing.
    let cancelled = jobs
        .start_proxy(
            project.clone(),
            bundle(),
            asset(),
            ProxySize::Half,
            cache.clone(),
        )
        .unwrap();
    let stopped = jobs.cancel(&cancelled.job, WAIT).unwrap();
    assert_eq!(stopped.state, JobState::Cancelled);
    assert_eq!(
        stopped.error.as_ref().map(|e| e.code.as_str()),
        Some("fosscut.job.cancelled")
    );
    assert!(!proxy_path(&cache, &identity(&long), ProxySize::Half).exists());

    // Remux jobs, and requests that fail before a job starts.
    let ts = media(
        "job.ts",
        "testsrc2=size=320x180:rate=30:duration=1",
        &["-c:v", "mpeg2video", "-q:v", "3"],
    );
    let stream = showing("job.ts", (320, 180), 30, Some("mpegts"));
    let copying = jobs
        .start_remux(stream.clone(), bundle(), asset(), cache.clone())
        .unwrap();
    assert_eq!(copying.kind, "remux");
    let copied = jobs.status(&copying.job, WAIT).unwrap();
    assert_eq!(copied.state, JobState::Succeeded, "{:?}", copied.error);
    assert_eq!(
        copied.remux.unwrap().path,
        remux_path(&cache, &identity(&ts)).display().to_string()
    );
    let not_needed = jobs
        .start_remux(project.clone(), bundle(), asset(), cache.clone())
        .unwrap_err();
    assert_eq!(not_needed.code(), "fosscut.cache.remux_not_needed");
    let mut image = project.clone();
    image.media.assets[0].kind = AssetKind::Image;
    let not_video = jobs
        .start_proxy(image, bundle(), asset(), ProxySize::Half, cache.clone())
        .unwrap_err();
    assert!(matches!(not_video, EngineError::NotVideo { .. }));
    let missing = jobs
        .start_proxy(
            project,
            bundle(),
            Id::new(IdKind::Asset, 9),
            ProxySize::Half,
            cache.clone(),
        )
        .unwrap_err();
    assert_eq!(missing.code(), "fosscut.asset.not_found");
    assert_eq!(jobs.list().len(), 3, "only the started jobs are listed");
}
