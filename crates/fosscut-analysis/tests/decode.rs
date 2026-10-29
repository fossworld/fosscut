//! The analyzers on files FFmpeg generates with known answers (ADR-0019, validation): a
//! calibrated tone, a tone with silent gaps, and shots joined at known frames.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use fosscut_analysis::decode::{Media, analyze};
use fosscut_analysis::{AnalysisError, Analyzer, Options, Progress};
use fosscut_time::{Flicks, Rate};

fn has_ffmpeg() -> bool {
    let present = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        present || !required,
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    present
}

fn ffmpeg(dir: &Path, args: &[&str]) {
    let status = Command::new("ffmpeg")
        .current_dir(dir)
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
}

/// The generated fixtures, made once per test run.
fn fixtures() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-analysis-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 5 s of a 997 Hz sine at -20 dBFS, mono, lossless.
        ffmpeg(
            &dir,
            &["-f", "lavfi", "-i", "aevalsrc=0.1*sin(2*PI*997*t):s=48000:d=5", "-c:a", "pcm_s16le", "tone.wav"],
        );
        // A 440 Hz tone with silent gaps at 1.0-1.5 s and 3.0-3.6 s.
        ffmpeg(
            &dir,
            &[
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=if(between(t\\,1\\,1.5)+between(t\\,3\\,3.6)\\,0\\,0.25*sin(2*PI*440*t)):s=48000:d=5",
                "-c:a",
                "pcm_s16le",
                "gaps.wav",
            ],
        );
        // Three one-second shots at 30 fps: dark grey, light grey, test pattern.
        ffmpeg(
            &dir,
            &[
                "-filter_complex",
                "color=c=0x202020:s=320x180:r=30:d=1[a];color=c=0xE0E0E0:s=320x180:r=30:d=1[b];testsrc2=s=320x180:r=30:d=1[c];[a][b][c]concat=n=3:v=1:a=0[v]",
                "-map",
                "[v]",
                "-c:v",
                "mpeg4",
                "-q:v",
                "2",
                "shots.mp4",
            ],
        );
        dir
    })
}

fn seconds(n: i64) -> Flicks {
    Flicks::from_seconds(n).unwrap()
}

fn audio(name: &str, channels: u16) -> Media<'static> {
    let path: &'static Path = Box::leak(fixtures().join(name).into_boxed_path());
    Media {
        path,
        duration: seconds(5),
        audio_channels: Some(channels),
        video_rate: None,
    }
}

/// Analyzers without a model never look in the models folder.
fn no_models() -> &'static std::path::Path {
    std::path::Path::new("no-models")
}

fn run(
    media: Media<'_>,
    analyzers: &[Analyzer],
) -> Result<fosscut_analysis::Results, AnalysisError> {
    analyze(
        media,
        analyzers,
        &Options::default(),
        no_models(),
        &mut |_| true,
    )
}

#[test]
fn a_calibrated_tone_measures_its_loudness() {
    if !has_ffmpeg() {
        return;
    }
    let mut seen = Vec::new();
    let results = analyze(
        audio("tone.wav", 1),
        &[Analyzer::Loudness],
        &Options::default(),
        no_models(),
        &mut |p: Progress| {
            seen.push(p);
            true
        },
    )
    .unwrap();
    let loudness = results.loudness.unwrap();
    assert!(
        (loudness.integrated_lufs.unwrap() + 23.01).abs() < 0.1,
        "{loudness:?}"
    );
    assert!(
        (loudness.sample_peak_dbfs.unwrap() + 20.0).abs() < 0.1,
        "{loudness:?}"
    );
    assert!(results.silences.is_none() && results.scenes.is_none());
    assert_eq!(seen.first(), Some(&Progress { done: 1, total: 5 }));
    assert_eq!(seen.last(), Some(&Progress { done: 5, total: 5 }));
}

#[test]
fn silent_gaps_are_found_at_their_times() {
    if !has_ffmpeg() {
        return;
    }
    let results = run(
        audio("gaps.wav", 1),
        &[Analyzer::Silences, Analyzer::Loudness],
    )
    .unwrap();
    let ranges: Vec<(String, String)> = results
        .silences
        .unwrap()
        .ranges
        .into_iter()
        .map(|r| (r.start.seconds, r.end.seconds))
        .collect();
    assert_eq!(
        ranges,
        [("1".into(), "1.5".into()), ("3".into(), "3.6".into())]
    );
    assert!(results.loudness.unwrap().integrated_lufs.is_some());
}

#[test]
fn scene_cuts_land_on_the_joined_frames() {
    if !has_ffmpeg() {
        return;
    }
    let path = fixtures().join("shots.mp4");
    let media = Media {
        path: &path,
        duration: seconds(3),
        audio_channels: None,
        video_rate: Some(Rate::FPS_30),
    };
    let scenes = run(media, &[Analyzer::Scenes]).unwrap().scenes.unwrap();
    let cuts: Vec<(u64, String)> = scenes
        .cuts
        .iter()
        .map(|c| (c.frame, c.time.seconds.clone()))
        .collect();
    assert_eq!(cuts, [(30, "1".to_string()), (60, "2".to_string())]);
    assert_eq!(scenes.frames, 90);
}

#[test]
fn missing_streams_and_cancelling_are_typed() {
    if !has_ffmpeg() {
        return;
    }
    // Scenes on audio, as the media table says (no rate) and as the file says (a rate given).
    let no_rate = run(audio("tone.wav", 1), &[Analyzer::Scenes]).unwrap_err();
    assert!(matches!(no_rate, AnalysisError::NoVideo), "{no_rate:?}");
    let mut claims_video = audio("tone.wav", 1);
    claims_video.video_rate = Some(Rate::FPS_30);
    let not_there = run(claims_video, &[Analyzer::Scenes]).unwrap_err();
    assert!(matches!(not_there, AnalysisError::NoVideo), "{not_there:?}");

    // Loudness on video without audio.
    let shots = fixtures().join("shots.mp4");
    let silent_video = Media {
        path: &shots,
        duration: seconds(3),
        audio_channels: None,
        video_rate: Some(Rate::FPS_30),
    };
    let no_audio = run(silent_video, &[Analyzer::Loudness]).unwrap_err();
    assert!(matches!(no_audio, AnalysisError::NoAudio), "{no_audio:?}");
    let claims_audio = Media {
        audio_channels: Some(2),
        ..silent_video
    };
    let absent = run(claims_audio, &[Analyzer::Loudness]).unwrap_err();
    assert!(matches!(absent, AnalysisError::NoAudio), "{absent:?}");

    // Stopping through the progress callback.
    let stopped = analyze(
        audio("gaps.wav", 1),
        &[Analyzer::Silences],
        &Options::default(),
        no_models(),
        &mut |_| false,
    )
    .unwrap_err();
    assert!(matches!(stopped, AnalysisError::Cancelled));
    let stopped_video = analyze(
        silent_video,
        &[Analyzer::Scenes],
        &Options::default(),
        no_models(),
        &mut |_| false,
    )
    .unwrap_err();
    assert!(matches!(stopped_video, AnalysisError::Cancelled));

    // A file that is not media.
    let bogus = fixtures().join("bogus.wav");
    std::fs::write(&bogus, b"not audio").unwrap();
    let media = Media {
        path: &bogus,
        ..audio("tone.wav", 1)
    };
    assert!(matches!(
        run(media, &[Analyzer::Loudness]).unwrap_err(),
        AnalysisError::Media(_)
    ));
}
