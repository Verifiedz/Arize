//! Turning what people type into what records store (ADR 0024): local times into UTC, and plain
//! dates kept as dates. Pure: the current moment and the timezone come in as a [`Now`], never from
//! a clock (§12 rule 10).

use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use serde_json::{Map, Value};
use shimmer_core::{Error, Result};

use crate::schema::{Collection, FieldType};

/// The current moment and the configured timezone (ADR 0009), together, so everything that
/// needs "now" or "today" reads the same one.
#[derive(Clone, Copy, Debug)]
pub struct Now {
    pub at: DateTime<Utc>,
    pub tz: Tz,
}

impl Now {
    /// Today in the configured timezone.
    pub fn today(&self) -> NaiveDate {
        self.at.with_timezone(&self.tz).date_naive()
    }

    /// The current moment as a stored `datetime` value.
    pub fn stamp(&self) -> String {
        utc(self.at)
    }
}

/// A `datetime` value as stored: RFC 3339 in UTC, to the second.
fn utc(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// What someone typed for a `datetime` field, as stored (ADR 0024 §1): a plain date stays a plain
/// date; a time with an offset or `Z` becomes UTC; a time without one is read in `tz`.
pub fn datetime(input: &str, tz: &Tz) -> std::result::Result<String, String> {
    let s = input.trim();
    if NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() {
        return Ok(s.to_owned());
    }
    if let Ok(at) = DateTime::parse_from_rfc3339(s) {
        return Ok(utc(at.with_timezone(&Utc)));
    }
    for format in ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(local) = NaiveDateTime::parse_from_str(s, format) {
            return match tz.from_local_datetime(&local) {
                LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => Ok(utc(at.with_timezone(&Utc))),
                LocalResult::None => Err(format!("{s} doesn't exist in {tz} (the clocks skip it)")),
            };
        }
    }
    Err("a date and time like \"2026-10-20 23:59\", \"2026-10-20T23:59:00-07:00\", or a date".into())
}

/// A stored `datetime` (or `date`) value as an instant, for comparing and sorting. A plain date
/// is the start of that day in `tz`. `None` for anything else (a hand-edit gone wrong).
pub fn instant(stored: &str, tz: &Tz) -> Option<DateTime<Utc>> {
    if let Ok(at) = DateTime::parse_from_rfc3339(stored) {
        return Some(at.with_timezone(&Utc));
    }
    let day = NaiveDate::parse_from_str(stored, "%Y-%m-%d").ok()?;
    match tz.from_local_datetime(&day.and_hms_opt(0, 0, 0)?) {
        LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => Some(at.with_timezone(&Utc)),
        // A day that starts in a gap (rare): its first moment is an hour later.
        LocalResult::None => {
            tz.from_local_datetime(&day.and_hms_opt(1, 0, 0)?).earliest().map(|at| at.with_timezone(&Utc))
        }
    }
}

/// Rewrite `values` in place so they're stored as records keeps them: `datetime` fields to their
/// stored form. Values of other fields, `null`s and unknown keys are left for the normal checks.
pub fn normalize(c: &Collection, values: &mut Map<String, Value>, now: &Now) -> Result<()> {
    for (name, value) in values.iter_mut() {
        let Some(field) = c.field(name) else { continue };
        if field.kind == FieldType::Datetime {
            if let Value::String(s) = value {
                let stored = datetime(s, &now.tz)
                    .map_err(|want| Error::invalid_params(format!("field '{name}' must be {want}, got \"{s}\"")))?;
                *value = Value::String(stored);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    const TORONTO: Tz = chrono_tz::America::Toronto;

    #[test]
    fn local_times_become_utc_and_dates_stay_dates() {
        // Toronto is UTC-4 in October.
        assert_eq!(datetime("2026-10-20 23:59", &TORONTO).unwrap(), "2026-10-21T03:59:00Z");
        assert_eq!(datetime("2026-10-20T23:59", &TORONTO).unwrap(), "2026-10-21T03:59:00Z");
        assert_eq!(datetime("2026-10-20T23:59:00-07:00", &TORONTO).unwrap(), "2026-10-21T06:59:00Z", "an offset wins");
        assert_eq!(datetime("2026-10-21T06:59:00Z", &TORONTO).unwrap(), "2026-10-21T06:59:00Z");
        assert_eq!(datetime("2026-10-20", &TORONTO).unwrap(), "2026-10-20", "no time given");
        assert!(datetime("next friday", &TORONTO).is_err());
    }

    #[test]
    fn daylight_saving_gaps_and_overlaps() {
        // 2026-03-08 02:30 doesn't happen in Toronto; 2026-11-01 01:30 happens twice.
        assert!(datetime("2026-03-08 02:30", &TORONTO).unwrap_err().contains("doesn't exist"));
        assert_eq!(datetime("2026-11-01 01:30", &TORONTO).unwrap(), "2026-11-01T05:30:00Z", "the earlier one");
    }

    #[test]
    fn a_plain_date_is_the_start_of_the_day() {
        let start = instant("2026-10-20", &TORONTO).unwrap();
        assert_eq!(start, Utc.with_ymd_and_hms(2026, 10, 20, 4, 0, 0).unwrap());
        assert!(instant("2026-10-20T03:00:00Z", &TORONTO).unwrap() < start, "11pm the night before");
        assert!(instant("garbage", &TORONTO).is_none());
    }

    #[test]
    fn today_is_local() {
        // 02:00 UTC is still the previous evening in Toronto.
        let now = Now { at: Utc.with_ymd_and_hms(2026, 10, 21, 2, 0, 0).unwrap(), tz: TORONTO };
        assert_eq!(now.today().to_string(), "2026-10-20");
        assert_eq!(now.stamp(), "2026-10-21T02:00:00Z");
    }
}
