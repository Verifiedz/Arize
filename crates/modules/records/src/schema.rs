//! Collection definitions: the TOML that turns the generic record store into a LeetCode tracker
//! or a job-application tracker (CLAUDE.md §8). Pure data and pure functions (§12 rule 10).

use std::collections::BTreeSet;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use swe_core::ids::is_valid_name;
use swe_core::{Error, Result};

use crate::item::toml_to_json;

/// Keys every item has on the wire; a field may not take these names.
pub const RESERVED: [&str; 2] = ["id", "status"];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Collection {
    pub id: String,
    pub label: String,
    pub fields: Vec<Field>,
    /// The date field `records.complete` sets to today.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stamp_on_complete: Option<String>,
    /// Opaque here, passed through for clients. There is no view type yet (§5).
    pub views: Vec<Value>,
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    id: String,
    label: String,
    #[serde(default)]
    stamp_on_complete: Option<String>,
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
        }

        let c = Self {
            id: file.collection.id,
            label: file.collection.label,
            fields: file.field,
            stamp_on_complete: file.collection.stamp_on_complete,
            views: file.view.into_iter().map(toml_to_json).collect(),
        };
        if let Some(stamp) = &c.stamp_on_complete {
            if c.field(stamp).map(|f| f.kind) != Some(FieldType::Date) {
                return Err(bad(format!("stamp_on_complete '{stamp}' must name a date field")));
            }
        }
        Ok(c)
    }

    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
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

    /// Every required field present in a complete set of values.
    pub fn check_required(&self, values: &Map<String, Value>) -> Result<()> {
        match self.fields.iter().find(|f| f.required && values.get(&f.name).is_none_or(Value::is_null)) {
            Some(f) => Err(Error::invalid_params(format!("field '{}' is required", f.name))),
            None => Ok(()),
        }
    }

    /// A filter may name `id`, `status` or any field; anything else is a typo, not a wildcard.
    pub fn check_filter(&self, filter: &Map<String, Value>) -> Result<()> {
        match filter.keys().find(|k| !RESERVED.contains(&k.as_str()) && self.field(k).is_none()) {
            Some(k) => Err(Error::invalid_params(format!("cannot filter '{}' on unknown field '{k}'", self.id))),
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
    fn required_fields_and_filters() {
        let c = leetcode();
        assert!(c.check_required(json!({"title": "x"}).as_object().unwrap()).is_ok());
        assert!(c.check_required(json!({"title": null}).as_object().unwrap()).is_err());
        assert!(c.check_required(&Map::new()).unwrap_err().message.contains("'title' is required"));

        assert!(c
            .check_filter(json!({"status": "todo", "id": "x", "difficulty": "easy"}).as_object().unwrap())
            .is_ok());
        assert!(c.check_filter(json!({"dificulty": "easy"}).as_object().unwrap()).is_err());
    }
}
