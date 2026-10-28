//! Edge cases of the time API: overflow, malformed inputs and the error codes agents see.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use fosscut_time::{FLICKS_PER_SECOND, Flicks, Rate, TimeError, TimeInput};
use serde_json::json;

#[test]
fn flick_arithmetic_is_exact_and_checked() {
    let one = Flicks::from_seconds(1).unwrap();
    assert_eq!(one.get(), FLICKS_PER_SECOND);
    assert_eq!(Flicks::new(5).get(), 5);
    assert_eq!(one + one, Flicks::from_seconds(2).unwrap());
    assert_eq!(one - one, Flicks::ZERO);
    assert_eq!(-one, Flicks::new(-FLICKS_PER_SECOND));
    assert!((-one).is_negative() && one.is_positive());
    assert!(!Flicks::ZERO.is_positive() && !Flicks::ZERO.is_negative());
    assert_eq!(
        one.checked_add(one),
        Some(Flicks::new(2 * FLICKS_PER_SECOND))
    );
    assert_eq!(Flicks::new(i64::MAX).checked_add(one), None);
    assert_eq!(Flicks::new(i64::MIN).checked_sub(one), None);
    assert_eq!(Flicks::from_seconds(i64::MAX), None);
    assert!((Flicks::new(FLICKS_PER_SECOND / 2).as_seconds_f64() - 0.5).abs() < 1e-12);
}

#[test]
fn float_seconds_round_and_reject_non_finite_values() {
    assert_eq!(
        Flicks::from_seconds_f64(1.5).unwrap(),
        Flicks::new(FLICKS_PER_SECOND * 3 / 2)
    );
    for bad in [f64::NAN, f64::INFINITY] {
        assert_eq!(
            Flicks::from_seconds_f64(bad).unwrap_err().code(),
            "fosscut.time.invalid_seconds"
        );
    }
    assert_eq!(
        Flicks::from_seconds_f64(1e30).unwrap_err(),
        TimeError::Overflow
    );
}

#[test]
fn malformed_seconds_text_is_rejected_with_a_code() {
    for bad in ["1.2.3", "abc", "1e5", "1234567890123456789.0"] {
        let error = Flicks::parse_seconds(bad).unwrap_err();
        assert!(
            error.code() == "fosscut.time.invalid_seconds" || error == TimeError::Overflow,
            "{bad}: {error:?}"
        );
    }
    assert_eq!(
        Flicks::parse_seconds("-0.5").unwrap(),
        Flicks::new(-FLICKS_PER_SECOND / 2)
    );
    assert_eq!(
        Flicks::parse_seconds(".25").unwrap(),
        Flicks::new(FLICKS_PER_SECOND / 4)
    );
}

#[test]
fn inputs_display_in_their_command_line_form() {
    let cases = [
        (TimeInput::Frames(118), "118f"),
        (TimeInput::Seconds("3.5".into()), "3.5s"),
        (TimeInput::Timecode("00:00:03:28".into()), "00:00:03:28"),
        (TimeInput::Flicks(42), "42fl"),
    ];
    for (input, shown) in cases {
        assert_eq!(input.to_string(), shown);
        assert_eq!(shown.parse::<TimeInput>().unwrap(), input);
    }
    assert_eq!(
        "12xf".parse::<TimeInput>().unwrap_err().code(),
        "fosscut.time.unit_required"
    );
    assert_eq!(
        "12xfl".parse::<TimeInput>().unwrap_err().code(),
        "fosscut.time.unit_required"
    );
}

#[test]
fn json_inputs_need_exactly_one_known_unit() {
    let parse = |value: serde_json::Value| serde_json::from_value::<TimeInput>(value);
    assert_eq!(
        parse(json!({ "seconds": 1.5 })).unwrap(),
        TimeInput::Seconds("1.5".into())
    );
    assert_eq!(
        parse(json!({ "seconds": 2 })).unwrap(),
        TimeInput::Seconds("2".into())
    );
    assert_eq!(parse(json!("90f")).unwrap(), TimeInput::Frames(90));
    for (bad, expected) in [
        (json!({}), "empty time object"),
        (json!({ "minutes": 1 }), "unknown field"),
        (json!({ "frames": 1, "seconds": "1" }), "exactly one unit"),
        (json!({ "seconds": "1.2.3" }), "invalid seconds"),
        (json!(1.5), "explicit unit"),
        (json!(-3), "explicit unit"),
        (json!(true), "explicit unit"),
    ] {
        let error = parse(bad.clone()).unwrap_err().to_string();
        assert!(error.contains(expected), "{bad}: {error}");
    }
}

#[test]
fn every_error_has_a_stable_code() {
    let rate = Rate::FPS_30;
    let errors = [
        (
            "0".parse::<Rate>().unwrap_err(),
            "fosscut.time.invalid_rate",
        ),
        (
            TimeInput::Timecode("99:99".into())
                .resolve(rate)
                .unwrap_err(),
            "fosscut.time.invalid_timecode",
        ),
        (
            "12".parse::<TimeInput>().unwrap_err(),
            "fosscut.time.unit_required",
        ),
        (
            Flicks::parse_seconds("x").unwrap_err(),
            "fosscut.time.invalid_seconds",
        ),
        (TimeError::Overflow, "fosscut.time.overflow"),
    ];
    for (error, code) in errors {
        assert_eq!(error.code(), code);
        assert!(!error.to_string().is_empty());
    }
}
