//! `shimmer records …`: typed commands over the `records.*` ops (ADR 0008).
//!
//! Nothing here knows about LeetCode or any other collection. Field flags (`--title`,
//! `--company`) and their types come from the daemon at runtime (`records.collections`), so a
//! collection added as a TOML file gets its own flags with no client change (§9).

use std::fmt::Write as _;

use serde_json::{json, Map, Value};
use shimmer_core::{Error, Result};

use crate::client::Client;
use crate::render;

pub const USAGE: &str = "usage: shimmer records <command>

commands:
  collections                          list collections and their fields
  add COLLECTION ID [--FIELD VALUE]…   add a record, e.g.
                                         shimmer records add leetcode two-sum --title \"Two Sum\" --difficulty easy
  list COLLECTION [--FIELD VALUE]…     list records; each --FIELD filters on an exact value
       [--status todo|done] [--limit N] [--offset N]
  get COLLECTION/ID                    show one record
  update COLLECTION/ID [--FIELD VALUE]… [--unset FIELD]…
  complete COLLECTION/ID               mark done and stamp the collection's date field
  remove COLLECTION/ID                 delete a record

A record can be written COLLECTION/ID or COLLECTION ID. Field names come from the
collection: see 'shimmer records collections'. Add --json to any command for raw output.";

/// `--name value` pairs, in the order given.
pub type Flags = Vec<(String, String)>;

#[derive(Clone, Debug, PartialEq)]
pub enum RecordsCmd {
    Help,
    Collections,
    Add { collection: String, id: String, set: Flags },
    List { collection: String, filter: Flags, limit: Option<usize>, offset: Option<usize> },
    Get { collection: String, id: String },
    Update { collection: String, id: String, set: Flags, unset: Vec<String> },
    Complete { collection: String, id: String },
    Remove { collection: String, id: String },
}

// ---------------------------------------------------------------- parsing

/// `words` are what follows `shimmer records`, with global flags already removed.
pub fn parse(words: Vec<String>) -> std::result::Result<RecordsCmd, String> {
    let mut words = words.into_iter();
    let Some(sub) = words.next() else { return Ok(RecordsCmd::Help) };
    let (positional, flags) = split(words.collect())?;
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
            let (collection, id) = target(&sub, positional)?;
            Ok(RecordsCmd::Add { collection, id, set: flags })
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
        "update" => {
            let (collection, id) = target(&sub, positional)?;
            let (unset, set) = flags.into_iter().partition::<Vec<_>, _>(|(name, _)| name == "unset");
            let unset: Vec<String> = unset.into_iter().map(|(_, field)| field).collect();
            if set.is_empty() && unset.is_empty() {
                return Err("nothing to update: give --FIELD VALUE or --unset FIELD".into());
            }
            Ok(RecordsCmd::Update { collection, id, set, unset })
        }
        "get" | "complete" | "remove" => {
            no_flags()?;
            let (collection, id) = target(&sub, positional)?;
            Ok(match sub.as_str() {
                "get" => RecordsCmd::Get { collection, id },
                "complete" => RecordsCmd::Complete { collection, id },
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
            | Self::List { collection, .. }
            | Self::Get { collection, .. }
            | Self::Update { collection, .. }
            | Self::Complete { collection, .. }
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
    Ok(match cmd {
        RecordsCmd::Help => return Err(Error::internal("help is not a request")),
        RecordsCmd::Collections => ("records.collections", json!({})),
        RecordsCmd::Add { collection, id, set } => {
            ("records.add", json!({"collection": collection, "id": id, "fields": fields(set)?}))
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
            let mut changes = fields(set)?;
            for name in unset {
                changes.insert(field_name(schema, name), Value::Null);
            }
            ("records.update", json!({"collection": collection, "id": id, "fields": changes}))
        }
        RecordsCmd::Complete { collection, id } => ("records.complete", json!({"collection": collection, "id": id})),
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
        RecordsCmd::Add { collection, id, .. } => format!("added {collection}/{id}"),
        RecordsCmd::Update { collection, id, .. } => format!("updated {collection}/{id}"),
        RecordsCmd::Remove { collection, id } => format!("removed {collection}/{id}"),
        RecordsCmd::Complete { collection, id } => {
            let mut out = format!("✓ {collection}/{id} done");
            if let Some(stamp) = schema["stamp_on_complete"].as_str() {
                let _ = write!(out, " ({stamp} {})", cell(&data[stamp]));
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

/// `difficulty (easy|medium|hard)`, `title*` for required, `last_solved (date)`.
fn describe_field(f: &Value) -> String {
    let mut s = cell(&f["name"]);
    if f["required"] == true {
        s.push('*');
    }
    match f["type"].as_str() {
        Some("enum") => {
            let values: Vec<String> = f["values"].as_array().into_iter().flatten().map(cell).collect();
            let _ = write!(s, " ({})", values.join("|"));
        }
        Some("string") | None => {}
        Some(other) => {
            let _ = write!(s, " ({other})");
        }
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

const MAX_CELL: usize = 40;

/// One value as table text: strings bare, `null` as `-`, long values cut with `…`.
fn cell(v: &Value) -> String {
    let s = match v {
        Value::Null => "-".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > MAX_CELL {
        s.chars().take(MAX_CELL - 1).chain(['…']).collect()
    } else {
        s
    }
}

fn table(header: &[String], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, c) in widths.iter_mut().zip(row) {
            *w = (*w).max(c.chars().count());
        }
    }
    let line = |cells: &[String]| {
        let mut s = String::new();
        for (i, (c, w)) in cells.iter().zip(&widths).enumerate() {
            if i + 1 == cells.len() {
                s.push_str(c);
            } else {
                let pad = w - c.chars().count();
                let _ = write!(s, "{c}{}  ", " ".repeat(pad));
            }
        }
        s
    };
    let mut out = line(header);
    for row in rows {
        out.push('\n');
        out.push_str(&line(row));
    }
    out
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
        let want = RecordsCmd::Complete { collection: "leetcode".into(), id: "two-sum".into() };
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
                id: "two-sum".into(),
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
            RecordsCmd::Add { collection: "leetcode".into(), id: "x".into(), set: pairs(&[("attempts", "-1")]) }
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
        let complete = RecordsCmd::Complete { collection: "leetcode".into(), id: "two-sum".into() };
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
        assert_eq!(c.chars().count(), MAX_CELL);
        assert!(c.ends_with('…'));
    }
}
