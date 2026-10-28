//! Property tests for the time model (ADR-0005, validation section).

#![allow(clippy::unwrap_used)]

use fosscut_time::{Flicks, Rate, TimeInput, format_timecode, parse_timecode};
use proptest::prelude::*;

fn supported_rates() -> Vec<Rate> {
    let mut rates: Vec<Rate> = [24, 25, 30, 48, 50, 60, 90, 100, 120]
        .into_iter()
        .map(|n| Rate::new(n, 1).unwrap())
        .collect();
    rates.extend([Rate::NTSC_24, Rate::NTSC_30, Rate::NTSC_60]);
    rates
}

fn rate_strategy() -> impl Strategy<Value = Rate> {
    proptest::sample::select(supported_rates())
}

proptest! {
    #[test]
    fn frames_to_flicks_round_trips(rate in rate_strategy(), frames in -10_000_000i64..10_000_000) {
        let time = rate.frames_to_flicks(frames);
        prop_assert_eq!(rate.frame_at(time), frames);
        prop_assert!(rate.is_on_frame(time));
        prop_assert_eq!(rate.snap(time), time);
    }

    #[test]
    fn frame_steps_are_constant(rate in rate_strategy(), frames in 0i64..10_000_000) {
        let step = rate.frame_duration();
        prop_assert_eq!(rate.frames_to_flicks(frames + 1) - rate.frames_to_flicks(frames), step);
    }

    #[test]
    fn split_then_join_is_exact(rate in rate_strategy(), length in 2i64..100_000, cut in 1i64..100_000) {
        let cut = cut % (length - 1) + 1;
        let start = rate.frames_to_flicks(0);
        let end = rate.frames_to_flicks(length);
        let split = rate.frames_to_flicks(cut);
        prop_assert_eq!((split - start) + (end - split), end - start);
    }

    #[test]
    fn timecode_round_trips(rate in rate_strategy(), frames in 0i64..(24 * 3600 * 60)) {
        let text = format_timecode(frames, rate);
        prop_assert_eq!(parse_timecode(&text, rate).unwrap(), frames, "{}", text);
    }

    #[test]
    fn seconds_text_round_trips_to_the_microsecond(micros in -1_000_000_000_000i64..1_000_000_000_000) {
        let text = format!("{}.{:06}", micros / 1_000_000, (micros % 1_000_000).abs());
        let text = if micros < 0 && micros > -1_000_000 { format!("-{text}") } else { text };
        let time = Flicks::parse_seconds(&text).unwrap();
        prop_assert_eq!(Flicks::parse_seconds(&time.to_seconds_string()).unwrap(), time);
    }
}

#[test]
fn round_one_split_lands_exactly_on_frame_118() {
    let rate = Rate::FPS_30;
    let at: TimeInput = "00:00:03:28".parse().unwrap();
    let time = at.resolve(rate).unwrap();
    assert_eq!(rate.frame_at(time), 118);
    assert!(rate.is_on_frame(time));
    let remainder = Flicks::from_seconds(8).unwrap() - time;
    assert_eq!(rate.frame_at(remainder), 240 - 118);
    assert!(rate.is_on_frame(remainder));
}
