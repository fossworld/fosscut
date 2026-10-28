use crate::{Rate, TimeError};

/// Frames skipped per minute in drop-frame timecode (2 at 29.97, 4 at 59.94).
fn dropped_per_minute(rate: Rate) -> i64 {
    i64::from(rate.nominal_fps()) / 15
}

/// Formats a frame index as SMPTE timecode at `rate`.
///
/// NTSC 29.97 and 59.94 use drop-frame timecode with `;` before the frame field;
/// every other rate uses non-drop timecode with `:` separators. Negative frame indices
/// are prefixed with `-`.
pub fn format_timecode(frames: i64, rate: Rate) -> String {
    let sign = if frames < 0 { "-" } else { "" };
    let mut frame = frames.unsigned_abs() as i64;
    let fps = i64::from(rate.nominal_fps());
    let drop = rate.is_drop_frame();
    if drop {
        let dropped = dropped_per_minute(rate);
        let per_ten_minutes = fps * 600 - dropped * 9;
        let per_minute = fps * 60 - dropped;
        let tens = frame / per_ten_minutes;
        let remainder = frame % per_ten_minutes;
        frame += dropped * 9 * tens;
        if remainder > dropped {
            frame += dropped * ((remainder - dropped) / per_minute);
        }
    }
    let ff = frame % fps;
    let total_seconds = frame / fps;
    let ss = total_seconds % 60;
    let mm = (total_seconds / 60) % 60;
    let hh = total_seconds / 3600;
    let separator = if drop { ';' } else { ':' };
    format!("{sign}{hh:02}:{mm:02}:{ss:02}{separator}{ff:02}")
}

/// Parses SMPTE timecode `HH:MM:SS:FF` at `rate` into a frame index.
///
/// A `;` or `.` before the frame field selects drop-frame timecode, which is only valid
/// at 29.97 and 59.94 fps.
pub fn parse_timecode(input: &str, rate: Rate) -> Result<i64, TimeError> {
    let invalid = |reason: String| TimeError::InvalidTimecode {
        input: input.to_string(),
        reason,
    };
    let text = input.trim();
    let drop = text.contains(';') || text.matches('.').count() == 1;
    let normalized: String = text
        .chars()
        .map(|c| if c == ';' || c == '.' { ':' } else { c })
        .collect();
    let parts: Vec<&str> = normalized.split(':').collect();
    if parts.len() != 4
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(invalid("expected HH:MM:SS:FF".into()));
    }
    let parse = |p: &str| {
        p.parse::<i64>()
            .map_err(|_| invalid("number too large".into()))
    };
    let (hh, mm, ss, ff) = (
        parse(parts[0])?,
        parse(parts[1])?,
        parse(parts[2])?,
        parse(parts[3])?,
    );
    let fps = i64::from(rate.nominal_fps());
    if mm >= 60 || ss >= 60 {
        return Err(invalid("minutes and seconds must be below 60".into()));
    }
    if ff >= fps {
        return Err(invalid(format!(
            "frame field must be below {fps} at {rate} fps"
        )));
    }
    let nominal = ((hh * 3600 + mm * 60 + ss) * fps)
        .checked_add(ff)
        .ok_or(TimeError::Overflow)?;
    if !drop {
        return Ok(nominal);
    }
    if !rate.is_drop_frame() {
        return Err(invalid(format!(
            "drop-frame timecode is only valid at 29.97 or 59.94 fps, not {rate}"
        )));
    }
    let dropped = dropped_per_minute(rate);
    if ss == 0 && mm % 10 != 0 && ff < dropped {
        return Err(invalid(
            "this frame number does not exist in drop-frame timecode".into(),
        ));
    }
    let total_minutes = hh * 60 + mm;
    Ok(nominal - dropped * (total_minutes - total_minutes / 10))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_drop_round_trip() {
        let rate = Rate::FPS_30;
        assert_eq!(parse_timecode("00:00:03:28", rate).unwrap(), 118);
        assert_eq!(format_timecode(118, rate), "00:00:03:28");
        assert_eq!(format_timecode(30 * 3600, rate), "01:00:00:00");
    }

    #[test]
    fn drop_frame_known_values() {
        let rate = Rate::NTSC_30;
        assert_eq!(format_timecode(1799, rate), "00:00:59;29");
        assert_eq!(format_timecode(1800, rate), "00:01:00;02");
        assert_eq!(format_timecode(17982, rate), "00:10:00;00");
        assert_eq!(parse_timecode("00:01:00;02", rate).unwrap(), 1800);
        assert!(parse_timecode("00:01:00;00", rate).is_err());
        assert!(parse_timecode("00:00:01;00", Rate::FPS_30).is_err());
    }

    #[test]
    fn rejects_malformed_timecode() {
        for bad in ["3:28", "00:00:03:30", "00:61:00:00", "aa:bb:cc:dd", ""] {
            assert!(
                parse_timecode(bad, Rate::FPS_30).is_err(),
                "{bad:?} should fail"
            );
        }
    }
}
