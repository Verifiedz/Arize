//! `shimmer records …`: typed commands over the `records.*` ops (ADR 0008).
//!
//! Nothing here knows about LeetCode or any other collection. Field flags (`--title`,
//! `--company`) and their types come from the daemon at runtime (`records.collections`), so a
//! collection added as a TOML file gets its own flags with no client change (§9).

use std::fmt::Write as _;

use serde_json::{json, Map, Value};
use shimmer_core::{Error, Result};

use crate::client::Client;
use crate::render::{self, cell, table};

pub const USAGE: &str = "usage: shimmer records <command>

commands:
  collections                          list collections and their fields
  add COLLECTION [ID] [--FIELD VALUE]… add a record, e.g.
                                         shimmer records add leetcode --title \"Two Sum\" --difficulty easy
                                       without an ID, one is made from its title (two-sum)
  list COLLECTION [--FIELD VALUE]…     list records; each --FIELD filters on an exact value
       [--status todo|done] [--limit N] [--offset N]
  get COLLECTION/ID                    show one record
  update COLLECTION/ID [--FIELD VALUE]… [--unset FIELD]…
  complete COLLECTION/ID [--FIELD VALUE]… [--unset FIELD]…
                                       mark done and stamp the collection's date field;
                                       a value for the date field back-dates it
  reopen COLLECTION/ID [--clear-stamp] mark a done record todo again
  rename COLLECTION/ID NEW_ID          give a record a new id
  remove COLLECTION/ID                 delete a record

A record can be written COLLECTION/ID or COLLECTION ID. Field names come from the
collection: see 'shimmer records collections'. Add --json to any command for raw output.";

/// `--name value` pairs, in the order given.
pub type Flags = Vec<(String, String)>;

#[derive(Clone, Debug, PartialEq)]
pub enum RecordsCmd {
    Help,
    Collections,
    /// `id` is `None` when the daemon should make one from the title (ADR 0018).
    Add {
        collection: String,
        id: Option<String>,
        set: Flags,
    },
    Rename {
        collection: String,
        id: String,
        new_id: String,
    },
    List {
        collection: String,
        filter: Flags,
        limit: Option<usize>,
        offset: Option<usize>,
    },
    Get {
        collection: String,
        id: String,
    },
    Update {
        collection: String,
        id: String,
        set: Flags,
        unset: Vec<String>,
    },
    Complete {
        collection: String,
        id: String,
        set: Flags,
        unset: Vec<String>,
    },
    Reopen {
        collection: String,
        id: String,
        clear_stamp: bool,
    },
    Remove {
        collection: String,
        id: String,
    },
}

// ---------------------------------------------------------------- parsing

/// `words` are what follows `shimmer records`, with global flags already removed.
pub fn parse(words: Vec<String>) -> std::result::Result<RecordsCmd, String> {
    let mut words = words.into_iter();
    let Some(sub) = words.next() else { return Ok(RecordsCmd::Help) };
    let mut rest: Vec<String> = words.collect();
    // `--clear-stamp` is the one flag here that takes no value.
    let clear_stamp = sub == "reopen" && rest.iter().any(|w| w == "--clear-stamp");
    if clear_stamp {
        rest.retain(|w| w != "--clear-stamp");
    }
    let (positional, flags) = split(rest)?;
    let no_flags = || match flags.first() {
        Some((f, _)) => Err(format!("'records {sub}' takes no options, got '--{f}'")),
        None => Ok(()),
    };
    match sub.as_str() {
        "help" => Ok(RecordsCmd::Help),
        "collections" => {
            no_flags()?;
            match positional.is_empty() {
                true => Ok(RecordsCmd::Collections),
                false => Err("'records collections' takes no arguments".into()),
            }
        }
        "add" => {
            // `COLLECTION` alone (no `/`) leaves the id to the daemon (ADR 0018).
            let (collection, id) = match positional.as_slice() {
                [one] if !one.contains('/') => (one.clone(), None),
                _ => target(&sub, positional).map(|(c, id)| (c, Some(id)))?,
            };
            Ok(RecordsCmd::Add { collection, id, set: flags })
        }
        "rename" => {
            no_flags()?;
            let usage = "usage: shimmer records rename COLLECTION/ID NEW_ID";
            let (target_words, new_id) = match positional.split_last() {
                Some((new_id, rest)) if !rest.is_empty() => (rest.to_vec(), new_id.clone()),
                _ => return Err(usage.into()),
            };
            let (collection, id) = target(&sub, target_words).map_err(|_| usage.to_owned())?;
            Ok(RecordsCmd::Rename { collection, id, new_id })
        }
        "list" => {
            let [collection]: [String; 1] =
                positional.try_into().map_err(|_| "usage: shimmer records list COLLECTION [--FIELD VALUE]…")?;
            let (mut filter, mut limit, mut offset) = (Vec::new(), None, None);
            for (name, value) in flags {
                match name.as_str() {
                    "limit" => limit = Some(number("--limit", &value)?),
                    "offset" => offset = Some(number("--offset", &value)?),
                    _ => filter.push((name, value)),
                }
            }
            Ok(RecordsCmd::List { collection, filter, limit, offset })
        }
        "update" | "complete" => {
            let (collection, id) = target(&sub, positional)?;
            let (unset, set) = flags.into_iter().partition::<Vec<_>, _>(|(name, _)| name == "unset");
            let unset: Vec<String> = unset.into_iter().map(|(_, field)| field).collect();
            if sub == "complete" {
                return Ok(RecordsCmd::Complete { collection, id, set, unset });
            }
            if set.is_empty() && unset.is_empty() {
                return Err("nothing to update: give --FIELD VALUE or --unset FIELD".into());
            }
            Ok(RecordsCmd::Update { collection, id, set, unset })
        }
        "get" | "remove" | "reopen" => {
            no_flags()?;
            let (collection, id) = target(&sub, positional)?;
            Ok(match sub.as_str() {
                "get" => RecordsCmd::Get { collection, id },
                "reopen" => RecordsCmd::Reopen { collection, id, clear_stamp },
                _ => RecordsCmd::Remove { collection, id },
            })
        }
        other => Err(format!("unknown records command '{other}'; see 'shimmer records --help'")),
    }
}

/// Separate positional words from `--name value` / `--name=value` pairs. Every flag here
/// takes a value, so the word after a flag is always its value, even if it starts with `-`.
fn split(words: Vec<String>) -> std::result::Result<(Vec<String>, Flags), String> {
    let (mut positional, mut flags) = (Vec::new(), Vec::new());
    let mut words = words.into_iter();
    while let Some(word) = words.next() {
        let Some(flag) = word.strip_prefix("--") else {
            if word.starts_with('-') && word.len() > 1 {
                return Err(format!("unknown option '{word}'"));
            }
            positional.push(word);
            continue;
        };
        let (name, value) = match flag.split_once('=') {
            Some((n, v)) => (n.to_owned(), v.to_owned()),
            None => (flag.to_owned(), words.next().ok_or(format!("--{flag} needs a value"))?),
        };
        if name.is_empty() {
            return Err("'--' is not an option".into());
        }
        flags.push((name, value));
    }
    Ok((positional, flags))
}

/// `COLLECTION/ID` or `COLLECTION ID`.
fn target(sub: &str, positional: Vec<String>) -> std::result::Result<(String, String), String> {
    let usage = || format!("usage: shimmer records {sub} COLLECTION/ID");
    match positional.as_slice() {
        [one] => match one.split_once('/') {
            Some((c, id)) if !c.is_empty() && !id.is_empty() => Ok((c.to_owned(), id.to_owned())),
            _ => Err(usage()),
        },
        [c, id] => Ok((c.clone(), id.clone())),
        _ => Err(usage()),
    }
}

fn number(flag: &str, value: &str) -> std::result::Result<usize, String> {
    value.parse().map_err(|_| format!("{flag} expects a whole number, got '{value}'"))
}

// ---------------------------------------------------------------- requests

impl RecordsCmd {
    fn collection(&self) -> Option<&str> {
        match self {
            Self::Help | Self::Collections => None,
            Self::Add { collection, .. }
            | Self::Rename { collection, .. }
            | Self::List { collection, .. }
            | Self::Get { collection, .. }
            | Self::Update { collection, .. }
            | Self::Complete { collection, .. }
            | Self::Reopen { collection, .. }
            | Self::Remove { collection, .. } => Some(collection),
        }
    }
}

/// The op and params for `cmd`. `schema` is the collection's definition from
/// `records.collections`, or `Null` when there is none: then values go as plain strings and
/// the daemon, which owns validation, reports what is wrong.
pub fn request(cmd: &RecordsCmd, schema: &Value) -> Result<(&'static str, Value)> {
    let fields = |pairs: &[(String, String)]| -> Result<Map<String, Value>> {
        pairs.iter().map(|(name, raw)| typed(schema, name, raw)).collect()
    };
    // `--FIELD VALUE` sets, `--unset FIELD` sends `null`.
    let changes = |set: &[(String, String)], unset: &[String]| -> Result<Map<String, Value>> {
        let mut changes = fields(set)?;
        for name in unset {
            changes.insert(field_name(schema, name), Value::Null);
        }
        Ok(changes)
    };
    Ok(match cmd {
        RecordsCmd::Help => return Err(Error::internal("help is not a request")),
        RecordsCmd::Collections => ("records.collections", json!({})),
        RecordsCmd::Add { collection, id, set } => {
            let mut params = json!({"collection": collection, "fields": fields(set)?});
            if let Some(id) = id {
                params["id"] = json!(id);
            }
            ("records.add", params)
        }
        RecordsCmd::Rename { collection, id, new_id } => {
            ("records.rename", json!({"collection": collection, "id": id, "new_id": new_id}))
        }
        RecordsCmd::List { collection, filter, limit, offset } => {
            let mut params = json!({"collection": collection, "filter": fields(filter)?});
            if let Some(n) = limit {
                params["limit"] = json!(n);
            }
            if let Some(n) = offset {
                params["offset"] = json!(n);
            }
            ("records.list", params)
        }
        RecordsCmd::Get { collection, id } => ("records.get", json!({"collection": collection, "id": id})),
        RecordsCmd::Update { collection, id, set, unset } => {
            ("records.update", json!({"collection": collection, "id": id, "fields": changes(set, unset)?}))
        }
        RecordsCmd::Complete { collection, id, set, unset } => {
            let mut params = json!({"collection": collection, "id": id});
            let changes = changes(set, unset)?;
            if !changes.is_empty() {
                params["fields"] = Value::Object(changes);
            }
            ("records.complete", params)
        }
        RecordsCmd::Reopen { collection, id, clear_stamp } => {
            let mut params = json!({"collection": collection, "id": id});
            if *clear_stamp {
                params["clear_stamp"] = json!(true);
            }
            ("records.reopen", params)
        }
        RecordsCmd::Remove { collection, id } => ("records.remove", json!({"collection": collection, "id": id})),
    })
}

/// `--last-solved` finds a field named `last_solved`; an exact match wins.
fn field_name(schema: &Value, flag: &str) -> String {
    let known = |n: &str| field(schema, n).is_some();
    if known(flag) {
        return flag.to_owned();
    }
    let underscored = flag.replace('-', "_");
    if known(&underscored) {
        underscored
    } else {
        flag.to_owned()
    }
}

fn field<'a>(schema: &'a Value, name: &str) -> Option<&'a Value> {
    schema["fields"].as_array()?.iter().find(|f| f["name"] == name)
}

/// A command-line value as the JSON type its field declares. `status` and `id` are strings.
fn typed(schema: &Value, flag: &str, raw: &str) -> Result<(String, Value)> {
    let name = field_name(schema, flag);
    let kind = field(schema, &name).and_then(|f| f["type"].as_str()).unwrap_or("string");
    let bad = |want: &str| Error::invalid_params(format!("--{flag} expects {want}, got '{raw}'"));
    let value = match kind {
        "int" => Value::from(raw.parse::<i64>().map_err(|_| bad("a whole number"))?),
        "bool" => Value::Bool(match raw {
            "true" | "yes" | "y" | "1" => true,
            "false" | "no" | "n" | "0" => false,
            _ => return Err(bad("true or false")),
        }),
        _ => Value::String(raw.to_owned()),
    };
    Ok((name, value))
}

// ---------------------------------------------------------------- running

/// Fetch the collection's schema, send the op, render the reply.
pub async fn run(client: &mut Client, cmd: &RecordsCmd, json: bool) -> Result<String> {
    let schema = match cmd.collection() {
        Some(id) => schema(client, id).await?,
        None => Value::Null,
    };
    let (op, params) = request(cmd, &schema)?;
    let data = client.call(op, params).await?;
    Ok(if json { render::json(&data) } else { show(cmd, &data, &schema) })
}

async fn schema(client: &mut Client, id: &str) -> Result<Value> {
    let all = client.call("records.collections", json!({})).await?;
    let found = all["collections"].as_array().and_then(|cs| cs.iter().find(|c| c["id"] == id)).cloned();
    Ok(found.unwrap_or(Value::Null))
}

// ---------------------------------------------------------------- output

/// Text for a successful reply. Everything shown comes from `data` or the schema; the client
/// computes nothing (§2).
pub fn show(cmd: &RecordsCmd, data: &Value, schema: &Value) -> String {
    match cmd {
        RecordsCmd::Help => USAGE.into(),
        RecordsCmd::Collections => collections(data),
        // The id comes from the reply: the daemon may have made it (ADR 0018).
        RecordsCmd::Add { collection, .. } => format!("added {collection}/{}", cell(&data["id"])),
        RecordsCmd::Rename { collection, id, new_id } => {
            format!("✓ {collection}/{id} renamed to {collection}/{new_id}")
        }
        RecordsCmd::Update { collection, id, .. } => format!("updated {collection}/{id}"),
        RecordsCmd::Remove { collection, id } => format!("removed {collection}/{id}"),
        RecordsCmd::Complete { collection, id, .. } => {
            let mut out = format!("✓ {collection}/{id} done");
            if let Some(stamp) = schema["stamp_on_complete"].as_str() {
                let _ = write!(out, " ({stamp} {})", cell(&data[stamp]));
            }
            out
        }
        RecordsCmd::Reopen { collection, id, clear_stamp } => {
            let mut out = format!("✓ {collection}/{id} reopened (todo)");
            if let (true, Some(stamp)) = (*clear_stamp, schema["stamp_on_complete"].as_str()) {
                let _ = write!(out, "; cleared {stamp}");
            }
            out
        }
        RecordsCmd::Get { collection, id } => item(collection, id, data, schema),
        RecordsCmd::List { .. } => list(data, schema),
    }
}

fn collections(data: &Value) -> String {
    let rows: Vec<Vec<String>> = data["collections"]
        .as_array()
        .map(|cs| {
            cs.iter()
                .map(|c| {
                    let fields = c["fields"]
                        .as_array()
                        .map(|fs| fs.iter().map(describe_field).collect::<Vec<_>>().join(", "))
                        .unwrap_or_default();
                    vec![cell(&c["id"]), cell(&c["label"]), fields]
                })
                .collect()
        })
        .unwrap_or_default();
    let mut out = match rows.is_empty() {
        true => "no collections".to_owned(),
        false => table(&["ID".into(), "LABEL".into(), "FIELDS".into()], &rows),
    };
    // In full, not cut like a table cell: the message is how the user finds the broken line.
    for skipped in data["skipped"].as_array().into_iter().flatten() {
        let _ = write!(out, "\nskipped: {}", skipped.as_str().unwrap_or_default());
    }
    out
}

/// `difficulty (easy|medium|hard)`, `title*` for required, `last_solved (date)`, and a field's
/// role and uniqueness (ADR 0017): `url (url, unique)`, `oa_deadline (date, deadline)`.
fn describe_field(f: &Value) -> String {
    let mut s = cell(&f["name"]);
    if f["required"] == true {
        s.push('*');
    }
    let mut notes = Vec::new();
    match f["type"].as_str() {
        Some("enum") => {
            notes.push(f["values"].as_array().into_iter().flatten().map(cell).collect::<Vec<_>>().join("|"))
        }
        Some("string") | None => {}
        Some(other) => notes.push(other.to_owned()),
    }
    if let Some(role) = f["role"].as_str() {
        notes.push(role.to_owned());
    }
    if f["unique"] == true {
        notes.push("unique".to_owned());
    }
    if !notes.is_empty() {
        let _ = write!(s, " ({})", notes.join(", "));
    }
    s
}

fn item(collection: &str, id: &str, data: &Value, schema: &Value) -> String {
    let mut out = format!("{collection}/{id}  ({})", cell(&data["status"]));
    let keys = columns(data, schema);
    let width = keys.iter().map(|k| k.chars().count()).max().unwrap_or(0);
    for key in keys {
        let _ = write!(out, "\n  {key:width$}  {}", cell(&data[key.as_str()]));
    }
    out
}

fn list(data: &Value, schema: &Value) -> String {
    let items = data["items"].as_array().cloned().unwrap_or_default();
    let total = data["total"].as_u64().unwrap_or(items.len() as u64);
    let mut out = if items.is_empty() {
        "no records".to_owned()
    } else {
        let keys = columns(&items[0], schema);
        let mut header = vec!["ID".to_owned(), "STATUS".to_owned()];
        header.extend(keys.iter().map(|k| k.to_uppercase().replace('_', " ")));
        let rows: Vec<Vec<String>> = items
            .iter()
            .map(|i| {
                let mut row = vec![cell(&i["id"]), cell(&i["status"])];
                row.extend(keys.iter().map(|k| cell(&i[k.as_str()])));
                row
            })
            .collect();
        let mut t = table(&header, &rows);
        let count = match (items.len() as u64, total) {
            (1, 1) => "1 record".to_owned(),
            (n, t) if n == t => format!("{n} records"),
            (n, t) => format!("{n} of {t}"),
        };
        let _ = write!(t, "\n{count}");
        t
    };
    for skipped in data["skipped"].as_array().into_iter().flatten() {
        let _ = write!(out, "\nskipped: {}", cell(skipped));
    }
    out
}

/// Field columns in schema order, then any extra keys the item carries.
fn columns(item: &Value, schema: &Value) -> Vec<String> {
    let mut keys: Vec<String> = schema["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["name"].as_str().map(String::from))
        .collect();
    if let Some(obj) = item.as_object() {
        for k in obj.keys() {
            if k != "id" && k != "status" && !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> std::result::Result<RecordsCmd, String> {
        parse(words.iter().map(|s| s.to_string()).collect())
    }

    fn pairs(p: &[(&str, &str)]) -> Flags {
        p.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    fn leetcode() -> Value {
        json!({
            "id": "leetcode", "label": "LeetCode", "stamp_on_complete": "last_solved",
            "fields": [
                {"name": "title", "type": "string", "required": true},
                {"name": "difficulty", "type": "enum", "values": ["easy", "medium", "hard"], "required": false},
                {"name": "url", "type": "string", "required": false},
                {"name": "last_solved", "type": "date", "required": false},
                {"name": "attempts", "type": "int", "required": false},
                {"name": "starred", "type": "bool", "required": false}
            ]
        })
    }

    #[test]
    fn targets_both_spellings() {
        let want =
            RecordsCmd::Complete { collection: "leetcode".into(), id: "two-sum".into(), set: vec![], unset: vec![] };
        assert_eq!(parse_words(&["complete", "leetcode/two-sum"]).unwrap(), want);
        assert_eq!(parse_words(&["complete", "leetcode", "two-sum"]).unwrap(), want);
        for bad in [&["complete"][..], &["complete", "leetcode"], &["complete", "/x"], &["complete", "a", "b", "c"]] {
            assert!(parse_words(bad).unwrap_err().contains("COLLECTION/ID"), "{bad:?}");
        }
    }

    #[test]
    fn add_and_update_take_field_flags() {
        assert_eq!(
            parse_words(&["add", "leetcode", "two-sum", "--title", "Two Sum", "--difficulty=easy"]).unwrap(),
            RecordsCmd::Add {
                collection: "leetcode".into(),
                id: Some("two-sum".into()),
                set: pairs(&[("title", "Two Sum"), ("difficulty", "easy")])
            }
        );
        assert_eq!(
            parse_words(&["update", "leetcode/two-sum", "--url", "https://x", "--unset", "difficulty"]).unwrap(),
            RecordsCmd::Update {
                collection: "leetcode".into(),
                id: "two-sum".into(),
                set: pairs(&[("url", "https://x")]),
                unset: vec!["difficulty".into()]
            }
        );
        assert!(parse_words(&["update", "leetcode/two-sum"]).unwrap_err().contains("nothing to update"));
        assert!(parse_words(&["add", "leetcode/x", "--title"]).unwrap_err().contains("needs a value"));
        // A value may look like a flag: the word after --attempts is its value.
        assert_eq!(
            parse_words(&["add", "leetcode/x", "--attempts", "-1"]).unwrap(),
            RecordsCmd::Add { collection: "leetcode".into(), id: Some("x".into()), set: pairs(&[("attempts", "-1")]) }
        );
    }

    #[test]
    fn list_filters_and_pages() {
        assert_eq!(
            parse_words(&["list", "leetcode", "--status", "todo", "--difficulty", "hard", "--limit", "5"]).unwrap(),
            RecordsCmd::List {
                collection: "leetcode".into(),
                filter: pairs(&[("status", "todo"), ("difficulty", "hard")]),
                limit: Some(5),
                offset: None
            }
        );
        assert!(parse_words(&["list"]).is_err());
        assert!(parse_words(&["list", "leetcode", "--limit", "lots"]).unwrap_err().contains("whole number"));
    }

    #[test]
    fn complete_takes_field_flags_and_reopen_takes_clear_stamp() {
        let complete =
            parse_words(&["complete", "leetcode/x", "--last-solved", "2026-10-03", "--unset", "url"]).unwrap();
        assert_eq!(
            complete,
            RecordsCmd::Complete {
                collection: "leetcode".into(),
                id: "x".into(),
                set: pairs(&[("last-solved", "2026-10-03")]),
                unset: vec!["url".into()]
            }
        );
        let (op, params) = request(&complete, &leetcode()).unwrap();
        assert_eq!(
            (op, params),
            (
                "records.complete",
                json!({"collection": "leetcode", "id": "x", "fields": {"last_solved": "2026-10-03", "url": null}})
            )
        );
        // No flags: the same request as before ADR 0016.
        let (_, params) = request(&parse_words(&["complete", "leetcode/x"]).unwrap(), &leetcode()).unwrap();
        assert_eq!(params, json!({"collection": "leetcode", "id": "x"}));

        for words in [&["reopen", "leetcode/x", "--clear-stamp"][..], &["reopen", "--clear-stamp", "leetcode", "x"]] {
            let reopen = parse_words(words).unwrap();
            assert_eq!(reopen, RecordsCmd::Reopen { collection: "leetcode".into(), id: "x".into(), clear_stamp: true });
            let (op, params) = request(&reopen, &leetcode()).unwrap();
            assert_eq!(
                (op, params),
                ("records.reopen", json!({"collection": "leetcode", "id": "x", "clear_stamp": true}))
            );
        }
        let (_, params) = request(&parse_words(&["reopen", "leetcode/x"]).unwrap(), &leetcode()).unwrap();
        assert_eq!(params, json!({"collection": "leetcode", "id": "x"}));
        assert!(parse_words(&["reopen", "leetcode/x", "--title", "t"]).unwrap_err().contains("takes no options"));
        // `--clear-stamp` means nothing to other commands.
        assert!(parse_words(&["get", "leetcode/x", "--clear-stamp"]).unwrap_err().contains("needs a value"));
    }

    #[test]
    fn reopen_output_says_what_it_cleared() {
        let item = json!({"id": "x", "status": "todo"});
        let keep = RecordsCmd::Reopen { collection: "leetcode".into(), id: "x".into(), clear_stamp: false };
        assert_eq!(show(&keep, &item, &leetcode()), "✓ leetcode/x reopened (todo)");
        let clear = RecordsCmd::Reopen { collection: "leetcode".into(), id: "x".into(), clear_stamp: true };
        assert_eq!(show(&clear, &item, &leetcode()), "✓ leetcode/x reopened (todo); cleared last_solved");
    }

    #[test]
    fn add_may_leave_the_id_to_the_daemon() {
        let add = parse_words(&["add", "leetcode", "--title", "Two Sum"]).unwrap();
        assert_eq!(
            add,
            RecordsCmd::Add { collection: "leetcode".into(), id: None, set: pairs(&[("title", "Two Sum")]) }
        );
        let (op, params) = request(&add, &leetcode()).unwrap();
        assert_eq!((op, params), ("records.add", json!({"collection": "leetcode", "fields": {"title": "Two Sum"}})));
        // The reply names the id it chose.
        assert_eq!(show(&add, &json!({"id": "two-sum"}), &leetcode()), "added leetcode/two-sum");
    }

    #[test]
    fn rename_takes_a_target_and_a_new_id() {
        let want = RecordsCmd::Rename { collection: "jobs".into(), id: "amzon".into(), new_id: "amazon".into() };
        assert_eq!(parse_words(&["rename", "jobs/amzon", "amazon"]).unwrap(), want);
        assert_eq!(parse_words(&["rename", "jobs", "amzon", "amazon"]).unwrap(), want);
        assert_eq!(
            request(&want, &Value::Null).unwrap(),
            ("records.rename", json!({"collection": "jobs", "id": "amzon", "new_id": "amazon"}))
        );
        assert_eq!(show(&want, &json!({}), &Value::Null), "✓ jobs/amzon renamed to jobs/amazon");
        for bad in [&["rename"][..], &["rename", "jobs/amzon"], &["rename", "jobs", "a", "b", "c"]] {
            assert!(parse_words(bad).unwrap_err().contains("NEW_ID"), "{bad:?}");
        }
    }

    #[test]
    fn misuse() {
        assert_eq!(parse_words(&[]).unwrap(), RecordsCmd::Help);
        assert!(parse_words(&["frob"]).unwrap_err().contains("unknown records command"));
        assert!(parse_words(&["get", "leetcode/x", "--title", "t"]).unwrap_err().contains("takes no options"));
        assert!(parse_words(&["collections", "extra"]).is_err());
        assert!(parse_words(&["add", "leetcode/x", "-t", "T"]).unwrap_err().contains("unknown option"));
    }

    #[test]
    fn values_get_their_field_types() {
        let add = parse_words(&[
            "add",
            "leetcode/x",
            "--title",
            "7",
            "--attempts",
            "3",
            "--starred",
            "yes",
            "--last-solved",
            "2026-09-25",
        ])
        .unwrap();
        let (op, params) = request(&add, &leetcode()).unwrap();
        assert_eq!(op, "records.add");
        assert_eq!(
            params["fields"],
            json!({"title": "7", "attempts": 3, "starred": true, "last_solved": "2026-09-25"}),
            "strings stay strings, ints and bools convert, --last-solved finds last_solved"
        );

        let bad = parse_words(&["add", "leetcode/x", "--attempts", "three"]).unwrap();
        let e = request(&bad, &leetcode()).unwrap_err();
        assert_eq!(e.message, "--attempts expects a whole number, got 'three'");

        // Without a schema everything goes as a string and the daemon judges it.
        let (_, params) = request(&bad, &Value::Null).unwrap();
        assert_eq!(params["fields"], json!({"attempts": "three"}));
    }

    #[test]
    fn list_and_update_requests() {
        let list = parse_words(&["list", "leetcode", "--status", "done", "--limit", "2", "--offset", "4"]).unwrap();
        let (op, params) = request(&list, &leetcode()).unwrap();
        assert_eq!(
            (op, params),
            ("records.list", json!({"collection": "leetcode", "filter": {"status": "done"}, "limit": 2, "offset": 4}))
        );

        let update = parse_words(&["update", "leetcode/x", "--unset", "last-solved"]).unwrap();
        let (_, params) = request(&update, &leetcode()).unwrap();
        assert_eq!(params["fields"], json!({"last_solved": null}));
    }

    #[test]
    fn list_renders_a_table_in_schema_order() {
        let data = json!({"total": 3, "items": [
            {"id": "two-sum", "status": "done", "title": "Two Sum", "difficulty": "easy", "url": null,
             "last_solved": "2026-09-18", "attempts": null, "starred": null},
            {"id": "lru-cache", "status": "todo", "title": "LRU Cache", "difficulty": "medium", "url": null,
             "last_solved": null, "attempts": 2, "starred": true}
        ]});
        let cmd = parse_words(&["list", "leetcode", "--limit", "2"]).unwrap();
        let want = "\
ID         STATUS  TITLE      DIFFICULTY  URL  LAST SOLVED  ATTEMPTS  STARRED
two-sum    done    Two Sum    easy        -    2026-09-18   -         -
lru-cache  todo    LRU Cache  medium      -    -            2         true
2 of 3";
        assert_eq!(show(&cmd, &data, &leetcode()), want);

        let empty = json!({"items": [], "total": 0, "skipped": ["record file 'x.toml': bad"]});
        assert_eq!(show(&cmd, &empty, &leetcode()), "no records\nskipped: record file 'x.toml': bad");
    }

    #[test]
    fn single_item_output() {
        let item = json!({"id": "two-sum", "status": "done", "title": "Two Sum", "difficulty": "easy",
            "url": null, "last_solved": "2026-09-25", "attempts": null, "starred": null});
        let complete =
            RecordsCmd::Complete { collection: "leetcode".into(), id: "two-sum".into(), set: vec![], unset: vec![] };
        assert_eq!(show(&complete, &item, &leetcode()), "✓ leetcode/two-sum done (last_solved 2026-09-25)");

        let get = RecordsCmd::Get { collection: "leetcode".into(), id: "two-sum".into() };
        assert!(show(&get, &item, &leetcode()).starts_with("leetcode/two-sum  (done)\n  title        Two Sum\n"));
    }

    #[test]
    fn collections_describe_their_fields() {
        let data = json!({"collections": [leetcode()]});
        let out = show(&RecordsCmd::Collections, &data, &Value::Null);
        assert!(
            out.contains("leetcode  LeetCode  title*, difficulty (easy|medium|hard), url, last_solved (date)"),
            "{out}"
        );
    }

    #[test]
    fn collections_show_roles_and_unique() {
        let jobs = json!({"id": "jobs", "label": "Jobs", "fields": [
            {"name": "company", "type": "string", "required": true},
            {"name": "url", "type": "string", "required": false, "role": "url", "unique": true},
            {"name": "stage", "type": "enum", "values": ["oa", "interview"], "required": false},
            {"name": "oa_deadline", "type": "date", "required": false, "role": "deadline"}
        ]});
        let out = show(&RecordsCmd::Collections, &json!({"collections": [jobs]}), &Value::Null);
        assert!(
            out.contains("company*, url (url, unique), stage (oa|interview), oa_deadline (date, deadline)"),
            "{out}"
        );
    }

    #[test]
    fn collections_list_skipped_files_in_full() {
        let message = "collection file 'broken.toml': TOML parse error at line 1, column 5, key with no value";
        let data = json!({"collections": [leetcode()], "skipped": [message]});
        let out = show(&RecordsCmd::Collections, &data, &Value::Null);
        assert!(out.starts_with("ID        LABEL"), "{out}");
        assert!(out.ends_with(&format!("\nskipped: {message}")), "{out}");

        let only_broken = json!({"collections": [], "skipped": [message]});
        let out = show(&RecordsCmd::Collections, &only_broken, &Value::Null);
        assert_eq!(out, format!("no collections\nskipped: {message}"));
    }

    #[test]
    fn long_values_are_cut() {
        let long = "x".repeat(60);
        let c = cell(&json!(long));
        assert_eq!(c.chars().count(), crate::render::MAX_CELL);
        assert!(c.ends_with('…'));
    }
}
