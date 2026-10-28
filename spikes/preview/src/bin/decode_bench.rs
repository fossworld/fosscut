//! Decode budget for the preview spike: sequential and random-access decoding of the 4K
//! source with VideoToolbox and in software, including the copy to system memory.

use std::time::Instant;

use fosscut_media::ffmpeg::FfmpegVideoDecoder;
use fosscut_media::{DecoderOptions, HardwareDecoding, VideoDecoder};
use fosscut_time::Rate;

fn percentile(values: &mut [f64], q: f64) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    values[((values.len() - 1) as f64 * q).round() as usize]
}

fn main() {
    let source = std::env::args().nth(1).unwrap_or_else(|| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/generated/test-pattern-4k.mp4").into()
    });
    let mut report = serde_json::Map::new();
    for (name, hardware) in [("videotoolbox", HardwareDecoding::Auto), ("software", HardwareDecoding::Off)] {
        let options = DecoderOptions { hardware };
        let mut decoder = FfmpegVideoDecoder::open(source.as_ref(), options).expect("open");
        let start = Instant::now();
        let mut per_frame = Vec::new();
        let mut last = Instant::now();
        while let Some(_frame) = decoder.next_frame().expect("decode") {
            per_frame.push(last.elapsed().as_secs_f64() * 1000.0);
            last = Instant::now();
        }
        let seconds = start.elapsed().as_secs_f64();
        let count = per_frame.len();
        let accel = format!("{:?}", decoder.acceleration());

        let mut decoder = FfmpegVideoDecoder::open(source.as_ref(), options).expect("open");
        let rate = decoder.info().rate.unwrap_or(Rate::FPS_30);
        let mut seed: u64 = 42;
        let mut seeks = Vec::new();
        for _ in 0..30 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let frame = (seed >> 33) as i64 % (count as i64 - 1);
            let t = Instant::now();
            decoder.frame_at(rate.frames_to_flicks(frame)).expect("seek");
            seeks.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        // Stepping backwards one frame at a time: the worst case for long-GOP sources.
        let mut backward = Vec::new();
        for frame in (200..230).rev() {
            let t = Instant::now();
            decoder.frame_at(rate.frames_to_flicks(frame)).expect("seek");
            backward.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        report.insert(
            name.into(),
            serde_json::json!({
                "acceleration": accel,
                "frames": count,
                "sequential_fps": (count as f64 / seconds * 10.0).round() / 10.0,
                "sequential_ms_p50": percentile(&mut per_frame, 0.5),
                "sequential_ms_p95": percentile(&mut per_frame, 0.95),
                "random_seek_ms_p50": percentile(&mut seeks, 0.5),
                "random_seek_ms_p95": percentile(&mut seeks, 0.95),
                "backward_step_ms_p50": percentile(&mut backward, 0.5),
                "backward_step_ms_p95": percentile(&mut backward, 0.95),
            }),
        );
    }
    println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
}
