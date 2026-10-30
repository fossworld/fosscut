//! The export formats beyond MP4, MOV and WebM: every ProRes profile, ProRes 4444 alpha,
//! looping GIFs, WAV, FLAC and MP3, and bit rates. Each file is written from known frames
//! and samples, probed, and where it matters decoded back and compared (ADR-0009, items 5
//! and 6). Decoding GIF and ProRes 4444 to RGBA needs the `ffmpeg` command; those checks
//! are skipped without it unless `FOSSCUT_REQUIRE_FFMPEG` is set.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use fosscut_media::ffmpeg::{
    FfmpegAudioDecoder, FfmpegWriter, audio_encoder, encoder_capabilities, probe_output,
    video_encoder,
};
use fosscut_media::{
    AudioBuffer, AudioCodec, AudioSettings, ColorInfo, Container, OutputReport, OutputSettings,
    PixelFormat, Plane, RgbaFrame, VideoCodec, VideoFrame, VideoSettings,
};
use fosscut_time::{Flicks, Rate};

const WIDTH: u32 = 160;
const HEIGHT: u32 = 90;
const FRAMES: i64 = 12;
const RATE: Rate = Rate::FPS_24;
const SAMPLE_RATE: u32 = 48_000;

fn have_ffmpeg() -> bool {
    let found = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        found || !required,
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is not installed"
    );
    found
}

fn dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-formats-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    })
}

/// Frame `n` of a gradient that moves: red across, green down, a blue wave sliding with
/// `n`. With `transparent_left`, the left half has alpha 0.
fn gradient(n: i64, transparent_left: bool) -> RgbaFrame {
    let mut rgba = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let r = (x * 255 / (WIDTH - 1)) as u8;
            let g = (y * 255 / (HEIGHT - 1)) as u8;
            let t = (x + y) as f32 / 18.0 + n as f32 * 0.5;
            let b = (128.0 + 100.0 * t.sin()).round() as u8;
            let a = if transparent_left && x < WIDTH / 2 {
                0
            } else {
                255
            };
            rgba.extend([r, g, b, a]);
        }
    }
    RgbaFrame {
        width: WIDTH,
        height: HEIGHT,
        rgba,
    }
}

/// `seconds` of a 440 Hz tone at 0.3 on the left and 0.2 on the right.
fn tone(channels: u16, seconds: f64) -> AudioBuffer {
    let frames = (f64::from(SAMPLE_RATE) * seconds).round() as usize;
    let mut samples = Vec::with_capacity(frames * usize::from(channels));
    for i in 0..frames {
        let v = (2.0 * std::f64::consts::PI * 440.0 * i as f64 / f64::from(SAMPLE_RATE)).sin();
        samples.push((0.3 * v) as f32);
        if channels == 2 {
            samples.push((0.2 * v) as f32);
        }
    }
    AudioBuffer {
        sample_rate: SAMPLE_RATE,
        channels,
        samples,
    }
}

fn video(codec: VideoCodec) -> VideoSettings {
    VideoSettings {
        codec,
        width: WIDTH,
        height: HEIGHT,
        rate: RATE,
        bit_rate: None,
    }
}

fn audio(codec: AudioCodec, channels: u16) -> AudioSettings {
    AudioSettings {
        channels,
        ..AudioSettings::stereo(codec)
    }
}

/// Writes `FRAMES` frames from `frame` and, when the output has audio, half a second of
/// the tone in one chunk per frame, then probes the file.
fn write(
    name: &str,
    settings: &OutputSettings,
    frame: impl Fn(i64) -> RgbaFrame,
) -> (PathBuf, OutputReport) {
    let path = dir().join(name);
    let mut writer = FfmpegWriter::create(&path, settings).unwrap();
    let channels = settings.audio.as_ref().map_or(2, |a| a.channels);
    let all = tone(channels, 0.5);
    let per_frame = all.samples.len() / FRAMES as usize;
    let mut chunks = all.samples.chunks(per_frame);
    for n in 0..FRAMES {
        if settings.video.is_some() {
            writer.push_rgba(&frame(n)).unwrap();
        }
        if let (Some(_), Some(chunk)) = (&settings.audio, chunks.next()) {
            writer
                .push_audio(&AudioBuffer {
                    samples: chunk.to_vec(),
                    ..all.clone()
                })
                .unwrap();
        }
    }
    writer.finish().unwrap();
    let report = probe_output(&path).unwrap();
    (path, report)
}

/// Decodes every frame of `path` to RGBA with the `ffmpeg` command, one per stored frame.
fn decode_rgba(path: &Path) -> Vec<Vec<u8>> {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let size = (WIDTH * HEIGHT * 4) as usize;
    assert_eq!(output.stdout.len() % size, 0);
    output.stdout.chunks(size).map(<[u8]>::to_vec).collect()
}

/// Mean absolute difference of the RGB channels, in 8-bit levels.
fn rgb_error(a: &[u8], b: &[u8]) -> f64 {
    let total: u64 = a
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .map(|(p, q)| (0..3).map(|k| u64::from(p[k].abs_diff(q[k]))).sum::<u64>())
        .sum();
    total as f64 / (a.len() / 4 * 3) as f64
}

fn half_second() -> Flicks {
    RATE.frames_to_flicks(FRAMES)
}

#[test]
fn every_prores_profile_writes_and_probes_with_its_name() {
    let profiles = [
        VideoCodec::ProResProxy,
        VideoCodec::ProResLt,
        VideoCodec::ProRes422,
        VideoCodec::ProResHq,
        VideoCodec::ProRes4444,
    ];
    let mut sizes = Vec::new();
    for codec in profiles {
        let Some(info) = video_encoder(codec) else {
            continue;
        };
        assert_eq!(info.encoder, "prores_ks");
        let settings = OutputSettings {
            container: Container::Mov,
            video: Some(video(codec)),
            audio: Some(audio(AudioCodec::Pcm24, 2)),
        };
        let name = format!("prores-{}.mov", codec.profile_name().unwrap());
        let (_, report) = write(&name, &settings, |n| gradient(n, false));
        assert!(Container::Mov.matches_probe(&report.container, report.brand.as_deref()));
        let got = report.video.as_ref().unwrap();
        let what = format!("{codec:?}");
        assert_eq!(got.codec, "prores", "{what}");
        assert_eq!(got.profile.as_deref(), codec.profile_name(), "{what}");
        assert_eq!((got.width, got.height), (WIDTH, HEIGHT), "{what}");
        assert_eq!(got.frame_count, FRAMES as u64, "{what}");
        assert_eq!(got.rate, Some(RATE), "{what}");
        assert_eq!(got.duration, Some(half_second()), "{what}");
        assert_eq!(
            got.pixel_format.starts_with("yuva"),
            codec.has_alpha(),
            "{what}: {}",
            got.pixel_format
        );
        let sound = report.audio.as_ref().unwrap();
        assert_eq!(
            (sound.codec.as_str(), sound.sample_rate, sound.channels),
            ("pcm_s24le", SAMPLE_RATE, 2),
            "{what}: 24-bit PCM in QuickTime"
        );
        assert_eq!(sound.duration, Some(half_second()), "{what}");
        sizes.push((codec, report.size_bytes));
    }
    if sizes.len() == profiles.len() {
        // Each profile spends more bits than the one before it.
        for pair in sizes.windows(2) {
            assert!(pair[0].1 < pair[1].1, "{sizes:?}");
        }
    }
}

#[test]
fn prores_4444_keeps_the_frames_alpha() {
    if video_encoder(VideoCodec::ProRes4444).is_none() {
        return;
    }
    let settings = OutputSettings {
        container: Container::Mov,
        video: Some(video(VideoCodec::ProRes4444)),
        audio: None,
    };
    let (path, report) = write("alpha.mov", &settings, |n| gradient(n, true));
    let got = report.video.unwrap();
    assert_eq!(got.profile.as_deref(), Some("4444"));
    assert!(got.pixel_format.starts_with("yuva"), "{}", got.pixel_format);
    assert!(report.audio.is_none());
    if !have_ffmpeg() {
        return;
    }
    let frames = decode_rgba(&path);
    assert_eq!(frames.len(), FRAMES as usize);
    let source = gradient(0, true);
    let mut left_alpha = 0u64;
    let mut right_alpha = 0u64;
    let mut opaque_error = 0.0;
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let at = (y * WIDTH as usize + x) * 4;
            let alpha = frames[0][at + 3];
            if x < WIDTH as usize / 2 {
                left_alpha = left_alpha.max(u64::from(alpha));
            } else {
                right_alpha = right_alpha.max(u64::from(255 - alpha));
                opaque_error += rgb_error(&frames[0][at..at + 4], &source.rgba[at..at + 4]);
            }
        }
    }
    let opaque_error = opaque_error / f64::from(WIDTH / 2 * HEIGHT);
    println!(
        "ProRes 4444: max alpha left {left_alpha}, max 255 - alpha right {right_alpha}, \
         opaque RGB error {opaque_error:.2}"
    );
    assert!(left_alpha <= 2, "the transparent half stays transparent");
    assert!(right_alpha <= 2, "the opaque half stays opaque");
    assert!(opaque_error < 4.0, "opaque colours survive: {opaque_error}");
}

#[test]
fn gifs_loop_forever_with_close_colours() {
    let settings = OutputSettings {
        container: Container::Gif,
        video: Some(video(VideoCodec::Gif)),
        audio: None,
    };
    assert_eq!(
        video_encoder(VideoCodec::Gif).map(|e| e.encoder.as_str()),
        Some("gif")
    );
    let (path, report) = write("moving.gif", &settings, |n| gradient(n, false));
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"GIF89a"));
    // The NETSCAPE2.0 application block with a loop count of 0: loop forever.
    let netscape = bytes
        .windows(11)
        .position(|w| w == b"NETSCAPE2.0")
        .expect("a looping GIF");
    assert_eq!(&bytes[netscape + 11..netscape + 15], &[3, 1, 0, 0]);
    assert!(Container::Gif.matches_probe(&report.container, report.brand.as_deref()));
    assert!(report.audio.is_none());
    let got = report.video.unwrap();
    assert_eq!(got.codec, "gif");
    assert_eq!((got.width, got.height), (WIDTH, HEIGHT));
    assert_eq!(got.frame_count, FRAMES as u64);
    // 12 frames at 24 fps last 50 centiseconds exactly.
    assert_eq!(got.rate, Some(RATE));
    assert_eq!(got.duration, Some(half_second()));
    println!("GIF: {} bytes for {FRAMES} frames", report.size_bytes);
    if !have_ffmpeg() {
        return;
    }
    let frames = decode_rgba(&path);
    assert_eq!(frames.len(), FRAMES as usize);
    let mut worst: f64 = 0.0;
    for (n, decoded) in frames.iter().enumerate() {
        let error = rgb_error(decoded, &gradient(n as i64, false).rgba);
        worst = worst.max(error);
        assert!(error < 12.0, "frame {n}: mean absolute error {error}");
    }
    println!("GIF: worst mean absolute error {worst:.2} of 255");
}

#[test]
fn gifs_take_yuv_frames_too() {
    let settings = OutputSettings {
        container: Container::Gif,
        video: Some(VideoSettings {
            rate: Rate::FPS_25,
            ..video(VideoCodec::Gif)
        }),
        audio: None,
    };
    let path = dir().join("gray.gif");
    let mut writer = FfmpegWriter::create(&path, &settings).unwrap();
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    // Limited-range grays 16 + 40 n, which are full-range RGB (40 n) * 255 / 219.
    for n in 0..5u8 {
        writer
            .push_frame(&VideoFrame {
                pts: Rate::FPS_25.frames_to_flicks(i64::from(n)),
                duration: Rate::FPS_25.frame_duration(),
                width: WIDTH,
                height: HEIGHT,
                format: PixelFormat::Nv12,
                planes: vec![
                    Plane {
                        stride: w,
                        rows: h,
                        data: Arc::from(vec![16 + 40 * n; w * h]),
                    },
                    Plane {
                        stride: w,
                        rows: h / 2,
                        data: Arc::from(vec![128u8; w * h / 2]),
                    },
                ],
                color: ColorInfo::default(),
            })
            .unwrap();
    }
    assert_eq!(writer.frames_written(), 5);
    writer.finish().unwrap();
    let got = probe_output(&path).unwrap().video.unwrap();
    assert_eq!((got.frame_count, got.rate), (5, Some(Rate::FPS_25)));
    if !have_ffmpeg() {
        return;
    }
    let frames = decode_rgba(&path);
    assert_eq!(frames.len(), 5);
    for (n, frame) in frames.iter().enumerate() {
        let want = (40.0 * n as f64 * 255.0 / 219.0).round();
        for pixel in frame.as_chunks::<4>().0 {
            for &v in &pixel[..3] {
                assert!(
                    (f64::from(v) - want).abs() <= 2.0,
                    "frame {n}: {v} for {want}"
                );
            }
        }
    }
}

/// Writes 0.5 s of the tone to `name` with `settings` and decodes it back.
fn round_trip(
    name: &str,
    settings: &AudioSettings,
    container: Container,
) -> (OutputReport, AudioBuffer) {
    let output = OutputSettings {
        container,
        video: None,
        audio: Some(settings.clone()),
    };
    let (path, report) = write(name, &output, |_| unreachable!("no video"));
    let mut decoder = FfmpegAudioDecoder::open(&path, SAMPLE_RATE, settings.channels).unwrap();
    let back = decoder.read(Flicks::ZERO, half_second()).unwrap();
    (report, back)
}

#[test]
fn lossless_audio_decodes_to_the_same_samples() {
    let cases = [
        ("tone-16.wav", AudioCodec::Pcm16, Container::Wav, 2, 16),
        ("tone-24.wav", AudioCodec::Pcm24, Container::Wav, 2, 24),
        ("tone-mono.wav", AudioCodec::Pcm16, Container::Wav, 1, 16),
        ("tone.flac", AudioCodec::Flac, Container::Flac, 2, 16),
    ];
    for (name, codec, container, channels, bits) in cases {
        assert!(codec.is_lossless());
        assert!(
            audio_encoder(codec).is_some(),
            "{codec:?} is built into FFmpeg"
        );
        // Lossless codecs ignore the bit rate, whatever it is.
        let bit_rate = match codec {
            AudioCodec::Pcm16 => 0,
            AudioCodec::Pcm24 => 1,
            _ => u64::MAX,
        };
        let settings = AudioSettings {
            bit_rate,
            ..audio(codec, channels)
        };
        let (report, back) = round_trip(name, &settings, container);
        assert!(container.matches_probe(&report.container, report.brand.as_deref()));
        assert!(report.video.is_none(), "{name}: audio only");
        let got = report.audio.as_ref().unwrap();
        assert_eq!(
            (got.codec.as_str(), got.sample_rate, got.channels),
            (codec.probe_name(), SAMPLE_RATE, channels),
            "{name}"
        );
        assert_eq!(got.duration, Some(half_second()), "{name}: 24000 samples");
        let source = tone(channels, 0.5);
        assert_eq!(back.samples.len(), source.samples.len(), "{name}");
        let step = 1.0 / f64::from(1u32 << (bits - 1));
        let worst = source
            .samples
            .iter()
            .zip(&back.samples)
            .map(|(a, b)| (f64::from(*a) - f64::from(*b)).abs())
            .fold(0.0, f64::max);
        println!(
            "{name}: worst error {:.3} of a {bits}-bit step",
            worst / step
        );
        assert!(
            worst <= step,
            "{name}: worst error {worst} is over one step {step}"
        );
    }
}

#[test]
fn mp3_is_written_with_lame_or_refused() {
    let settings = AudioSettings::stereo(AudioCodec::Mp3);
    let output = OutputSettings {
        container: Container::Mp3,
        video: None,
        audio: Some(settings.clone()),
    };
    let Some(info) = audio_encoder(AudioCodec::Mp3) else {
        let error = FfmpegWriter::create(&dir().join("missing.mp3"), &output)
            .err()
            .unwrap();
        assert_eq!(error.code(), "fosscut.media.encoder_unavailable");
        assert!(error.to_string().contains("mp3"), "{error}");
        return;
    };
    assert_eq!(info.encoder, "libmp3lame");
    let (report, back) = round_trip("tone.mp3", &settings, Container::Mp3);
    assert!(Container::Mp3.matches_probe(&report.container, report.brand.as_deref()));
    assert!(report.video.is_none());
    let got = report.audio.unwrap();
    assert_eq!(
        (got.codec.as_str(), got.sample_rate, got.channels),
        ("mp3", SAMPLE_RATE, 2)
    );
    let duration = got.duration.or(report.duration).unwrap();
    let frame = Flicks::from_seconds_f64(1152.0 / f64::from(SAMPLE_RATE)).unwrap();
    let off = (duration - half_second()).get().abs();
    println!(
        "MP3: duration {:.4} s, {} bytes",
        duration.get() as f64 / fosscut_time::FLICKS_PER_SECOND as f64,
        report.size_bytes
    );
    assert!(off <= frame.get(), "duration within one MP3 frame of 0.5 s");
    // The tone survives: its level on the left is 0.3 / sqrt(2) away from the edges.
    let left: Vec<f32> = back.samples.iter().step_by(2).copied().collect();
    let middle = &left[4_000..20_000];
    let rms =
        (middle.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / middle.len() as f64).sqrt();
    let want = 0.3 / 2f64.sqrt();
    assert!((rms - want).abs() < 0.02, "rms {rms}, expected {want}");
}

#[test]
fn unavailable_audio_codecs_are_refused_not_substituted() {
    for codec in AudioCodec::ALL
        .into_iter()
        .filter(|c| encoder_capabilities().audio(*c).is_none())
    {
        let container = match codec {
            AudioCodec::Opus => Container::Webm,
            AudioCodec::Mp3 => Container::Mp3,
            AudioCodec::Flac => Container::Flac,
            AudioCodec::Pcm16 | AudioCodec::Pcm24 => Container::Wav,
            AudioCodec::Aac => Container::Mp4,
        };
        let error = FfmpegWriter::create(
            &dir().join(format!("missing.{}", container.extension())),
            &OutputSettings {
                container,
                video: None,
                audio: Some(AudioSettings::stereo(codec)),
            },
        )
        .err()
        .unwrap();
        assert_eq!(
            error.code(),
            "fosscut.media.encoder_unavailable",
            "{codec:?}"
        );
    }
}

#[test]
fn containers_refuse_streams_they_cannot_hold() {
    let refused = [
        (Container::Gif, Some(VideoCodec::Gif), Some(AudioCodec::Aac)),
        (Container::Gif, Some(VideoCodec::ProRes422), None),
        (Container::Mp4, Some(VideoCodec::Gif), None),
        (Container::Mov, Some(VideoCodec::Gif), None),
        (Container::Mp4, Some(VideoCodec::ProResHq), None),
        (
            Container::Wav,
            Some(VideoCodec::Gif),
            Some(AudioCodec::Pcm16),
        ),
        (Container::Wav, None, Some(AudioCodec::Flac)),
        (Container::Flac, None, Some(AudioCodec::Pcm16)),
        (Container::Mp3, None, Some(AudioCodec::Aac)),
        (Container::Mp4, None, Some(AudioCodec::Pcm16)),
    ];
    for (container, video_codec, audio_codec) in refused {
        let error = FfmpegWriter::create(
            &dir().join(format!("refused.{}", container.extension())),
            &OutputSettings {
                container,
                video: video_codec.map(video),
                audio: audio_codec.map(AudioSettings::stereo),
            },
        )
        .err()
        .unwrap();
        assert_eq!(
            error.code(),
            "fosscut.media.unsupported",
            "{container:?} {video_codec:?} {audio_codec:?}"
        );
    }
    // A short RGBA frame is an error, not a crash.
    let settings = OutputSettings {
        container: Container::Gif,
        video: Some(video(VideoCodec::Gif)),
        audio: None,
    };
    let mut writer = FfmpegWriter::create(&dir().join("short.gif"), &settings).unwrap();
    let short = RgbaFrame {
        width: WIDTH,
        height: HEIGHT,
        rgba: vec![0; 16],
    };
    assert_eq!(
        writer.push_rgba(&short).unwrap_err().code(),
        "fosscut.media.unsupported"
    );
    writer.push_rgba(&gradient(0, false)).unwrap();
    writer.finish().unwrap();
}

/// Frame `n` of a 320x180 moving gradient with deterministic grain of +-24 levels:
/// detailed enough that encoders need every bit of a 1 Mb/s target, simple enough that
/// they can reach 100 kb/s.
fn textured(n: i64) -> RgbaFrame {
    let (width, height) = (320u32, 180u32);
    let mut state = 0x9E37_79B9_7F4A_7C15u64 ^ (n as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let grain = state.to_le_bytes();
            let t = (x + y) as f32 / 18.0 + n as f32 * 0.5;
            let base = [
                (x * 255 / (width - 1)) as i32,
                (y * 255 / (height - 1)) as i32,
                (128.0 + 100.0 * t.sin()) as i32,
            ];
            for k in 0..3 {
                rgba.push((base[k] + i32::from(grain[k]) * 24 / 128 - 24).clamp(0, 255) as u8);
            }
            rgba.push(255);
        }
    }
    RgbaFrame {
        width,
        height,
        rgba,
    }
}

#[test]
fn video_bit_rates_are_honoured_where_codecs_take_one() {
    for codec in VideoCodec::ALL {
        let Some(info) = video_encoder(codec) else {
            continue;
        };
        // Two seconds give rate control time to settle; codecs without it need less.
        let frames = if codec.uses_bit_rate() { 48 } else { 6 };
        let seconds = frames as f64 / 24.0;
        let container = match codec {
            VideoCodec::Gif => Container::Gif,
            VideoCodec::Vp9 => Container::Webm,
            codec if codec.probe_name() == "prores" => Container::Mov,
            _ => Container::Mp4,
        };
        let mut sizes = Vec::new();
        for bit_rate in [None, Some(100_000), Some(1_000_000)] {
            let settings = OutputSettings {
                container,
                video: Some(VideoSettings {
                    width: 320,
                    height: 180,
                    bit_rate,
                    ..video(codec)
                }),
                audio: None,
            };
            let path = dir().join(format!(
                "rate-{codec:?}-{}.{}",
                bit_rate.unwrap_or(0),
                container.extension()
            ));
            let mut writer = FfmpegWriter::create(&path, &settings).unwrap();
            for n in 0..frames {
                writer.push_rgba(&textured(n)).unwrap();
            }
            writer.finish().unwrap();
            let report = probe_output(&path).unwrap();
            assert_eq!(report.video.unwrap().frame_count, frames as u64);
            sizes.push(report.size_bytes);
        }
        let what = format!("{codec:?} with {}", info.encoder);
        let rate = |bytes: u64| bytes as f64 * 8.0 / seconds;
        println!(
            "{what}: default {:.0} b/s, 100 kb/s target {:.0} b/s, 1 Mb/s target {:.0} b/s",
            rate(sizes[0]),
            rate(sizes[1]),
            rate(sizes[2])
        );
        if codec.uses_bit_rate() {
            assert!(
                sizes[2] > 2 * sizes[1],
                "{what}: ten times the bit rate, sizes {sizes:?}"
            );
            // Windows' Media Foundation H.264 encoder has a quality floor: on this per-frame
            // noise it gives about 445 kb/s for the 100 kb/s target and for its own default
            // (about 276 kb/s here), in constant bit rate mode, which is its default. It
            // must not exceed its default there; every other encoder must reach the target.
            if info.encoder.ends_with("_mf") {
                assert!(
                    sizes[1] <= sizes[0] + sizes[0] / 20,
                    "{what}: 100 kb/s target gave {} b/s, more than its default {} b/s",
                    rate(sizes[1]),
                    rate(sizes[0])
                );
            } else {
                assert!(
                    rate(sizes[1]) < 300_000.0,
                    "{what}: 100 kb/s target gave {} b/s",
                    rate(sizes[1])
                );
            }
            assert!(
                (500_000.0..3_000_000.0).contains(&rate(sizes[2])),
                "{what}: 1 Mb/s target gave {} b/s",
                rate(sizes[2])
            );
        } else {
            assert!(
                sizes.iter().all(|s| *s == sizes[0]),
                "{what}: no bit rate control, so the same file every time: {sizes:?}"
            );
        }
    }
}
