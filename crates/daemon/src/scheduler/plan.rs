//! When does a trigger fire? Pure functions over plain data, no runtime, no I/O (§12 rule 10).
//!
//! All times are UTC. Cron expressions are standard 5-field (`min hour dom month dow`,
//! Sunday = 0 or 7, Monday = 1) or 6-field with leading seconds.

use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use shimmer_core::{CatchUp, Error, Result, Schedule};

/// A firing later than this after its scheduled time counts as *missed*, not on time. Wide
/// enough that a busy poll never turns a normal firing into a "catch-up".
pub const ON_TIME_GRACE: Duration = Duration::from_secs(60);
/// Most occurrences examined in one catch-up. Beyond it the rest are counted as missed.
pub const MAX_SCAN: usize = 10_000;
/// Most firings a single `Backfill` may produce in one go.
pub const MAX_BACKFILL: usize = 1_000;

/// A validated schedule, ready to ask "what comes next?".
pub struct Parsed(Kind);

enum Kind {
    Cron(Box<cron::Schedule>),
    Every(chrono::Duration),
    Once(DateTime<Utc>),
}

impl Parsed {
    pub fn parse(schedule: &Schedule) -> Result<Self> {
        Ok(Self(match schedule {
            Schedule::Cron(expr) => Kind::Cron(Box::new(parse_cron(expr)?)),
            Schedule::Every(d) => {
                if d.is_zero() {
                    return Err(Error::invalid_params("'every' must be at least one second"));
                }
                let d = chrono::Duration::from_std(*d).map_err(|_| Error::invalid_params("'every' is too large"))?;
                Kind::Every(d)
            }
            Schedule::Once(t) => Kind::Once(*t),
        }))
    }

    /// First occurrence strictly after `after`, if any. A `Once` has exactly one.
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match &self.0 {
            Kind::Cron(c) => c.after(&after).next(),
            Kind::Every(d) => after.checked_add_signed(*d),
            Kind::Once(t) => (*t > after).then_some(*t),
        }
    }

    /// Where a trigger created at `now` first becomes due. `Every` counts from creation; a
    /// `Once` is due at its own time even if that is already past (`catch_up` then decides).
    pub fn first_due(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match &self.0 {
            Kind::Once(t) => Some(*t),
            _ => self.next_after(now),
        }
    }
}

/// Standard cron -> the `cron` crate. That crate numbers weekdays 1-7 from Sunday and rejects
/// 0, so a literal `1` would run on Sundays. Translate numeric weekdays to names.
fn parse_cron(expr: &str) -> Result<cron::Schedule> {
    let mut fields: Vec<String> = expr.split_whitespace().map(str::to_owned).collect();
    match fields.len() {
        5 => fields.insert(0, "0".into()),
        6 => {}
        n => return Err(Error::invalid_params(format!("cron needs 5 or 6 fields, got {n}: '{expr}'"))),
    }
    fields[5] = weekday_names(&fields[5]);
    cron::Schedule::from_str(&fields.join(" "))
        .map_err(|e| Error::invalid_params(format!("invalid cron '{expr}': {e}")))
}

fn weekday_names(field: &str) -> String {
    const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let mut out = String::new();
    let mut digits = String::new();
    let mut after_step = false;
    let flush = |digits: &mut String, out: &mut String, after_step: bool| {
        if digits.is_empty() {
            return;
        }
        match digits.parse::<usize>() {
            // A step size ("1-5/2") is a count, not a weekday.
            Ok(n) if !after_step && n <= 7 => out.push_str(NAMES[n % 7]),
            _ => out.push_str(digits),
        }
        digits.clear();
    };
    for c in field.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            flush(&mut digits, &mut out, after_step);
            after_step = c == '/';
            out.push(c);
        }
    }
    flush(&mut digits, &mut out, after_step);
    out
}

#[derive(Debug, PartialEq, Eq)]
pub struct Decision {
    /// Scheduled times to fire now, oldest first.
    pub fire: Vec<DateTime<Utc>>,
    /// Due occurrences dropped by the catch-up policy.
    pub missed: usize,
    /// The new `next_due`; `None` means the trigger is finished (a `Once`).
    pub next_due: Option<DateTime<Utc>>,
}

/// Given a trigger whose `next_due` has passed, decide what fires (§6.3).
///
/// * `Skip`: only an on-time firing runs; anything missed is dropped.
/// * `RunOnce`: one firing, however many were missed, stamped with the latest occurrence.
/// * `Backfill`: one firing per occurrence, each stamped with its own historical time.
pub fn decide(schedule: &Parsed, catch_up: CatchUp, next_due: DateTime<Utc>, now: DateTime<Utc>) -> Decision {
    if next_due > now {
        return Decision { fire: vec![], missed: 0, next_due: Some(next_due) };
    }
    let mut occurrences = Vec::new();
    let mut cursor = Some(next_due);
    let mut truncated = false;
    while let Some(t) = cursor.filter(|t| *t <= now) {
        if occurrences.len() == MAX_SCAN {
            truncated = true;
            break;
        }
        occurrences.push(t);
        cursor = schedule.next_after(t);
    }
    // After a truncated scan `cursor` is still in the past; jump to the first future one.
    let next_due = if truncated { schedule.next_after(now) } else { cursor };
    let dropped_by_scan = if truncated { 1 } else { 0 }; // "at least one more": exact count is unknowable cheaply

    let grace = chrono::Duration::from_std(ON_TIME_GRACE).unwrap_or(chrono::Duration::MAX);
    let total = occurrences.len();
    let fire: Vec<_> = match catch_up {
        CatchUp::Skip => occurrences.iter().copied().filter(|t| now - *t <= grace).collect(),
        CatchUp::RunOnce => occurrences.last().copied().into_iter().collect(),
        CatchUp::Backfill => occurrences.iter().copied().take(MAX_BACKFILL).collect(),
    };
    Decision { missed: total - fire.len() + dropped_by_scan, fire, next_due }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(d: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, d, h, m, s).unwrap()
    }

    fn daily() -> Parsed {
        Parsed::parse(&Schedule::Every(Duration::from_secs(86_400))).unwrap()
    }

    #[test]
    fn asleep_three_days_then_each_policy_does_what_it_says() {
        // Due Fri 09:00, machine wakes Mon 12:00: Fri, Sat, Sun and Mon 09:00 all passed.
        let due = at(18, 9, 0, 0);
        let now = at(21, 12, 0, 0);
        let s = daily();

        let skip = decide(&s, CatchUp::Skip, due, now);
        assert_eq!((skip.fire.len(), skip.missed), (0, 4));

        let once = decide(&s, CatchUp::RunOnce, due, now);
        assert_eq!(once.fire, [at(21, 9, 0, 0)], "one firing, stamped with the latest occurrence");
        assert_eq!(once.missed, 3);

        let fill = decide(&s, CatchUp::Backfill, due, now);
        assert_eq!(fill.fire, [at(18, 9, 0, 0), at(19, 9, 0, 0), at(20, 9, 0, 0), at(21, 9, 0, 0)]);
        assert_eq!(fill.missed, 0);

        for d in [&skip, &once, &fill] {
            assert_eq!(d.next_due, Some(at(22, 9, 0, 0)), "next_due always lands in the future");
        }
    }

    #[test]
    fn a_firing_within_grace_is_on_time_even_for_skip() {
        let d = decide(&daily(), CatchUp::Skip, at(21, 9, 0, 0), at(21, 9, 0, 30));
        assert_eq!(d.fire, [at(21, 9, 0, 0)]);
        assert_eq!(d.missed, 0);
        let late = decide(&daily(), CatchUp::Skip, at(21, 9, 0, 0), at(21, 9, 2, 0));
        assert!(late.fire.is_empty() && late.missed == 1);
    }

    #[test]
    fn not_yet_due_changes_nothing() {
        let d = decide(&daily(), CatchUp::Backfill, at(22, 9, 0, 0), at(21, 9, 0, 0));
        assert_eq!(d, Decision { fire: vec![], missed: 0, next_due: Some(at(22, 9, 0, 0)) });
    }

    #[test]
    fn every_does_not_drift_with_late_wakeups() {
        let d = decide(&daily(), CatchUp::RunOnce, at(21, 9, 0, 0), at(21, 17, 45, 0));
        assert_eq!(d.next_due, Some(at(22, 9, 0, 0)), "anchored to the schedule, not to when we woke");
    }

    #[test]
    fn once_fires_then_finishes() {
        let s = Parsed::parse(&Schedule::Once(at(24, 9, 0, 0))).unwrap();
        assert_eq!(s.first_due(at(21, 0, 0, 0)), Some(at(24, 9, 0, 0)));
        let d = decide(&s, CatchUp::Skip, at(24, 9, 0, 0), at(24, 9, 0, 5));
        assert_eq!((d.fire, d.next_due), (vec![at(24, 9, 0, 0)], None));
        // Already past by days when first seen: skip drops it, and it is finished either way.
        let d = decide(&s, CatchUp::Skip, at(24, 9, 0, 0), at(27, 9, 0, 0));
        assert_eq!((d.fire.len(), d.missed, d.next_due), (0, 1, None));
        let d = decide(&s, CatchUp::RunOnce, at(24, 9, 0, 0), at(27, 9, 0, 0));
        assert_eq!((d.fire, d.next_due), (vec![at(24, 9, 0, 0)], None));
    }

    #[test]
    fn runaway_catch_up_is_bounded() {
        let s = Parsed::parse(&Schedule::Every(Duration::from_secs(1))).unwrap();
        let now = at(21, 0, 0, 0);
        let due = now - chrono::Duration::days(30);
        let d = decide(&s, CatchUp::Backfill, due, now);
        assert_eq!(d.fire.len(), MAX_BACKFILL);
        assert!(d.missed > 0);
        assert!(d.next_due.unwrap() > now);
    }

    #[test]
    fn cron_weekdays_mean_what_standard_cron_means() {
        let monday_9 = |expr: &str| {
            let s = Parsed::parse(&Schedule::Cron(expr.into())).unwrap();
            s.next_after(at(21, 12, 0, 0)).unwrap() // Mon 21 Sep 2026, after 09:00
        };
        // 1 = Monday. The `cron` crate alone would say Sunday.
        assert_eq!(monday_9("0 9 * * 1"), at(28, 9, 0, 0));
        assert_eq!(monday_9("0 9 * * Mon"), at(28, 9, 0, 0));
        assert_eq!(monday_9("0 9 * * 0"), at(27, 9, 0, 0), "0 = Sunday");
        assert_eq!(monday_9("0 9 * * 7"), at(27, 9, 0, 0), "7 = Sunday");
        assert_eq!(monday_9("0 9 * * 1-5"), at(22, 9, 0, 0), "Tuesday is the next weekday");
        assert_eq!(monday_9("0 9 * * 6,0"), at(26, 9, 0, 0), "Saturday");
        assert_eq!(monday_9("0 0 9 * * 1"), at(28, 9, 0, 0), "6-field form too");
        assert_eq!(monday_9("*/30 * * * *"), at(21, 12, 30, 0));
    }

    #[test]
    fn weekday_translation_leaves_step_sizes_alone() {
        assert_eq!(weekday_names("1-5/2"), "Mon-Fri/2");
        assert_eq!(weekday_names("*/2"), "*/2");
        assert_eq!(weekday_names("*"), "*");
        assert_eq!(weekday_names("1,3,5"), "Mon,Wed,Fri");
    }

    #[test]
    fn bad_schedules_are_rejected() {
        for bad in ["", "* * *", "61 * * * *", "0 9 * * banana", "0 0 0 0 0 0 0 0"] {
            assert!(Parsed::parse(&Schedule::Cron(bad.into())).is_err(), "{bad:?}");
        }
        assert!(Parsed::parse(&Schedule::Every(Duration::ZERO)).is_err());
    }
}
