//! Exact conversions between stream time bases and flicks.

use fosscut_time::{FLICKS_PER_SECOND, Flicks, Rate};

/// Converts a timestamp in a stream time base (`num / den` seconds per tick) to flicks,
/// rounding to the nearest flick. Exact for every time base whose denominator divides
/// 705,600,000, which covers the common video and audio time bases.
pub fn to_flicks(ticks: i64, num: i32, den: i32) -> Flicks {
    if den == 0 {
        return Flicks::ZERO;
    }
    let numerator = i128::from(ticks) * i128::from(num) * i128::from(FLICKS_PER_SECOND);
    Flicks::new(saturate(div_round(numerator, i128::from(den))))
}

/// Converts flicks to a stream time base, rounding down, so that seeking to the result
/// never lands after the requested time.
pub fn from_flicks(time: Flicks, num: i32, den: i32) -> i64 {
    if num == 0 {
        return 0;
    }
    let numerator = i128::from(time.get()) * i128::from(den);
    let denominator = i128::from(num) * i128::from(FLICKS_PER_SECOND);
    saturate(numerator.div_euclid(denominator))
}

/// An exact frame rate from a rational, or `None` when it is zero or invalid.
pub fn rate(num: i32, den: i32) -> Option<Rate> {
    let num = u32::try_from(num).ok()?;
    let den = u32::try_from(den).ok()?;
    Rate::new(num, den).ok()
}

fn div_round(numerator: i128, denominator: i128) -> i128 {
    let (numerator, denominator) = if denominator < 0 {
        (-numerator, -denominator)
    } else {
        (numerator, denominator)
    };
    (2 * numerator + denominator).div_euclid(2 * denominator)
}

fn saturate(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn common_time_bases_are_exact() {
        // MP4 at 30000/1001: frame n has pts n * 1001 in a 1/30000 time base.
        let rate = Rate::new(30_000, 1_001).unwrap();
        for n in [0, 1, 118, 10_000] {
            assert_eq!(to_flicks(n * 1_001, 1, 30_000), rate.frames_to_flicks(n));
        }
        assert_eq!(
            to_flicks(90_000, 1, 90_000),
            Flicks::from_seconds(1).unwrap()
        );
        assert_eq!(
            to_flicks(48_000, 1, 48_000),
            Flicks::from_seconds(1).unwrap()
        );
        assert_eq!(to_flicks(-1_001, 1, 30_000), -rate.frames_to_flicks(1));
        assert_eq!(to_flicks(5, 1, 0), Flicks::ZERO);
    }

    #[test]
    fn seeking_targets_never_land_late() {
        assert_eq!(
            from_flicks(Flicks::from_seconds(1).unwrap(), 1, 90_000),
            90_000
        );
        assert_eq!(from_flicks(Flicks::new(1), 1, 30), 0);
        assert_eq!(from_flicks(Flicks::new(-1), 1, 30), -1);
        assert_eq!(from_flicks(Flicks::new(5), 0, 30), 0);
    }

    #[test]
    fn rates_must_be_positive() {
        assert_eq!(rate(30, 1), Some(Rate::FPS_30));
        assert_eq!(rate(0, 1), None);
        assert_eq!(rate(30, 0), None);
        assert_eq!(rate(-30, 1), None);
    }

    proptest! {
        #[test]
        fn round_trips_within_one_tick(ticks in -1_000_000_000i64..1_000_000_000, den in prop::sample::select(vec![24, 25, 30, 1000, 12_800, 15_360, 30_000, 90_000])) {
            let back = from_flicks(to_flicks(ticks, 1, den), 1, den);
            prop_assert!((back - ticks).abs() <= 1);
        }
    }
}
