//! `shimmer records …`: typed commands over the `records.*` ops (ADR 0008).
//!
//! Nothing here knows about LeetCode or any other collection. Field flags (`--title`,
//! `--company`) and their types come from the daemon at runtime (`records.collections`), so a
//! collection added as a TOML file gets its own flags with no client change (§9).

use std::fmt::Write as _;

use serde_json::{json, Map, Value};
use shimmer_core::{Error, ErrorCode, Result};

use crate::client::Client;
use crate::render::{self, cell, table};
use crate::workspaces::Prompt;

pub const USAGE: &str = "usage: shimmer records <command>

commands:
  collections                          list collections and their fields
  templates                            list ready-made collections to start from
  new ID --from TEMPLATE [--label TEXT] create a collection from a template, e.g.
                                         shimmer records new jobs --from job-applications
  add COLLECTION [ID] [--FIELD VALUE]… add a record, e.g.
                                         shimmer records add leetcode --title \"Two Sum\" --difficulty easy
                                       without an ID, one is made from its title (two-sum)
  list COLLECTION [--FIELD VALUE]…     list records; each --FIELD filters on an exact value
       [--status todo|done] [--limit N] [--offset N]
       [--filter \"FIELD OP VALUE\"]…        OP: = != < <= > >= ~ (contains); a|b|c after = is one-of
       [--has FIELD] [--missing FIELD]     has a value / has none
       [--search TEXT]                     text in the id or any text field
       [--sort FIELD]… [--sort -FIELD]     order, e.g. --sort oa_deadline --sort -applied_on
  get COLLECTION/ID                    show one record
  update COLLECTION/ID [--FIELD VALUE]… [--unset FIELD]…
  complete COLLECTION/ID [--FIELD VALUE]… [--unset FIELD]…
                                       mark done and stamp the collection's date field;
                                       a value for the date field back-dates it
  reopen COLLECTION/ID [--clear-stamp] mark a done record todo again
  rename COLLECTION/ID NEW_ID          give a record a new id
  remove COLLECTION/ID                 remove a record (it goes to the trash)
  restore COLLECTION/ID                bring a removed record back
  trash COLLECTION                     list removed records you can restore

changing a collection:
  check COLLECTION                     list records that don't fit the collection
  rename-field COLLECTION FROM TO      rename a field in the collection and every record
  rename-collection ID NEW_ID          give a collection a new id
  remove-collection ID [--yes]         remove a collection and its records (asks first;
                                       restore-collection brings it back)
  restore-collection ID                bring a removed collection back

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
        refine: Refine,
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
    Restore {
        collection: String,
        id: String,
    },
    Trash {
        collection: String,
    },
    Check {
        collection: String,
    },
    Templates,
    New {
        id: String,
        template: String,
        label: Option<String>,
    },
    RenameField {
        collection: String,
        from: String,
        to: String,
    },
    RenameCollection {
        id: String,
        new_id: String,
    },
    /// `yes`: confirm without asking (`--yes`).
    RemoveCollection {
        id: String,
        yes: bool,
    },
    RestoreCollection {
        id: String,
    },
}

/// What `records list` asks beyond exact `--FIELD VALUE` matches (ADR 0019).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Refine {
    /// `--filter "FIELD OP VALUE"`, as written.
    pub conditions: Vec<String>,
    pub has: Vec<String>,
    pub missing: Vec<String>,
    pub search: Option<String>,
    /// `--sort FIELD` / `--sort -FIELD`, in order.
    pub sort: Vec<String>,
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
    let yes = sub == "remove-collection" && rest.iter().any(|w| w == "--yes");
    if yes {
        rest.retain(|w| w != "--yes");
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
            let (mut filter, mut limit, mut offset, mut refine) = (Vec::new(), None, None, Refine::default());
            for (name, value) in flags {
                match name.as_str() {
                    "limit" => limit = Some(number("--limit", &value)?),
                    "offset" => offset = Some(number("--offset", &value)?),
                    "filter" => refine.conditions.push(value),
                    "has" => refine.has.push(value),
                    "missing" => refine.missing.push(value),
                    "search" => refine.search = Some(value),
                    "sort" => refine.sort.push(value),
                    _ => filter.push((name, value)),
                }
            }
            Ok(RecordsCmd::List { collection, filter, limit, offset, refine })
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
        "templates" => {
            no_flags()?;
            match positional.is_empty() {
                true => Ok(RecordsCmd::Templates),
                false => Err("'records templates' takes no arguments".into()),
            }
        }
        "new" => {
            let usage = "usage: shimmer records new ID --from TEMPLATE [--label TEXT]";
            let [id]: [String; 1] = positional.try_into().map_err(|_| usage)?;
            let (mut template, mut label) = (None, None);
            for (name, value) in flags {
                match name.as_str() {
                    "from" => template = Some(value),
                    "label" => label = Some(value),
                    other => return Err(format!("'records new' takes --from and --label, not '--{other}'")),
                }
            }
            let template = template.ok_or("records new needs --from TEMPLATE; see 'shimmer records templates'")?;
            Ok(RecordsCmd::New { id, template, label })
        }
        "check" | "rename-field" | "rename-collection" | "remove-collection" | "restore-collection" => {
            no_flags()?;
            let usage = match sub.as_str() {
                "check" => "usage: shimmer records check COLLECTION",
                "rename-field" => "usage: shimmer records rename-field COLLECTION FROM TO",
                "rename-collection" => "usage: shimmer records rename-collection ID NEW_ID",
                "remove-collection" => "usage: shimmer records remove-collection ID [--yes]",
                _ => "usage: shimmer records restore-collection ID",
            };
            let p = positional;
            Ok(match (sub.as_str(), p.as_slice()) {
                ("check", [c]) => RecordsCmd::Check { collection: c.clone() },
                ("rename-field", [c, from, to]) => {
                    RecordsCmd::RenameField { collection: c.clone(), from: from.clone(), to: to.clone() }
                }
                ("rename-collection", [id, new_id]) => {
                    RecordsCmd::RenameCollection { id: id.clone(), new_id: new_id.clone() }
                }
                ("remove-collection", [id]) => RecordsCmd::RemoveCollection { id: id.clone(), yes },
                ("restore-collection", [id]) => RecordsCmd::RestoreCollection { id: id.clone() },
                _ => return Err(usage.into()),
            })
        }
        "trash" => {
            no_flags()?;
            let [collection]: [String; 1] =
                positional.try_into().map_err(|_| "usage: shimmer records trash COLLECTION")?;
            Ok(RecordsCmd::Trash { collection })
        }
        "get" | "remove" | "reopen" | "restore" => {
            no_flags()?;
            let (collection, id) = target(&sub, positional)?;
            Ok(match sub.as_str() {
                "get" => RecordsCmd::Get { collection, id },
                "reopen" => RecordsCmd::Reopen { collection, id, clear_stamp },
                "restore" => RecordsCmd::Restore { collection, id },
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
            Self::Help | Self::Collections | Self::Templates | Self::New { .. } => None,
            Self::Add { collection, .. }
            | Self::Rename { collection, .. }
            | Self::List { collection, .. }
            | Self::Get { collection, .. }
            | Self::Update { collection, .. }
            | Self::Complete { collection, .. }
            | Self::Reopen { collection, .. }
            | Self::Remove { collection, .. }
            | Self::Restore { collection, .. }
            | Self::Trash { collection }
            | Self::Check { collection }
            | Self::RenameField { collection, .. } => Some(collection),
            Self::RenameCollection { .. } | Self::RemoveCollection { .. } | Self::RestoreCollection { .. } => None,
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
        RecordsCmd::List { collection, filter, limit, offset, refine } => {
            let mut params =
                json!({"collection": collection, "filter": query_filter(schema, &fields(filter)?, refine)?});
            if let Some(text) = &refine.search {
                params["search"] = json!(text);
            }
            if !refine.sort.is_empty() {
                let keys: Vec<String> = refine
                    .sort
                    .iter()
                    .map(|k| match k.strip_prefix('-') {
                        Some(name) => format!("-{}", field_name(schema, name)),
                        None => field_name(schema, k),
                    })
                    .collect();
                params["sort"] = json!(keys);
            }
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
        RecordsCmd::Restore { collection, id } => ("records.restore", json!({"collection": collection, "id": id})),
        RecordsCmd::Trash { collection } => ("records.trash", json!({"collection": collection})),
        RecordsCmd::Check { collection } => ("records.check", json!({"collection": collection})),
        RecordsCmd::Templates => ("records.templates", json!({})),
        RecordsCmd::New { id, template, label } => {
            let mut params = json!({"id": id, "template": template});
            if let Some(label) = label {
                params["label"] = json!(label);
            }
            ("records.create_collection", params)
        }
        RecordsCmd::RenameField { collection, from, to } => (
            "records.rename_field",
            json!({"collection": collection, "from": field_name(schema, from), "to": to.replace('-', "_")}),
        ),
        RecordsCmd::RenameCollection { id, new_id } => {
            ("records.rename_collection", json!({"id": id, "new_id": new_id}))
        }
        RecordsCmd::RemoveCollection { id, .. } => ("records.remove_collection", json!({"id": id})),
        RecordsCmd::RestoreCollection { id } => ("records.restore_collection", json!({"id": id})),
    })
}

/// The `filter` param for `records list` (ADR 0019): exact `--FIELD VALUE` matches, then each
/// `--filter`, `--has` and `--missing` as operators. Two conditions on one field combine into one
/// operator object (`>=` and `<=` make a range).
fn query_filter(schema: &Value, exact: &Map<String, Value>, refine: &Refine) -> Result<Map<String, Value>> {
    let mut out = exact.clone();
    let mut add = |name: String, op: &str, value: Value| {
        let entry = out.entry(name).or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({"eq": entry.take()});
        }
        entry[op] = value;
    };
    for raw in &refine.conditions {
        let (name, op, value) = condition(schema, raw)?;
        add(name, op, value);
    }
    for name in &refine.has {
        add(field_name(schema, name), "set", json!(true));
    }
    for name in &refine.missing {
        add(field_name(schema, name), "set", json!(false));
    }
    Ok(out)
}

/// `oa_deadline<=2026-10-10` → (`oa_deadline`, `lte`, `"2026-10-10"`). `stage=oa|interview` is
/// `in`; `company~goog` is `contains`. The value is typed from the field, like `--FIELD VALUE`.
fn condition(schema: &Value, raw: &str) -> Result<(String, &'static str, Value)> {
    const OPS: [(&str, &str); 7] =
        [("<=", "lte"), (">=", "gte"), ("!=", "ne"), ("<", "lt"), (">", "gt"), ("=", "eq"), ("~", "contains")];
    let bad = || {
        Error::invalid_params(format!(
            "--filter '{raw}': write FIELD OP VALUE with OP one of = != < <= > >= ~, e.g. --filter \"oa_deadline<=2026-10-10\""
        ))
    };
    let at = raw.find(['<', '>', '!', '=', '~']).ok_or_else(bad)?;
    let (flag, rest) = (raw[..at].trim(), &raw[at..]);
    let (symbol, op) = OPS.iter().find(|(symbol, _)| rest.starts_with(symbol)).ok_or_else(bad)?;
    let value = rest[symbol.len()..].trim();
    if flag.is_empty() || value.is_empty() {
        return Err(bad());
    }
    if *op == "eq" && value.contains('|') {
        let items =
            value.split('|').map(|v| typed(schema, flag, v.trim()).map(|(_, v)| v)).collect::<Result<Vec<_>>>()?;
        return Ok((field_name(schema, flag), "in", json!(items)));
    }
    // `contains` always takes text, whatever the field's type.
    let (name, value) = match *op {
        "contains" => (field_name(schema, flag), json!(value)),
        _ => typed(schema, flag, value)?,
    };
    Ok((name, op, value))
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
pub async fn run(client: &mut Client, cmd: &RecordsCmd, json: bool, prompt: &mut dyn Prompt) -> Result<String> {
    let schema = match cmd.collection() {
        Some(id) => schema(client, id).await?,
        None => Value::Null,
    };
    let (op, mut params) = request(cmd, &schema)?;
    let data = match client.call(op, params.clone()).await {
        // ADR 0021 §5: the daemon says how many records go; ask, then send that count back.
        Err(e) if e.code == ErrorCode::ConfirmationRequired => {
            let RecordsCmd::RemoveCollection { id, yes } = cmd else { return Err(e) };
            let detail = e.detail.clone().unwrap_or_default();
            if !confirm_removal(prompt, *yes, id, &detail)? {
                return Ok(format!("kept {id}"));
            }
            params["confirm"] = detail;
            client.call(op, params).await?
        }
        other => other?,
    };
    Ok(if json { render::json(&data) } else { show(cmd, &data, &schema) })
}

/// Whether to go ahead with removing a collection, given the daemon's
/// `{"records": N}`. Never waits for an answer nobody can see (CLAUDE.md §15.1).
fn confirm_removal(prompt: &mut dyn Prompt, yes: bool, id: &str, detail: &Value) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    let records = detail["records"].as_u64().unwrap_or(0);
    if !prompt.interactive() {
        return Err(Error::invalid_params(format!(
            "refusing to remove '{id}' and its {records} record(s) without confirmation; pass --yes to do it anyway"
        )));
    }
    eprintln!(
        "This removes '{id}' and its {records} record(s). They go to the trash:\n  shimmer records restore-collection {id} brings them back."
    );
    Ok(prompt.confirm("Remove it?"))
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
        RecordsCmd::Remove { collection, id } => {
            format!("removed {collection}/{id} (undo: shimmer records restore {collection}/{id})")
        }
        RecordsCmd::Restore { collection, id } => format!("✓ {collection}/{id} restored"),
        RecordsCmd::Check { collection } => check_report(collection, data),
        RecordsCmd::Templates => templates(data),
        RecordsCmd::New { id, template, .. } => created(id, template, data),
        RecordsCmd::RenameField { collection, from, to } => format!(
            "✓ {collection}: field '{}' is now '{}' ({} record(s) updated)",
            field_name(schema, from),
            to.replace('-', "_"),
            cell(&data["updated"])
        ),
        RecordsCmd::RenameCollection { id, new_id } => format!("✓ {id} is now {new_id}"),
        RecordsCmd::RemoveCollection { id, .. } => format!(
            "removed {id} ({} record(s); undo: shimmer records restore-collection {id})",
            cell(&data["records"])
        ),
        RecordsCmd::RestoreCollection { id } => format!("✓ {id} restored"),
        RecordsCmd::Trash { collection } => match data["items"].as_array().is_none_or(Vec::is_empty) {
            true => format!("nothing in {collection}'s trash"),
            false => list(data, schema),
        },
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

fn templates(data: &Value) -> String {
    let rows: Vec<Vec<String>> = data["templates"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| vec![cell(&t["id"]), cell(&t["label"]), t["description"].as_str().unwrap_or_default().to_owned()])
        .collect();
    match rows.is_empty() {
        true => "no templates".into(),
        false => table(&["ID".into(), "LABEL".into(), "DESCRIPTION".into()], &rows),
    }
}

/// What was made, how to add the first record (its required fields), and what goes with it.
/// Everything comes from the new collection the daemon returned (§2).
fn created(id: &str, template: &str, collection: &Value) -> String {
    let mut out = format!("✓ created collection {id} from {template}");
    let required: Vec<String> = collection["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["required"] == true)
        .map(|f| format!("--{} …", f["name"].as_str().unwrap_or_default().replace('_', "-")))
        .collect();
    let _ = write!(out, "\n  add one: shimmer records add {id}");
    if !required.is_empty() {
        let _ = write!(out, " {}", required.join(" "));
    }
    for related in collection["related"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let _ = write!(out, "\n  goes with: {related} (shimmer records new {related} --from {related})");
    }
    out
}

/// `checked 42 records; 2 with problems`, then each record's problems, one per line.
fn check_report(collection: &str, data: &Value) -> String {
    let checked = data["checked"].as_u64().unwrap_or(0);
    let problems = data["problems"].as_array().cloned().unwrap_or_default();
    if problems.is_empty() {
        return format!("checked {checked} record(s) in {collection}; all fit");
    }
    let mut out = format!("checked {checked} record(s) in {collection}; {} with problems", problems.len());
    let width = problems.iter().map(|p| cell(&p["id"]).chars().count()).max().unwrap_or(0);
    for p in &problems {
        let id = cell(&p["id"]);
        for (i, problem) in p["problems"].as_array().into_iter().flatten().enumerate() {
            let shown = if i == 0 { id.as_str() } else { "" };
            let _ = write!(out, "\n  {shown:width$}  {}", problem.as_str().unwrap_or_default());
        }
    }
    out
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

/// Reference collections (`completable = false`, ADR 0021) have no status to show.
fn has_status(schema: &Value) -> bool {
    schema["completable"] != false
}

fn item(collection: &str, id: &str, data: &Value, schema: &Value) -> String {
    let mut out = match has_status(schema) {
        true => format!("{collection}/{id}  ({})", cell(&data["status"])),
        false => format!("{collection}/{id}"),
    };
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
        // Only columns some listed record has filled in: a 20-field collection with three fields
        // set shouldn't print 17 columns of dashes. `records get` still shows every field.
        let keys: Vec<String> = columns(&items[0], schema)
            .into_iter()
            .filter(|k| items.iter().any(|i| !i[k.as_str()].is_null() && i[k.as_str()] != ""))
            .collect();
        let status = has_status(schema);
        let mut header = vec!["ID".to_owned()];
        if status {
            header.push("STATUS".to_owned());
        }
        header.extend(keys.iter().map(|k| k.to_uppercase().replace('_', " ")));
        let rows: Vec<Vec<String>> = items
            .iter()
            .map(|i| {
                let mut row = vec![cell(&i["id"])];
                if status {
                    row.push(cell(&i["status"]));
                }
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
                offset: None,
                refine: Refine::default()
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
    fn list_builds_operator_filters_search_and_sort() {
        let cmd = parse_words(&[
            "list",
            "leetcode",
            "--filter",
            "last-solved>=2026-10-01",
            "--filter",
            "last_solved <= 2026-10-10",
            "--filter",
            "difficulty=easy|medium",
            "--filter",
            "title~two",
            "--filter",
            "attempts!=3",
            "--has",
            "url",
            "--missing",
            "starred",
            "--search",
            "sum",
            "--sort",
            "-last-solved",
            "--sort",
            "title",
        ])
        .unwrap();
        let (op, params) = request(&cmd, &leetcode()).unwrap();
        assert_eq!(op, "records.list");
        assert_eq!(
            params,
            json!({"collection": "leetcode",
                "filter": {
                    "last_solved": {"gte": "2026-10-01", "lte": "2026-10-10"},
                    "difficulty": {"in": ["easy", "medium"]},
                    "title": {"contains": "two"},
                    "attempts": {"ne": 3},
                    "url": {"set": true},
                    "starred": {"set": false}},
                "search": "sum",
                "sort": ["-last_solved", "title"]})
        );

        // An exact --FIELD and an operator on the same field combine.
        let both = parse_words(&["list", "leetcode", "--difficulty", "easy", "--has", "difficulty"]).unwrap();
        assert_eq!(
            request(&both, &leetcode()).unwrap().1["filter"],
            json!({"difficulty": {"eq": "easy", "set": true}})
        );

        for bad in ["title", "=x", "title=", "title!"] {
            let cmd = parse_words(&["list", "leetcode", "--filter", bad]).unwrap();
            let e = request(&cmd, &leetcode()).unwrap_err();
            assert!(e.message.contains("FIELD OP VALUE") || e.message.contains("expects"), "{bad}: {}", e.message);
        }
    }

    #[test]
    fn remove_says_how_to_undo_and_restore_and_trash_parse() {
        let remove = parse_words(&["remove", "jobs/acme"]).unwrap();
        assert_eq!(
            show(&remove, &json!({"removed": true}), &Value::Null),
            "removed jobs/acme (undo: shimmer records restore jobs/acme)"
        );

        let restore = parse_words(&["restore", "jobs", "acme"]).unwrap();
        assert_eq!(
            request(&restore, &Value::Null).unwrap(),
            ("records.restore", json!({"collection": "jobs", "id": "acme"}))
        );
        assert_eq!(show(&restore, &json!({}), &Value::Null), "✓ jobs/acme restored");

        let trash = parse_words(&["trash", "jobs"]).unwrap();
        assert_eq!(request(&trash, &Value::Null).unwrap(), ("records.trash", json!({"collection": "jobs"})));
        assert_eq!(show(&trash, &json!({"items": []}), &Value::Null), "nothing in jobs's trash");
        assert!(parse_words(&["trash"]).unwrap_err().contains("records trash COLLECTION"));
    }

    #[test]
    fn collection_commands_parse_and_report() {
        assert_eq!(parse_words(&["check", "jobs"]).unwrap(), RecordsCmd::Check { collection: "jobs".into() });
        let rename = parse_words(&["rename-field", "leetcode", "last-solved", "solved-on"]).unwrap();
        assert_eq!(
            request(&rename, &leetcode()).unwrap(),
            ("records.rename_field", json!({"collection": "leetcode", "from": "last_solved", "to": "solved_on"}))
        );
        assert_eq!(
            parse_words(&["remove-collection", "jobs", "--yes"]).unwrap(),
            RecordsCmd::RemoveCollection { id: "jobs".into(), yes: true }
        );
        assert_eq!(
            request(&parse_words(&["rename-collection", "jobs", "apps"]).unwrap(), &Value::Null).unwrap(),
            ("records.rename_collection", json!({"id": "jobs", "new_id": "apps"}))
        );
        assert!(parse_words(&["rename-field", "jobs", "a"]).unwrap_err().contains("FROM TO"));

        let report = json!({"checked": 3, "problems": [
            {"id": "acme", "problems": ["field 'stage' must be one of oa", "unknown key 'x' (not a field of 'jobs')"]}]});
        let out = show(&RecordsCmd::Check { collection: "jobs".into() }, &report, &Value::Null);
        assert_eq!(
            out,
            "checked 3 record(s) in jobs; 1 with problems\n  acme  field 'stage' must be one of oa\n        unknown key 'x' (not a field of 'jobs')"
        );
        let clean = show(
            &RecordsCmd::Check { collection: "jobs".into() },
            &json!({"checked": 2, "problems": []}),
            &Value::Null,
        );
        assert_eq!(clean, "checked 2 record(s) in jobs; all fit");
    }

    struct Scripted {
        interactive: bool,
        answer: bool,
    }

    impl Prompt for Scripted {
        fn interactive(&self) -> bool {
            self.interactive
        }
        fn confirm(&mut self, _: &str) -> bool {
            self.answer
        }
    }

    #[test]
    fn removing_a_collection_asks_at_a_terminal_and_refuses_without_one() {
        let detail = json!({"records": 42});
        let mut yes = Scripted { interactive: true, answer: true };
        let mut no = Scripted { interactive: true, answer: false };
        let mut script = Scripted { interactive: false, answer: true };
        assert!(confirm_removal(&mut yes, false, "jobs", &detail).unwrap());
        assert!(!confirm_removal(&mut no, false, "jobs", &detail).unwrap());
        let e = confirm_removal(&mut script, false, "jobs", &detail).unwrap_err();
        assert!(e.message.contains("its 42 record(s)") && e.message.contains("--yes"), "{}", e.message);
        assert!(confirm_removal(&mut script, true, "jobs", &detail).unwrap(), "--yes needs no terminal");
    }

    #[test]
    fn reference_collections_show_no_status() {
        let education =
            json!({"id": "education", "completable": false, "fields": [{"name": "school", "type": "string"}]});
        let data = json!({"total": 1, "items": [{"id": "uni", "school": "Uni"}]});
        let cmd = parse_words(&["list", "education"]).unwrap();
        assert_eq!(show(&cmd, &data, &education), "ID   SCHOOL\nuni  Uni\n1 record");
        let get = RecordsCmd::Get { collection: "education".into(), id: "uni".into() };
        assert!(show(&get, &data["items"][0], &education).starts_with("education/uni\n"));
    }

    #[test]
    fn templates_and_new() {
        assert_eq!(parse_words(&["templates"]).unwrap(), RecordsCmd::Templates);
        let new = parse_words(&["new", "jobs", "--from", "job-applications", "--label", "Internships"]).unwrap();
        assert_eq!(
            request(&new, &Value::Null).unwrap(),
            (
                "records.create_collection",
                json!({"id": "jobs", "template": "job-applications", "label": "Internships"})
            )
        );
        assert!(parse_words(&["new", "jobs"]).unwrap_err().contains("--from TEMPLATE"));
        assert!(parse_words(&["new"]).unwrap_err().contains("usage"));
        assert!(parse_words(&["new", "jobs", "--from", "x", "--colour", "red"])
            .unwrap_err()
            .contains("--from and --label"));

        let collection = json!({"id": "jobs", "related": ["interviews"], "fields": [
            {"name": "position", "type": "string", "required": true},
            {"name": "company", "type": "string", "required": true},
            {"name": "url", "type": "string", "required": false}]});
        assert_eq!(
            show(&new, &collection, &Value::Null),
            "✓ created collection jobs from job-applications\n  add one: shimmer records add jobs --position … --company …\n  goes with: interviews (shimmer records new interviews --from interviews)"
        );
        let listing = json!({"templates": [{"id": "leetcode", "label": "LeetCode", "description": "Problems"}]});
        assert_eq!(
            show(&RecordsCmd::Templates, &listing, &Value::Null),
            "ID        LABEL     DESCRIPTION\nleetcode  LeetCode  Problems"
        );
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
ID         STATUS  TITLE      DIFFICULTY  LAST SOLVED  ATTEMPTS  STARRED
two-sum    done    Two Sum    easy        2026-09-18   -         -
lru-cache  todo    LRU Cache  medium      -            2         true
2 of 3";
        // URL is empty in every listed record, so its column is left out.
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
