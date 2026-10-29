//! Quiet ranges: the level of 50 ms windows of the mono mix, compared with a threshold.
//! Runs of quiet windows at least as long as the minimum duration are reported.

use fosscut_time::{FLICKS_PER_SECOND, Flicks};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Moment, Span};

/// The sample rate the finder expects.
pub const RATE: u32 = 48_000;
/// Samples per window: 50 ms.
const WINDOW: u64 = (RATE / 20) as u64;
/// Flicks per sample at 48 kHz.
const FLICKS_PER_SAMPLE: i64 = FLICKS_PER_SECOND / RATE as i64;

/// Options of the `silences` analyzer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SilenceOptions {
    /// A window is quiet when its RMS level is below this, in dBFS (default -40).
    #[serde(default = "default_threshold")]
    pub threshold_dbfs: f64,
    /// Shortest quiet run reported, in milliseconds (default 300).
    #[serde(default = "default_min_duration")]
    pub min_duration_ms: u32,
}

fn default_threshold() -> f64 {
    -40.0
}

fn default_min_duration() -> u32 {
    300
}

impl Default for SilenceOptions {
    fn default() -> SilenceOptions {
        SilenceOptions {
            threshold_dbfs: default_threshold(),
            min_duration_ms: default_min_duration(),
        }
    }
}

/// Quiet ranges of a stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Silences {
    /// The threshold used, in dBFS.
    pub threshold_dbfs: f64,
    /// The shortest run reported, in milliseconds.
    pub min_duration_ms: u32,
    /// Each quiet range, in order; edges fall on 50 ms windows.
    pub ranges: Vec<Span>,
    /// Their total length.
    pub total: Moment,
}

/// Streams interleaved 48 kHz samples and reports quiet ranges.
#[derive(Clone, Debug)]
pub struct SilenceFinder {
    options: SilenceOptions,
    channels: usize,
    /// Mean square below which a window is quiet.
    floor: f64,
    window_start: u64,
    acc: f64,
    count: u64,
    run_start: Option<u64>,
    ranges: Vec<(u64, u64)>,
}

impl SilenceFinder {
    /// A finder for `channels` interleaved channels.
    pub fn new(channels: u16, options: SilenceOptions) -> SilenceFinder {
        SilenceFinder {
            floor: 10f64.powf(options.threshold_dbfs / 10.0),
            options,
            channels: usize::from(channels.max(1)),
            window_start: 0,
            acc: 0.0,
            count: 0,
            run_start: None,
            ranges: Vec::new(),
        }
    }

    /// Adds interleaved samples; a trailing partial frame is ignored.
    pub fn push(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channels) {
            let mono = frame.iter().map(|&s| f64::from(s)).sum::<f64>() / frame.len() as f64;
            self.acc += mono * mono;
            self.count += 1;
            if self.count == WINDOW {
                self.close_window();
            }
        }
    }

    fn close_window(&mut self) {
        let quiet = self.acc / (self.count as f64) < self.floor;
        let end = self.window_start + self.count;
        match (quiet, self.run_start) {
            (true, None) => self.run_start = Some(self.window_start),
            (false, Some(start)) => self.end_run(start, self.window_start),
            _ => {}
        }
        self.window_start = end;
        self.acc = 0.0;
        self.count = 0;
    }

    fn end_run(&mut self, start: u64, end: u64) {
        let min = u64::from(self.options.min_duration_ms) * u64::from(RATE) / 1000;
        if end - start >= min {
            self.ranges.push((start, end));
        }
        self.run_start = None;
    }

    /// The quiet ranges of everything pushed.
    pub fn finish(mut self) -> Silences {
        if self.count > 0 {
            self.close_window();
        }
        if let Some(start) = self.run_start {
            self.end_run(start, self.window_start);
        }
        let flicks = |samples: u64| Flicks::new(samples as i64 * FLICKS_PER_SAMPLE);
        let total: u64 = self.ranges.iter().map(|(s, e)| e - s).sum();
        Silences {
            threshold_dbfs: self.options.threshold_dbfs,
            min_duration_ms: self.options.min_duration_ms,
            ranges: self
                .ranges
                .iter()
                .map(|&(s, e)| Span::new(flicks(s), flicks(e)))
                .collect(),
            total: Moment::new(flicks(total)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f64) -> Vec<f32> {
        let n = (seconds * f64::from(RATE)) as usize;
        (0..n).map(|i| 0.25 * (i as f32 * 0.05).sin()).collect()
    }

    fn quiet(seconds: f64, level: f32) -> Vec<f32> {
        vec![level; (seconds * f64::from(RATE)) as usize]
    }

    fn seconds(span: &Span) -> (String, String) {
        (span.start.seconds.clone(), span.end.seconds.clone())
    }

    #[test]
    fn finds_quiet_runs_longer_than_the_minimum() {
        let mut signal = tone(1.0);
        signal.extend(quiet(0.5, 0.0)); // a pause
        signal.extend(tone(1.0));
        signal.extend(quiet(0.2, 0.0)); // too short to report
        signal.extend(tone(1.0));
        signal.extend(quiet(0.6, 0.001)); // -60 dBFS room tone at the end
        let mut finder = SilenceFinder::new(1, SilenceOptions::default());
        for chunk in signal.chunks(1_234) {
            finder.push(chunk);
        }
        let found = finder.finish();
        let ranges: Vec<_> = found.ranges.iter().map(seconds).collect();
        assert_eq!(
            ranges,
            [("1".into(), "1.5".into()), ("3.7".into(), "4.3".into())]
        );
        assert_eq!(found.total.seconds, "1.1");
        assert_eq!(found.threshold_dbfs, -40.0);
    }

    #[test]
    fn options_change_what_counts() {
        let mut signal = tone(0.5);
        signal.extend(quiet(0.5, 0.005)); // -46 dBFS
        signal.extend(tone(0.5));
        let strict = SilenceOptions {
            threshold_dbfs: -50.0,
            min_duration_ms: 300,
        };
        let mut finder = SilenceFinder::new(1, strict);
        finder.push(&signal);
        assert!(finder.finish().ranges.is_empty(), "-46 dBFS is above -50");

        let short = SilenceOptions {
            threshold_dbfs: -40.0,
            min_duration_ms: 600,
        };
        let mut finder = SilenceFinder::new(1, short);
        finder.push(&signal);
        assert!(
            finder.finish().ranges.is_empty(),
            "0.5 s is shorter than 0.6 s"
        );
    }

    #[test]
    fn stereo_is_mixed_and_a_silent_stream_is_one_range() {
        // Opposite channels cancel in the mono mix.
        let stereo: Vec<f32> = tone(1.0).iter().flat_map(|&s| [s, -s]).collect();
        let mut finder = SilenceFinder::new(2, SilenceOptions::default());
        finder.push(&stereo);
        let found = finder.finish();
        assert_eq!(found.ranges.len(), 1);
        assert_eq!(found.ranges[0].end.seconds, "1");

        let finder = SilenceFinder::new(1, SilenceOptions::default());
        assert!(finder.finish().ranges.is_empty(), "nothing pushed");
        let defaults: SilenceOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, SilenceOptions::default());
    }
}
