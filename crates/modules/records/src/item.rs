//! One record: its file on disk and its shape on the wire. Pure functions (§12 rule 10).
//!
//! On disk, `items/<collection>/<id>.toml` is flat and hand-editable: `status` plus the field
//! values, with unset fields absent. On the wire an item is flat too: `id`, `status` and every
//! schema field, unset ones as `null`. That is the shape of the `records.list` mock fixture.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use shimmer_core::ids::is_valid_id;
use shimmer_core::{Error, Result};

use crate::schema::Collection;

const MAX_ID_LEN: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Todo,
    Done,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Done => "done",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub status: Status,
    /// Whether the file has (or gets) a `status` line. New records in a `completable = false`
    /// collection don't; a line already in a file is kept, never rewritten away (ADR 0021 §1).
    pub status_line: bool,
    /// Set fields only. May include keys a hand-edit added that the schema does not know.
    pub fields: BTreeMap<String, Value>,
}

/// A record id is its file name: `[a-z0-9][a-z0-9_-]*` (`shimmer_core::ids::is_valid_id`,
/// ADR 0008), so `1-two-sum` works too.
pub fn check_id(id: &str) -> Result<()> {
    if id.len() <= MAX_ID_LEN && is_valid_id(id) {
        Ok(())
    } else {
        Err(Error::invalid_params(format!(
            "record id '{id}' must be lowercase letters, digits, '-' or '_' (at most {MAX_ID_LEN})"
        )))
    }
}

/// Generated ids leave room for a `-NN` suffix under the 128-character limit (ADR 0018 §1).
const MAX_SLUG_LEN: usize = 120;

/// Text turned into an id (ADR 0018 §1): lowercase `a-z` and `0-9` kept, every run of anything
/// else becomes one `-`, no `-` at either end, cut at a `-` to at most 120 characters. Empty when
/// nothing usable is left (a title with no Latin letters or digits).
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out = out.trim_end_matches('-').to_owned();
    if out.len() > MAX_SLUG_LEN {
        out.truncate(MAX_SLUG_LEN);
        // Cut at the last whole word when there is one, so `amazon-sde-inte` doesn't happen.
        if let Some(cut) = out.rfind('-') {
            out.truncate(cut);
        }
    }
    out
}

/// The first id from `base` that `taken` says is free: `base`, then `base-2`, `base-3`, …, or,
/// when `always_number` is set (date ids), `base-1`, `base-2`, … (ADR 0018 §1).
pub fn first_free(base: &str, always_number: bool, mut taken: impl FnMut(&str) -> Result<bool>) -> Result<String> {
    if !always_number && !taken(base)? {
        return Ok(base.to_owned());
    }
    let first = if always_number { 1 } else { 2 };
    for n in first.. {
        let candidate = format!("{base}-{n}");
        if !taken(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!("an unbounded range always finds a free number")
}

impl Item {
    pub fn new(id: &str, fields: Map<String, Value>) -> Self {
        Self { id: id.to_owned(), status: Status::Todo, status_line: true, fields: fields.into_iter().collect() }
    }

    /// Read a record file. Lenient on purpose: a hand-edited file with an unknown key or a native
    /// TOML date still reads, and is only validated again when it is next written.
    pub fn from_toml(id: &str, text: &str) -> Result<Self> {
        let bad = |msg: String| Error::module_error(format!("record file '{id}.toml': {msg}"));
        let mut table: toml::Table = toml::from_str(text).map_err(|e| bad(e.to_string()))?;
        let status_line = table.contains_key("status");
        let status = match table.remove("status") {
            None => Status::Todo,
            Some(toml::Value::String(s)) if s == "todo" => Status::Todo,
            Some(toml::Value::String(s)) if s == "done" => Status::Done,
            Some(other) => return Err(bad(format!("status must be \"todo\" or \"done\", got {other}"))),
        };
        table.remove("id");
        let fields = table.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect();
        Ok(Self { id: id.to_owned(), status, status_line, fields })
    }

    pub fn to_toml(&self) -> String {
        let mut table = toml::Table::new();
        if self.status_line {
            table.insert("status".into(), toml::Value::String(self.status.as_str().into()));
        }
        for (k, v) in &self.fields {
            if let Some(v) = json_to_toml(v) {
                table.insert(k.clone(), v);
            }
        }
        toml::to_string(&table).unwrap_or_default()
    }

    /// The wire shape: `id`, `status`, every schema field (`null` when unset), then anything
    /// else the file holds.
    pub fn to_wire(&self, c: &Collection) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), Value::String(self.id.clone()));
        if c.completable {
            out.insert("status".into(), Value::String(self.status.as_str().into()));
        }
        for f in &c.fields {
            out.insert(f.name.clone(), self.fields.get(&f.name).cloned().unwrap_or(Value::Null));
        }
        for (k, v) in &self.fields {
            out.entry(k.clone()).or_insert_with(|| v.clone());
        }
        Value::Object(out)
    }

    /// Set each value, or unset it where the value is `null`.
    pub fn apply(&mut self, changes: Map<String, Value>) {
        for (k, v) in changes {
            if v.is_null() {
                self.fields.remove(&k);
            } else {
                self.fields.insert(k, v);
            }
        }
    }

    pub fn fields_map(&self) -> Map<String, Value> {
        self.fields.clone().into_iter().collect()
    }
}

/// The fields whose value differs between `before` and `after`, sorted: set, unset, or changed
/// (ADR 0017 §8).
pub fn changed(before: &BTreeMap<String, Value>, after: &BTreeMap<String, Value>) -> Vec<String> {
    let keys: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    keys.into_iter().filter(|k| before.get(*k) != after.get(*k)).cloned().collect()
}

/// TOML to JSON. Dates become `"YYYY-MM-DD"`-style strings, the form the schema checks.
pub fn toml_to_json(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::from(i),
        toml::Value::Float(f) => Value::from(f),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Value::Object(t.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect()),
    }
}

/// JSON to TOML. `null` has no TOML form and yields `None`: an unset field is an absent key.
fn json_to_toml(v: &Value) -> Option<toml::Value> {
    Some(match v {
        Value::Null => return None,
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => toml::Value::Integer(i),
            None => toml::Value::Float(n.as_f64()?),
        },
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(a) => toml::Value::Array(a.iter().filter_map(json_to_toml).collect()),
        Value::Object(o) => {
            toml::Value::Table(o.iter().filter_map(|(k, v)| Some((k.clone(), json_to_toml(v)?))).collect())
        }
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn leetcode() -> Collection {
        Collection::parse("leetcode", include_str!("../collections/leetcode.toml")).unwrap()
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn ids() {
        for ok in ["two-sum", "1-two-sum", "lru_cache", "a"] {
            assert!(check_id(ok).is_ok(), "{ok}");
        }
        for bad in ["", "Two-Sum", "-x", "a/b", "../x", "a b", &"x".repeat(129)] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn file_round_trip_drops_nulls() {
        let mut item = Item::new("two-sum", obj(json!({"title": "Two Sum", "difficulty": "easy"})));
        item.status = Status::Done;
        let text = item.to_toml();
        assert!(text.contains("status = \"done\""), "{text}");
        assert_eq!(Item::from_toml("two-sum", &text).unwrap(), item);

        item.apply(obj(json!({"difficulty": null, "url": "https://leetcode.com/problems/two-sum"})));
        assert!(!item.fields.contains_key("difficulty"));
        assert!(!item.to_toml().contains("difficulty"));
    }

    #[test]
    fn hand_edited_files_read_leniently() {
        let item = Item::from_toml("x", "title = \"X\"\nlast_solved = 2026-09-18\nextra = 1\n").unwrap();
        assert_eq!(item.status, Status::Todo);
        assert_eq!(item.fields["last_solved"], json!("2026-09-18"));
        assert_eq!(item.fields["extra"], json!(1));

        assert!(Item::from_toml("x", "status = \"maybe\"").unwrap_err().message.contains("'x.toml'"));
        assert!(Item::from_toml("x", "not toml [").is_err());
    }

    #[test]
    fn wire_shape_matches_the_mock_fixture() {
        let item = Item::new("lru-cache", obj(json!({"title": "LRU Cache", "difficulty": "medium"})));
        assert_eq!(
            item.to_wire(&leetcode()),
            json!({"id": "lru-cache", "status": "todo", "title": "LRU Cache", "difficulty": "medium",
                   "url": null, "last_solved": null})
        );
    }

    #[test]
    fn slugs_keep_letters_and_digits() {
        for (text, want) in [
            ("Amazon: SDE Intern", "amazon-sde-intern"),
            ("Two Sum", "two-sum"),
            ("  LRU   Cache!! ", "lru-cache"),
            ("Café Résumé", "caf-r-sum"),
            ("C++ & Rust (2027)", "c-rust-2027"),
            ("شركة", ""),
            ("---", ""),
        ] {
            assert_eq!(slug(text), want, "{text:?}");
            if !want.is_empty() {
                assert!(check_id(want).is_ok(), "{want} must be a valid id");
            }
        }
    }

    #[test]
    fn long_slugs_are_cut_at_a_word() {
        let long = "word ".repeat(40);
        let s = slug(&long);
        assert!(s.len() <= MAX_SLUG_LEN && !s.ends_with('-') && s.ends_with("word"), "{s}");
        assert!(check_id(&format!("{s}-99")).is_ok(), "room for a suffix");
    }

    #[test]
    fn first_free_adds_a_counter_only_when_needed() {
        let taken = ["two-sum", "two-sum-2", "2026-10-05-1"];
        let is_taken = |id: &str| Ok(taken.contains(&id));
        assert_eq!(first_free("lru-cache", false, is_taken).unwrap(), "lru-cache");
        assert_eq!(first_free("two-sum", false, is_taken).unwrap(), "two-sum-3");
        assert_eq!(first_free("2026-10-05", true, is_taken).unwrap(), "2026-10-05-2", "date ids always count");
        assert_eq!(first_free("2026-10-06", true, is_taken).unwrap(), "2026-10-06-1");
    }

    #[test]
    fn changed_lists_only_real_changes() {
        let before: BTreeMap<String, Value> = obj(json!({"a": 1, "b": "x", "c": true})).into_iter().collect();
        let after: BTreeMap<String, Value> = obj(json!({"a": 1, "b": "y", "d": 2})).into_iter().collect();
        assert_eq!(changed(&before, &after), ["b", "c", "d"], "changed, unset and set; 'a' is the same");
        assert!(changed(&before, &before).is_empty());
    }
}
