//! Collection definitions: the TOML that turns the generic record store into a LeetCode tracker
//! or a job-application tracker (CLAUDE.md §8). Pure data and pure functions (§12 rule 10).

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use shimmer_core::ids::is_valid_name;
use shimmer_core::{Error, Result};

use crate::item::toml_to_json;

/// Keys every item has on the wire; a field may not take these names.
pub const RESERVED: [&str; 2] = ["id", "status"];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Collection {
    pub id: String,
    pub label: String,
    /// One line, display only (ADR 0017 §1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// How to name a record for people, e.g. `"{company}: {position}"` (ADR 0017 §2). Checked
    /// on parse; render with [`Collection::title_of`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Collections that go with this one. Display only, never checked (ADR 0017 §5).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    pub fields: Vec<Field>,
    /// The date field `records.complete` sets to today.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stamp_on_complete: Option<String>,
    /// What completing an already-`done` record does (ADR 0016 §3). Always serialised, with
    /// the default filled in, so clients never need to know the default.
    pub repeat_complete: RepeatComplete,
    /// Opaque here, passed through for clients. There is no view type yet (§5).
    pub views: Vec<Value>,
    /// `[extra.<name>]` tables, kept and returned untouched (ADR 0017 §6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

/// `[collection] repeat_complete` (ADR 0016 §3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatComplete {
    /// Complete it again: re-stamp and emit another completion. Right for things you repeat.
    #[default]
    Restamp,
    /// `conflict` until the record is reopened. Right for things that happen once.
    Refuse,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: FieldType,
    /// Allowed values of an `enum` field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(default)]
    pub required: bool,
    /// What the field means (ADR 0017 §3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    /// No two records share a set value (ADR 0017 §4).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unique: bool,
}

/// What a field means, so other features can use it without knowing the collection
/// (ADR 0017 §3). The vocabulary grows by ADR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Something is due on this date. `date` fields only.
    Deadline,
    /// The record's link. `string` fields only; at most one per collection.
    Url,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    String,
    Int,
    Bool,
    Date,
    Enum,
}

/// The file as written. `deny_unknown_fields` turns a typo like `value = [...]` into an error
/// instead of a silently ignored key.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    collection: Header,
    #[serde(default)]
    field: Vec<Field>,
    #[serde(default)]
    view: Vec<toml::Value>,
    #[serde(default)]
    extra: Option<toml::Table>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    id: String,
    label: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    related: Vec<String>,
    #[serde(default)]
    stamp_on_complete: Option<String>,
    #[serde(default)]
    repeat_complete: RepeatComplete,
}

impl Collection {
    /// Parse `collections/<file_id>.toml`. Every problem names the file, since the user wrote it.
    pub fn parse(file_id: &str, text: &str) -> Result<Self> {
        let bad = |msg: String| Error::invalid_params(format!("collection file '{file_id}.toml': {msg}"));
        let file: File = toml::from_str(text).map_err(|e| bad(e.to_string()))?;
        if file.collection.id != file_id {
            return Err(bad(format!("declares id '{}'; it must match the file name", file.collection.id)));
        }

        let mut seen = BTreeSet::new();
        for f in &file.field {
            if !is_valid_name(&f.name) || RESERVED.contains(&f.name.as_str()) {
                return Err(bad(format!("field name '{}' is invalid or reserved", f.name)));
            }
            if !seen.insert(f.name.as_str()) {
                return Err(bad(format!("field '{}' is defined twice", f.name)));
            }
            match (f.kind, f.values.is_empty()) {
                (FieldType::Enum, true) => return Err(bad(format!("enum field '{}' needs values", f.name))),
                (FieldType::Enum, false) if f.values.iter().collect::<BTreeSet<_>>().len() != f.values.len() => {
                    return Err(bad(format!("enum field '{}' repeats a value", f.name)));
                }
                (FieldType::Enum, false) => {}
                (_, false) => return Err(bad(format!("field '{}': only enum fields take values", f.name))),
                (_, true) => {}
            }
            match (f.role, f.kind) {
                (Some(Role::Deadline), k) if k != FieldType::Date => {
                    return Err(bad(format!("field '{}': role \"deadline\" is only for date fields", f.name)));
                }
                (Some(Role::Url), k) if k != FieldType::String => {
                    return Err(bad(format!("field '{}': role \"url\" is only for string fields", f.name)));
                }
                _ => {}
            }
            if f.unique && f.kind == FieldType::Bool {
                return Err(bad(format!("field '{}': a bool field can't be unique", f.name)));
            }
        }
        if file.field.iter().filter(|f| f.role == Some(Role::Url)).count() > 1 {
            return Err(bad("only one field may have role \"url\"".into()));
        }
        if let Some(r) = file.collection.related.iter().find(|r| !is_valid_name(r)) {
            return Err(bad(format!("related '{r}' is not a valid collection id")));
        }
        if let Some(title) = &file.collection.title {
            let names: BTreeSet<&str> = file.field.iter().map(|f| f.name.as_str()).collect();
            for part in title_parts(title).map_err(|m| bad(format!("title: {m}")))? {
                if let Part::Field(name) = part {
                    if !names.contains(name) {
                        return Err(bad(format!("title: '{{{name}}}' is not a field")));
                    }
                }
            }
        }
        let extra = match file.extra {
            None => None,
            Some(table) => {
                if let Some((name, _)) = table.iter().find(|(_, v)| !v.is_table()) {
                    return Err(bad(format!("extra.{name} must be a table: write it as [extra.{name}]")));
                }
                Some(toml_to_json(toml::Value::Table(table)))
            }
        };

        let c = Self {
            id: file.collection.id,
            label: file.collection.label,
            description: file.collection.description,
            title: file.collection.title,
            related: file.collection.related,
            fields: file.field,
            stamp_on_complete: file.collection.stamp_on_complete,
            repeat_complete: file.collection.repeat_complete,
            views: file.view.into_iter().map(toml_to_json).collect(),
            extra,
        };
        if let Some(stamp) = &c.stamp_on_complete {
            match c.field(stamp) {
                Some(f) if f.kind == FieldType::Date && f.role == Some(Role::Deadline) => {
                    return Err(bad(format!(
                        "stamp_on_complete '{stamp}' is a deadline; a stamp records something that happened"
                    )));
                }
                Some(f) if f.kind == FieldType::Date => {}
                _ => return Err(bad(format!("stamp_on_complete '{stamp}' must name a date field"))),
            }
        }
        Ok(c)
    }

    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// A record's title (ADR 0017 §2): the `title` template with each `{field}` replaced by its
    /// value (unset renders as nothing), trimmed. The record id when there is no template, or
    /// when none of the fields it names is set (so `": "` never stands in for a name).
    pub fn title_of(&self, id: &str, fields: &BTreeMap<String, Value>) -> String {
        self.render_title(fields).unwrap_or_else(|| id.to_owned())
    }

    /// The rendered title, or `None` when there is no template or none of its fields is set.
    /// Also what a generated record id is made from (ADR 0018 §1).
    pub fn render_title(&self, fields: &BTreeMap<String, Value>) -> Option<String> {
        let parts = title_parts(self.title.as_deref()?).ok()?;
        let value = |name: &str| match fields.get(name) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => Some(other.to_string()),
        };
        let mut any_set = false;
        let mut out = String::new();
        for part in &parts {
            match part {
                Part::Text(t) => out.push_str(t),
                Part::Field(name) => {
                    if let Some(v) = value(name) {
                        any_set = true;
                        out.push_str(&v);
                    }
                }
            }
        }
        let out = out.trim();
        (any_set && !out.is_empty()).then(|| out.to_owned())
    }

    /// Every set field with `role = "deadline"`, as `{field: date}` (ADR 0017 §7).
    pub fn deadlines_of(&self, fields: &BTreeMap<String, Value>) -> Map<String, Value> {
        self.fields
            .iter()
            .filter(|f| f.role == Some(Role::Deadline))
            .filter_map(|f| fields.get(&f.name).filter(|v| !v.is_null()).map(|v| (f.name.clone(), v.clone())))
            .collect()
    }

    /// The values in `values` that a unique field would take (ADR 0017 §4). Unset and `""`
    /// never count.
    pub fn unique_values<'a>(&self, values: &'a Map<String, Value>) -> Vec<(&'a str, &'a Value)> {
        values
            .iter()
            .filter(|(k, v)| self.field(k).is_some_and(|f| f.unique) && !v.is_null() && v.as_str() != Some(""))
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }

    /// Check values being written. `null` is not a value; callers that allow it as "unset"
    /// remove those keys first.
    pub fn check_values(&self, values: &Map<String, Value>) -> Result<()> {
        for (name, value) in values {
            let field = self
                .field(name)
                .ok_or_else(|| Error::invalid_params(format!("collection '{}' has no field '{name}'", self.id)))?;
            field.check(value)?;
        }
        Ok(())
    }

    /// Check a set of changes as `records.update` and `records.complete` take them: every key a
    /// field of the collection, every non-null value of its type, `null` meaning "unset".
    pub fn check_changes(&self, changes: &Map<String, Value>) -> Result<()> {
        if let Some(k) = changes.keys().find(|k| self.field(k).is_none()) {
            return Err(Error::invalid_params(format!("collection '{}' has no field '{k}'", self.id)));
        }
        let set: Map<String, Value> =
            changes.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect();
        self.check_values(&set)
    }

    /// Every required field present in a complete set of values.
    pub fn check_required(&self, values: &Map<String, Value>) -> Result<()> {
        match self.fields.iter().find(|f| f.required && values.get(&f.name).is_none_or(Value::is_null)) {
            Some(f) => Err(Error::invalid_params(format!("field '{}' is required", f.name))),
            None => Ok(()),
        }
    }
}

impl Field {
    pub fn check(&self, value: &Value) -> Result<()> {
        let ok = match self.kind {
            FieldType::String => value.is_string(),
            FieldType::Int => value.is_i64(),
            FieldType::Bool => value.is_boolean(),
            FieldType::Date => value.as_str().is_some_and(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()),
            FieldType::Enum => value.as_str().is_some_and(|s| self.values.iter().any(|v| v == s)),
        };
        if ok {
            return Ok(());
        }
        let want = match self.kind {
            FieldType::String => "a string".to_owned(),
            FieldType::Int => "an integer".to_owned(),
            FieldType::Bool => "true or false".to_owned(),
            FieldType::Date => "a date like \"2026-09-25\"".to_owned(),
            FieldType::Enum => format!("one of {}", self.values.join(", ")),
        };
        Err(Error::invalid_params(format!("field '{}' must be {want}, got {value}", self.name)))
    }
}

/// A piece of a `title` template.
#[derive(Debug, PartialEq)]
enum Part<'a> {
    Text(&'a str),
    Field(&'a str),
}

/// Split a `title` template into text and `{field}` parts. A `{` or `}` that isn't part of a
/// `{name}` is an error: there is no escaping and no other syntax (ADR 0017 §2).
fn title_parts(template: &str) -> std::result::Result<Vec<Part<'_>>, String> {
    let mut parts = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find(['{', '}']) {
        if rest[open..].starts_with('}') {
            return Err(format!("unmatched '}}' in \"{template}\""));
        }
        if open > 0 {
            parts.push(Part::Text(&rest[..open]));
        }
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or_else(|| format!("unmatched '{{' in \"{template}\""))?;
        let name = &after[..close];
        if name.contains('{') || !is_valid_name(name) {
            return Err(format!("'{{{name}}}' is not a field name"));
        }
        parts.push(Part::Field(name));
        rest = &after[close + 1..];
    }
    if !rest.is_empty() {
        parts.push(Part::Text(rest));
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const LEETCODE: &str = include_str!("../collections/leetcode.toml");

    fn leetcode() -> Collection {
        Collection::parse("leetcode", LEETCODE).unwrap()
    }

    fn with_fields(fields: &str) -> Result<Collection> {
        Collection::parse("c", &format!("[collection]\nid = \"c\"\nlabel = \"C\"\n{fields}"))
    }

    #[test]
    fn the_built_in_collection_parses() {
        let c = leetcode();
        assert_eq!(c.label, "LeetCode");
        let names: Vec<_> = c.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["title", "difficulty", "url", "last_solved"]);
        assert_eq!(c.stamp_on_complete.as_deref(), Some("last_solved"));
        assert_eq!(c.views, [json!({"kind": "heatmap", "source": "last_solved"})]);
    }

    #[test]
    fn bad_definitions_are_rejected_with_the_file_name() {
        let cases = [
            ("[[field]]\nname = \"id\"\ntype = \"string\"", "reserved"),
            ("[[field]]\nname = \"Title\"\ntype = \"string\"", "invalid"),
            ("[[field]]\nname = \"a\"\ntype = \"string\"\n[[field]]\nname = \"a\"\ntype = \"int\"", "twice"),
            ("[[field]]\nname = \"a\"\ntype = \"enum\"", "needs values"),
            ("[[field]]\nname = \"a\"\ntype = \"enum\"\nvalues = [\"x\", \"x\"]", "repeats"),
            ("[[field]]\nname = \"a\"\ntype = \"int\"\nvalues = [\"x\"]", "only enum"),
            ("[[field]]\nname = \"a\"\ntype = \"float\"", "unknown variant"),
            ("[[field]]\nname = \"a\"\ntype = \"enum\"\nvalue = [\"x\"]", "unknown field"),
        ];
        for (fields, want) in cases {
            let e = with_fields(fields).unwrap_err();
            assert!(e.message.contains("'c.toml'") && e.message.contains(want), "{fields:?}: {}", e.message);
        }
        let e = Collection::parse("other", LEETCODE).unwrap_err();
        assert!(e.message.contains("must match the file name"));
        let e = Collection::parse(
            "c",
            "[collection]\nid = \"c\"\nlabel = \"C\"\nstamp_on_complete = \"a\"\n[[field]]\nname = \"a\"\ntype = \"string\"",
        )
        .unwrap_err();
        assert!(e.message.contains("must name a date field"));
        // ADR 0016 §3: only the two values; anything else names the file.
        let e =
            Collection::parse("c", "[collection]\nid = \"c\"\nlabel = \"C\"\nrepeat_complete = \"twice\"").unwrap_err();
        assert!(e.message.contains("'c.toml'") && e.message.contains("unknown variant"), "{}", e.message);
    }

    // ------------------------------------------------------------ ADR 0017

    const JOBS: &str = r#"
        description = "Jobs I'm tracking"
        title = "{company}: {position}"
        related = ["interviews"]
        stamp_on_complete = "applied_on"

        [[field]]
        name = "company"
        type = "string"
        required = true

        [[field]]
        name = "position"
        type = "string"

        [[field]]
        name = "url"
        type = "string"
        role = "url"
        unique = true

        [[field]]
        name = "round"
        type = "int"

        [[field]]
        name = "oa_deadline"
        type = "date"
        role = "deadline"

        [[field]]
        name = "applied_on"
        type = "date"

        [extra.calendar]
        lead_days = [3, 1]
    "#;

    fn jobs() -> Collection {
        with_fields(JOBS).unwrap()
    }

    fn values(v: Value) -> BTreeMap<String, Value> {
        v.as_object().unwrap().clone().into_iter().collect()
    }

    #[test]
    fn integration_keys_parse_and_are_returned() {
        let c = jobs();
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["description"], "Jobs I'm tracking");
        assert_eq!(json["title"], "{company}: {position}");
        assert_eq!(json["related"], json!(["interviews"]));
        assert_eq!(json["extra"], json!({"calendar": {"lead_days": [3, 1]}}), "passed through untouched");
        let url = json["fields"].as_array().unwrap().iter().find(|f| f["name"] == "url").unwrap();
        assert_eq!((url["role"].clone(), url["unique"].clone()), (json!("url"), json!(true)));
        // Unset keys stay out, so a collection without them looks exactly as before.
        let plain = serde_json::to_value(leetcode()).unwrap();
        for key in ["description", "related", "extra"] {
            assert!(plain.get(key).is_none(), "{key}");
        }
        assert!(plain["fields"][0].get("role").is_none() && plain["fields"][0].get("unique").is_none());
    }

    #[test]
    fn titles_render_from_fields_and_fall_back_to_the_id() {
        let c = jobs();
        assert_eq!(
            c.title_of("x", &values(json!({"company": "Amazon", "position": "SDE Intern"}))),
            "Amazon: SDE Intern"
        );
        assert_eq!(c.title_of("x", &values(json!({"company": "Amazon"}))), "Amazon:", "unset renders as nothing");
        assert_eq!(c.title_of("x", &values(json!({}))), "x", "nothing set: the id, not a bare ':'");
        let numbered = with_fields("title = \"Round {round}\"\n[[field]]\nname = \"round\"\ntype = \"int\"").unwrap();
        assert_eq!(numbered.title_of("r", &values(json!({"round": 3}))), "Round 3");
        assert_eq!(numbered.title_of("r", &values(json!({}))), "r");
        // No template, or an empty result: the id.
        let only = with_fields("title = \"{a}\"\n[[field]]\nname = \"a\"\ntype = \"string\"").unwrap();
        assert_eq!(only.title_of("the-id", &values(json!({}))), "the-id");
        assert_eq!(with_fields("").unwrap().title_of("the-id", &values(json!({"a": "x"}))), "the-id");
        assert_eq!(leetcode().title_of("two-sum", &values(json!({"title": "Two Sum"}))), "Two Sum");
    }

    #[test]
    fn deadlines_are_the_set_deadline_fields() {
        let c = jobs();
        let v = values(json!({"oa_deadline": "2026-10-20", "applied_on": "2026-10-04"}));
        assert_eq!(c.deadlines_of(&v), *json!({"oa_deadline": "2026-10-20"}).as_object().unwrap());
        assert!(c.deadlines_of(&values(json!({"applied_on": "2026-10-04"}))).is_empty());
    }

    #[test]
    fn unique_values_skip_unset_and_empty() {
        let c = jobs();
        let v = json!({"url": "https://x", "company": "Acme"}).as_object().unwrap().clone();
        assert_eq!(c.unique_values(&v), [("url", &json!("https://x"))]);
        for empty in [json!({"url": ""}), json!({"url": null}), json!({"company": "Acme"})] {
            assert!(c.unique_values(empty.as_object().unwrap()).is_empty(), "{empty}");
        }
    }

    #[test]
    fn bad_integration_keys_name_the_file() {
        let cases = [
            ("[[field]]\nname = \"a\"\ntype = \"string\"\nrole = \"deadline\"", "only for date fields"),
            ("[[field]]\nname = \"a\"\ntype = \"date\"\nrole = \"url\"", "only for string fields"),
            ("[[field]]\nname = \"a\"\ntype = \"date\"\nrole = \"birthday\"", "unknown variant"),
            (
                "[[field]]\nname = \"a\"\ntype = \"string\"\nrole = \"url\"\n[[field]]\nname = \"b\"\ntype = \"string\"\nrole = \"url\"",
                "only one field",
            ),
            ("[[field]]\nname = \"a\"\ntype = \"bool\"\nunique = true", "can't be unique"),
            ("related = [\"Not An Id\"]", "not a valid collection id"),
            ("title = \"{nope}\"", "'{nope}' is not a field"),
            ("title = \"{a\"\n[[field]]\nname = \"a\"\ntype = \"string\"", "unmatched '{'"),
            ("title = \"a}\"", "unmatched '}'"),
            ("title = \"{A B}\"", "is not a field name"),
            ("[extra]\nlead_days = 3", "must be a table"),
            (
                "stamp_on_complete = \"d\"\n[[field]]\nname = \"d\"\ntype = \"date\"\nrole = \"deadline\"",
                "is a deadline",
            ),
        ];
        for (text, want) in cases {
            let e = with_fields(text).unwrap_err();
            assert!(e.message.contains("'c.toml'") && e.message.contains(want), "{text:?}: {}", e.message);
        }
    }

    #[test]
    fn repeat_complete_defaults_to_restamp() {
        assert_eq!(leetcode().repeat_complete, RepeatComplete::Restamp);
        let c =
            Collection::parse("c", "[collection]\nid = \"c\"\nlabel = \"C\"\nrepeat_complete = \"refuse\"").unwrap();
        assert_eq!(c.repeat_complete, RepeatComplete::Refuse);
    }

    #[test]
    fn values_are_checked_against_their_type() {
        let c = leetcode();
        let ok = json!({"title": "Two Sum", "difficulty": "easy", "last_solved": "2026-09-25"});
        c.check_values(ok.as_object().unwrap()).unwrap();

        for (bad, want) in [
            (json!({"difficulty": "trivial"}), "one of easy, medium, hard"),
            (json!({"last_solved": "25/09/2026"}), "a date"),
            (json!({"title": 3}), "a string"),
            (json!({"nope": "x"}), "no field 'nope'"),
        ] {
            let e = c.check_values(bad.as_object().unwrap()).unwrap_err();
            assert!(e.message.contains(want), "{bad}: {}", e.message);
        }
    }

    #[test]
    fn required_fields() {
        let c = leetcode();
        assert!(c.check_required(json!({"title": "x"}).as_object().unwrap()).is_ok());
        assert!(c.check_required(json!({"title": null}).as_object().unwrap()).is_err());
        assert!(c.check_required(&Map::new()).unwrap_err().message.contains("'title' is required"));
    }
}
