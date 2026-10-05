//! Changing a collection once it has data (ADR 0021): what doesn't fit its schema, and edits to
//! the collection file that keep the user's comments and layout. Pure: text and values in, text
//! and reports out (§12 rule 10). The ops in `lib.rs` read, call these, and write the result.

use shimmer_core::{Error, Result};
use toml_edit::DocumentMut;

use crate::item::Item;
use crate::schema::Collection;

/// Everything about `item` that doesn't fit `c` (ADR 0021 §2): missing required fields, values
/// of the wrong type, and keys the collection doesn't know. Empty when it all fits.
pub fn problems(c: &Collection, item: &Item) -> Vec<String> {
    let mut out = Vec::new();
    for f in c.fields.iter().filter(|f| f.required) {
        if item.fields.get(&f.name).is_none_or(crate::schema::is_unset) {
            out.push(format!("required field '{}' is missing", f.name));
        }
    }
    for (key, value) in &item.fields {
        match c.field(key) {
            Some(f) => {
                if let Err(e) = f.check(value) {
                    out.push(e.message);
                }
            }
            None => out.push(format!("unknown key '{key}' (not a field of '{}')", c.id)),
        }
    }
    out
}

/// The collection file with field `from` renamed to `to` everywhere the file names it: the
/// field's `name`, `stamp_on_complete`, `{from}` in `title`, and `source` in `[[view]]` tables.
/// Comments and layout are kept (ADR 0021 §3).
pub fn rename_field(file_id: &str, text: &str, from: &str, to: &str) -> Result<String> {
    let mut doc = parse(file_id, text)?;
    let header = doc.get_mut("collection").and_then(toml_edit::Item::as_table_like_mut);
    if let Some(header) = header {
        if let Some(slot) = header.get_mut("stamp_on_complete") {
            if slot.as_str() == Some(from) {
                set_str(slot, to);
            }
        }
        if let Some(slot) = header.get_mut("title") {
            if let Some(title) = slot.as_str() {
                let renamed = title.replace(&format!("{{{from}}}"), &format!("{{{to}}}"));
                set_str(slot, &renamed);
            }
        }
    }
    for (table, key) in [("field", "name"), ("view", "source")] {
        if let Some(tables) = doc.get_mut(table).and_then(toml_edit::Item::as_array_of_tables_mut) {
            for t in tables.iter_mut() {
                if let Some(slot) = t.get_mut(key) {
                    if slot.as_str() == Some(from) {
                        set_str(slot, to);
                    }
                }
            }
        }
    }
    Ok(doc.to_string())
}

/// The collection file with `[collection] id` set to `new_id`, comments kept (ADR 0021 §4).
pub fn set_id(file_id: &str, text: &str, new_id: &str) -> Result<String> {
    let mut doc = parse(file_id, text)?;
    let slot = doc
        .get_mut("collection")
        .and_then(toml_edit::Item::as_table_like_mut)
        .and_then(|h| h.get_mut("id"))
        .ok_or_else(|| Error::invalid_params(format!("collection file '{file_id}.toml' has no [collection] id")))?;
    set_str(slot, new_id);
    Ok(doc.to_string())
}

/// The collection file with `[collection] label` set, comments kept (ADR 0022 §2).
pub fn set_label(file_id: &str, text: &str, label: &str) -> Result<String> {
    let mut doc = parse(file_id, text)?;
    let slot = doc
        .get_mut("collection")
        .and_then(toml_edit::Item::as_table_like_mut)
        .and_then(|h| h.get_mut("label"))
        .ok_or_else(|| Error::invalid_params(format!("collection file '{file_id}.toml' has no [collection] label")))?;
    set_str(slot, label);
    Ok(doc.to_string())
}

fn parse(file_id: &str, text: &str) -> Result<DocumentMut> {
    text.parse::<DocumentMut>().map_err(|e| Error::invalid_params(format!("collection file '{file_id}.toml': {e}")))
}

/// Replace a string value, keeping the spacing and comment around it.
fn set_str(slot: &mut toml_edit::Item, s: &str) {
    if let Some(value) = slot.as_value_mut() {
        let decor = value.decor().clone();
        *value = toml_edit::Value::from(s);
        *value.decor_mut() = decor;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;

    const JOBS: &str = r#"# My job tracker. Edit freely.

[collection]
id = "jobs"
label = "Jobs"
title = "{company}: {recruiter}"   # what reminders call a job
stamp_on_complete = "applied_on"

[[field]]
name = "company"
type = "string"
required = true

[[field]]
name = "recruiter"   # who I talk to
type = "string"

[[field]]
name = "stage"
type = "enum"
values = ["oa", "interview"]

[[field]]
name = "applied_on"
type = "date"

[[view]]
kind = "heatmap"
source = "applied_on"
"#;

    fn item(fields: Value) -> Item {
        Item::new("acme", fields.as_object().unwrap().clone())
    }

    #[test]
    fn problems_name_what_does_not_fit() {
        let c = Collection::parse("jobs", JOBS).unwrap();
        assert!(problems(&c, &item(json!({"company": "Acme", "stage": "oa"}))).is_empty());
        let found = problems(&c, &item(json!({"stage": "ghosted", "applied_on": "last week", "contact": "Jay"})));
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(found[0].contains("required field 'company' is missing"));
        assert!(found.iter().any(|p| p.contains("'stage' must be one of oa, interview")));
        assert!(found.iter().any(|p| p.contains("'applied_on' must be a date")));
        assert!(found.iter().any(|p| p == "unknown key 'contact' (not a field of 'jobs')"));
    }

    #[test]
    fn rename_field_changes_every_mention_and_keeps_comments() {
        let out = rename_field("jobs", JOBS, "recruiter", "contact").unwrap();
        assert!(out.contains("name = \"contact\"   # who I talk to"), "{out}");
        assert!(out.contains("title = \"{company}: {contact}\"   # what reminders call a job"), "{out}");
        assert!(out.starts_with("# My job tracker. Edit freely."));
        assert!(!out.contains("recruiter"), "{out}");
        Collection::parse("jobs", &out).expect("still a valid collection");

        let out = rename_field("jobs", JOBS, "applied_on", "sent_on").unwrap();
        assert!(out.contains("stamp_on_complete = \"sent_on\"") && out.contains("source = \"sent_on\""), "{out}");
        Collection::parse("jobs", &out).unwrap();
    }

    #[test]
    fn set_id_changes_only_the_id() {
        let out = set_id("jobs", JOBS, "applications").unwrap();
        assert!(out.contains("id = \"applications\""));
        assert_eq!(out.replace("id = \"applications\"", "id = \"jobs\""), JOBS);
        Collection::parse("applications", &out).unwrap();
    }
}
