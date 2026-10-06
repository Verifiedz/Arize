//! Durations and times as people type and read them (ADR 0015 §2, §3). Presentation only: what
//! goes to the daemon is whole seconds and UTC RFC 3339, exactly as the protocol defines.

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde_json::Value;

const UNITS: [(char, u64); 5] = [('w', 604_800), ('d', 86_400), ('h', 3_600), ('m', 60), ('s', 1)];

/// `30s`, `15m`, `2h`, `3d`, `1w`, or combined (`1h30m`) → seconds. A bare number is rejected, so
/// `--every 2` can never mean two seconds by accident.
pub fn parse_duration(text: &str) -> Result<u64, String> {
    let bad = || format!("'{text}' isn't a duration: use a number and a unit, e.g. 30s, 15m, 2h, 3d, 1w or 1h30m");
    let (mut total, mut number) = (0u64, String::new());
    for c in text.trim().chars() {
        if c.is_ascii_digit() {
            number.push(c);
        } else {
            let unit = UNITS.iter().find(|(u, _)| *u == c.to_ascii_lowercase()).ok_or_else(bad)?.1;
            let n: u64 = number.parse().map_err(|_| bad())?;
            total = n.checked_mul(unit).and_then(|v| total.checked_add(v)).ok_or_else(bad)?;
            number.clear();
        }
    }
    if !number.is_empty() || total == 0 {
        return Err(bad());
    }
    Ok(total)
}

/// Seconds as the two largest units: `172800` → `2d`, `5400` → `1h 30m`, `45` → `45s`.
pub fn format_duration(secs: u64) -> String {
    let mut parts = Vec::new();
    let mut left = secs;
    for (unit, size) in UNITS {
        if left >= size && parts.len() < 2 {
            parts.push(format!("{}{unit}", left / size));
            left %= size;
        } else if !parts.is_empty() {
            break; // two largest *adjacent* units only: "1w 3h" would read oddly
        }
    }
    if parts.is_empty() {
        "0s".into()
    } else {
        parts.join(" ")
    }
}

/// A `--once` time: RFC 3339 with an offset, or `YYYY-MM-DD HH:MM[:SS]` (a `T` works too) read as
/// this machine's local time.
pub fn parse_once(text: &str) -> Result<DateTime<Utc>, String> {
    if let Ok(t) = DateTime::parse_from_rfc3339(text) {
        return Ok(t.with_timezone(&Utc));
    }
    let naive = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(text, f).ok())
        .ok_or_else(|| {
            format!("'{text}' isn't a time: use 2026-10-10 09:00 (your local time) or 2026-10-10T09:00:00Z")
        })?;
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(|| format!("'{text}' doesn't exist or is ambiguous in your local time (a clock change); add an offset, e.g. {text}+01:00"))
}

/// An RFC 3339 timestamp from the daemon, if `v` holds one.
pub fn timestamp(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|t| t.with_timezone(&Utc))
}

/// How long ago `t` was: `12s`, `3m`, `2h 5m`.
pub fn age(t: DateTime<Utc>, now: DateTime<Utc>) -> String {
    format_short((now - t).num_seconds().max(0) as u64)
}

/// `3m ago`, or `in 2h` for a time still to come.
pub fn relative(t: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (t - now).num_seconds();
    if secs > 0 {
        format!("in {}", format_short(secs as u64))
    } else if secs > -2 {
        "now".into()
    } else {
        format!("{} ago", format_short((-secs) as u64))
    }
}

/// `t` in this machine's local time: `2026-10-10 09:00:00`.
pub fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Like [`format_duration`] but the one largest unit for short spans, two for long ones.
fn format_short(secs: u64) -> String {
    if secs < 3_600 {
        UNITS.iter().find(|(_, size)| secs >= *size).map_or("0s".into(), |(u, size)| format!("{}{u}", secs / size))
    } else {
        format_duration(secs)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn durations_need_a_unit_and_combine() {
        for (text, secs) in [
            ("30s", 30),
            ("15m", 900),
            ("2h", 7_200),
            ("3d", 259_200),
            ("1w", 604_800),
            ("1h30m", 5_400),
            ("2D", 172_800),
        ] {
            assert_eq!(parse_duration(text), Ok(secs), "{text}");
        }
        for bad in ["2", "", "h", "2x", "0s", "1h30", "-5m", "99999999999999999999w"] {
            assert!(parse_duration(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn durations_read_back_in_two_units() {
        for (secs, text) in [
            (0, "0s"),
            (45, "45s"),
            (900, "15m"),
            (5_400, "1h 30m"),
            (172_800, "2d"),
            (90_061, "1d 1h"),
            (604_800, "1w"),
        ] {
            assert_eq!(format_duration(secs), text, "{secs}");
        }
        assert_eq!(parse_duration(&format_duration(5_400).replace(' ', "")), Ok(5_400));
    }

    #[test]
    fn once_takes_an_offset_or_local_time() {
        let utc = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        assert_eq!(parse_once("2026-10-10T09:00:00Z"), Ok(utc("2026-10-10T09:00:00Z")));
        assert_eq!(parse_once("2026-10-10T09:00:00+01:00"), Ok(utc("2026-10-10T08:00:00Z")));
        let local = Local
            .from_local_datetime(&NaiveDateTime::parse_from_str("2026-10-10 09:00", "%Y-%m-%d %H:%M").unwrap())
            .single()
            .unwrap();
        for text in ["2026-10-10 09:00", "2026-10-10T09:00", "2026-10-10 09:00:00"] {
            assert_eq!(parse_once(text), Ok(local.with_timezone(&Utc)), "{text}");
        }
        assert!(parse_once("tomorrow").unwrap_err().contains("isn't a time"));
    }

    #[test]
    fn relative_times_read_naturally() {
        let now = Utc::now();
        assert_eq!(relative(now + Duration::seconds(7_201), now), "in 2h");
        assert_eq!(relative(now - Duration::seconds(180), now), "3m ago");
        assert_eq!(relative(now, now), "now");
        assert_eq!(age(now - Duration::seconds(12), now), "12s");
        assert_eq!(age(now - Duration::seconds(93_600), now), "1d 2h");
    }
}
