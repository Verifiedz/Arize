//! Completing and reopening a record (ADR 0016): what may happen and what the record looks like
//! afterwards. Pure functions over `Item` and `Collection`, no `Ctx` (§12 rule 10); the ops in
//! `lib.rs` read the record, call these, and write the result with its event.

use serde_json::{Map, Value};
use shimmer_core::{Error, Result};

use crate::item::{Item, Status};
use crate::schema::{Collection, RepeatComplete};

/// The record after `records.complete`. `changes` are checked like `records.update`'s, applied
/// first, then the record is marked done and stamped. An explicit, non-null value for the stamp
/// field in `changes` wins over `today`, which is how a completion is back-dated.
pub fn complete(c: &Collection, item: &Item, mut changes: Map<String, Value>, today: &str) -> Result<Item> {
    if item.status == Status::Done && c.repeat_complete == RepeatComplete::Refuse {
        let when = c.stamp_on_complete.as_ref().and_then(|s| item.fields.get(s)).and_then(Value::as_str);
        return Err(Error::conflict(match when {
            Some(date) => format!("'{}' in '{}' was already completed on {date}", item.id, c.id),
            None => format!("'{}' in '{}' is already done", item.id, c.id),
        }));
    }
    c.check_changes(&changes)?;

    if let Some(stamp) = &c.stamp_on_complete {
        // No value, or `null` (which would leave a completed record unstamped): today.
        if changes.get(stamp).is_none_or(Value::is_null) {
            changes.insert(stamp.clone(), Value::String(today.to_owned()));
        }
    }
    let mut next = item.clone();
    next.apply(changes);
    next.status = Status::Done;
    c.check_required(&next.fields_map())?;
    Ok(next)
}

/// The record after `records.reopen`: `todo` again. The stamp is kept unless `clear_stamp`.
/// Reopening a record that isn't done is `conflict`, so a script reopening the wrong record
/// finds out.
pub fn reopen(c: &Collection, item: &Item, clear_stamp: bool) -> Result<Item> {
    if item.status != Status::Done {
        return Err(Error::conflict(format!("'{}' in '{}' is not done", item.id, c.id)));
    }
    let mut next = item.clone();
    next.status = Status::Todo;
    if let (true, Some(stamp)) = (clear_stamp, &c.stamp_on_complete) {
        next.fields.remove(stamp);
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use shimmer_core::ErrorCode;

    use super::*;

    const TODAY: &str = "2026-10-05";

    fn collection(repeat: &str) -> Collection {
        let text = format!(
            r#"
            [collection]
            id = "jobs"
            label = "Jobs"
            stamp_on_complete = "applied_on"
            repeat_complete = "{repeat}"

            [[field]]
            name = "company"
            type = "string"
            required = true

            [[field]]
            name = "stage"
            type = "enum"
            values = ["oa", "interview"]

            [[field]]
            name = "applied_on"
            type = "date"
            "#
        );
        Collection::parse("jobs", &text).unwrap()
    }

    fn todo() -> Item {
        Item::new("acme", json!({"company": "Acme"}).as_object().unwrap().clone())
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn complete_marks_done_and_stamps_today() {
        let done = complete(&collection("restamp"), &todo(), Map::new(), TODAY).unwrap();
        assert_eq!(done.status, Status::Done);
        assert_eq!(done.fields["applied_on"], json!(TODAY));
    }

    #[test]
    fn complete_applies_changes_in_the_same_step() {
        let done = complete(&collection("restamp"), &todo(), obj(json!({"stage": "oa"})), TODAY).unwrap();
        assert_eq!((done.fields["stage"].clone(), done.status), (json!("oa"), Status::Done));
    }

    #[test]
    fn an_explicit_stamp_value_back_dates_the_completion() {
        let done = complete(&collection("restamp"), &todo(), obj(json!({"applied_on": "2026-10-03"})), TODAY).unwrap();
        assert_eq!(done.fields["applied_on"], json!("2026-10-03"));

        // A null stamp can't leave a completed record unstamped: today wins.
        let done = complete(&collection("restamp"), &todo(), obj(json!({"applied_on": null})), TODAY).unwrap();
        assert_eq!(done.fields["applied_on"], json!(TODAY));
    }

    #[test]
    fn complete_checks_changes_like_update() {
        let c = collection("restamp");
        for (changes, says) in [
            (json!({"colour": "red"}), "has no field 'colour'"),
            (json!({"stage": "offer"}), "must be one of oa, interview"),
            (json!({"applied_on": "yesterday"}), "must be a date"),
            (json!({"company": null}), "'company' is required"),
        ] {
            let e = complete(&c, &todo(), obj(changes.clone()), TODAY).unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidParams, "{changes}");
            assert!(e.message.contains(says), "{changes}: {}", e.message);
        }
    }

    #[test]
    fn restamp_completes_a_done_record_again() {
        let c = collection("restamp");
        let done = complete(&c, &todo(), Map::new(), "2026-10-01").unwrap();
        let again = complete(&c, &done, Map::new(), TODAY).unwrap();
        assert_eq!(again.fields["applied_on"], json!(TODAY));
    }

    #[test]
    fn refuse_turns_a_second_completion_into_a_conflict() {
        let c = collection("refuse");
        let done = complete(&c, &todo(), Map::new(), "2026-10-01").unwrap();
        let e = complete(&c, &done, obj(json!({"stage": "oa"})), TODAY).unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        assert_eq!(e.message, "'acme' in 'jobs' was already completed on 2026-10-01");

        // Without a stamp there's no date to name.
        let mut unstamped = done.clone();
        unstamped.fields.remove("applied_on");
        let e = complete(&c, &unstamped, Map::new(), TODAY).unwrap_err();
        assert_eq!(e.message, "'acme' in 'jobs' is already done");

        // Reopening first makes it possible again, on purpose.
        let reopened = reopen(&c, &done, false).unwrap();
        assert!(complete(&c, &reopened, Map::new(), TODAY).is_ok());
    }

    #[test]
    fn reopen_keeps_the_stamp_unless_asked() {
        let c = collection("restamp");
        let done = complete(&c, &todo(), Map::new(), TODAY).unwrap();

        let kept = reopen(&c, &done, false).unwrap();
        assert_eq!((kept.status, kept.fields["applied_on"].clone()), (Status::Todo, json!(TODAY)));

        let cleared = reopen(&c, &done, true).unwrap();
        assert!(!cleared.fields.contains_key("applied_on"));
    }

    #[test]
    fn reopening_a_todo_record_is_a_conflict() {
        let e = reopen(&collection("restamp"), &todo(), false).unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (ErrorCode::Conflict, "'acme' in 'jobs' is not done"));
    }
}
