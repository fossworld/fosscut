//! The `beats` and `levels` analyzers on a file FFmpeg generates with known answers: a
//! stereo click track at 120 bpm, louder in its second half, read through the same 48 kHz
//! decoding as loudness and silences.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

use fosscut_analysis::decode::{Media, analyze};
use fosscut_analysis::{Analyzer, Options};
use fosscut_time::{FLICKS_PER_SECOND, Flicks};

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

#[test]
fn a_click_track_file_gives_its_beats_and_levels() {
    if !has_ffmpeg() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("fosscut-beats-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A 5 ms decaying 2 kHz click every half second from 0.25 s, twice as loud after 5 s,
    // in both channels.
    let click = "exp(-mod(t-0.25\\,0.5)/0.0015)*gte(t\\,0.25)*sin(2*PI*2000*t)";
    let expression = format!("(0.3+0.3*gte(t\\,5))*{click}|(0.3+0.3*gte(t\\,5))*{click}");
    let status = Command::new("ffmpeg")
        .current_dir(&dir)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("aevalsrc={expression}:s=48000:d=10"))
        .args(["-c:a", "pcm_s16le", "clicks.wav"])
        .status()
        .unwrap();
    assert!(status.success());
    let path = dir.join("clicks.wav");
    let media = Media {
        path: &path,
        duration: Flicks::new(10 * FLICKS_PER_SECOND),
        audio_channels: Some(2),
        video_rate: None,
    };
    let mut reports = 0;
    let results = analyze(
        media,
        &[Analyzer::Beats, Analyzer::Levels],
        &Options::default(),
        &dir,
        &mut |_| {
            reports += 1;
            true
        },
    )
    .unwrap();
    assert!(reports >= 10, "progress every second");
    let beats = results.beats.unwrap();
    let tempo = beats.tempo_bpm.unwrap();
    assert!((tempo - 120.0).abs() < 1.0, "tempo {tempo}");
    assert_eq!(beats.beats.len(), 20);
    for (k, beat) in beats.beats.iter().enumerate() {
        let want = 0.25 + 0.5 * k as f64;
        let got = beat.time.flicks.as_seconds_f64();
        assert!((got - want).abs() < 0.005, "beat {k} at {got} for {want}");
    }
    let levels = results.levels.unwrap();
    assert_eq!(levels.window_ms, 1000);
    assert_eq!(levels.lufs.len(), 10);
    let quiet = levels.lufs[2].unwrap();
    let loud = levels.lufs[7].unwrap();
    assert!((loud - quiet - 6.0).abs() < 0.3, "{quiet} then {loud}");
    // Their options are part of what they report and of the cache key.
    let mut options = Options::default();
    options.levels.window_ms = 500;
    options.beats.bpm = Some(120.0);
    assert_ne!(
        options.for_analyzer(Analyzer::Levels),
        Options::default().for_analyzer(Analyzer::Levels)
    );
    let results = analyze(media, &[Analyzer::Levels], &options, &dir, &mut |_| true).unwrap();
    assert_eq!(results.levels.unwrap().lufs.len(), 20);
    assert!(results.beats.is_none() && results.loudness.is_none());
}
