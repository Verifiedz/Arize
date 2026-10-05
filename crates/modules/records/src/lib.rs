//! `shimmer-records`: typed collections of records (CLAUDE.md §8). A LeetCode tracker, a job tracker
//! or any user-defined tracker is a collection TOML; this module is the one implementation of
//! storage, validation and filtering behind all of them. Layout and ops: ADR 0008.
//!
//! Depends on `core` only. Everything durable goes through `ctx.store`, one transaction per
//! change, with its event committed alongside (§7.1).

mod item;
mod lifecycle;
mod schema;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use shimmer_core::ids::is_valid_name;
use shimmer_core::params::decode;
use shimmer_core::{CommandSpec, Ctx, Error, ErrorCode, Execution, Manifest, Module, Result, WriteLock};

use crate::item::Item;
use crate::schema::Collection;

const LEETCODE: &str = include_str!("../collections/leetcode.toml");
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;

#[derive(Default)]
pub struct Records {
    /// Inline requests run concurrently. Held across each read-check-write so two `records.add`
    /// calls for one id cannot both pass the existence check. Never held across an `.await`.
    write: WriteLock,
}

#[async_trait]
impl Module for Records {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "records".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            namespace: "records".into(),
            topics: [
                "records.item.created",
                "records.item.updated",
                "records.item.completed",
                "records.item.reopened",
                "records.item.removed",
                "records.collection.created",
            ]
            .map(String::from)
            .to_vec(),
            capabilities: vec![],
        }
    }

    /// Seed the built-in LeetCode collection when there are no collections at all, so a fresh
    /// install has something to track.
    async fn init(&self, ctx: &Ctx) -> Result<()> {
        let _g = self.write.lock();
        if !ctx.store.list("collections")?.is_empty() {
            return Ok(());
        }
        ctx.store.transaction(|tx| {
            tx.put("collections/leetcode.toml", LEETCODE)?;
            tx.emit("records.collection.created", json!({"collection": "leetcode"}))
        })
    }

    fn commands(&self) -> Vec<CommandSpec> {
        let target = json!({"collection": {"type": "string"}, "id": {"type": "string"}});
        let with = |extra: Value| {
            let mut props = target.as_object().cloned().unwrap_or_default();
            props.extend(extra.as_object().cloned().unwrap_or_default());
            json!({"type": "object", "properties": props, "required": ["collection", "id"]})
        };
        let fields = json!({"fields": {"type": "object"}});
        [
            ("records.collections", "List collection definitions", json!({"type": "object"})),
            ("records.add", "Add a record to a collection", with(fields.clone())),
            ("records.get", "Get one record", with(json!({}))),
            (
                "records.list",
                "List records in a collection",
                json!({"type": "object", "required": ["collection"], "properties": {
                    "collection": {"type": "string"}, "filter": {"type": "object"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT},
                    "offset": {"type": "integer", "minimum": 0}}}),
            ),
            ("records.update", "Set or unset (null) fields of a record", with(fields.clone())),
            (
                "records.complete",
                "Mark a record done and stamp its completion date, optionally setting fields",
                with(fields),
            ),
            (
                "records.reopen",
                "Mark a done record todo again (the stamp is kept unless clear_stamp)",
                with(json!({"clear_stamp": {"type": "boolean"}})),
            ),
            ("records.remove", "Delete a record", with(json!({}))),
        ]
        .into_iter()
        .map(|(op, summary, params_schema)| CommandSpec {
            op: op.into(),
            summary: summary.into(),
            params_schema,
            execution: Execution::Inline,
        })
        .collect()
    }

    async fn handle(&self, op: &str, params: Value, ctx: &Ctx) -> Result<Value> {
        match op {
            "records.collections" => self.collections(ctx),
            "records.add" => self.add(ctx, decode(params)?),
            "records.get" => {
                let t: Target = decode(params)?;
                let c = load_collection(ctx, &t.collection)?;
                Ok(load_item(ctx, &c, &t.id)?.to_wire(&c))
            }
            "records.list" => self.list(ctx, decode(params)?),
            "records.update" => self.update(ctx, decode(params)?),
            "records.complete" => self.complete(ctx, decode(params)?),
            "records.reopen" => self.reopen(ctx, decode(params)?),
            "records.remove" => self.remove(ctx, decode(params)?),
            _ => Err(Error::unknown_op(format!("records has no op '{op}'"))),
        }
    }
}

#[derive(Deserialize)]
struct Target {
    collection: String,
    id: String,
}

#[derive(Deserialize)]
struct WithFields {
    collection: String,
    id: String,
    #[serde(default)]
    fields: Map<String, Value>,
}

#[derive(Deserialize)]
struct Reopen {
    collection: String,
    id: String,
    #[serde(default)]
    clear_stamp: bool,
}

#[derive(Deserialize)]
struct ListParams {
    collection: String,
    #[serde(default)]
    filter: Map<String, Value>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

impl Records {
    /// Every readable collection, plus `skipped` naming each malformed file, the same shape as
    /// `records.list`. One broken file must not hide the others (ADR 0008): a request that
    /// uses it still fails, naming the file.
    fn collections(&self, ctx: &Ctx) -> Result<Value> {
        let (mut out, mut skipped) = (Vec::new(), Vec::new());
        for path in ctx.store.list("collections")? {
            let Some(id) = path.strip_prefix("collections/").and_then(|p| p.strip_suffix(".toml")) else { continue };
            if !is_valid_name(id) {
                skipped.push(format!("collection file '{id}.toml': the file name must match [a-z][a-z0-9_-]*"));
                continue;
            }
            match load_collection(ctx, id) {
                Ok(c) => out.push(serde_json::to_value(c).unwrap_or_default()),
                Err(e) if e.code == ErrorCode::InvalidParams => skipped.push(e.message),
                Err(e) => return Err(e),
            }
        }
        let mut data = json!({"collections": out});
        if !skipped.is_empty() {
            data["skipped"] = json!(skipped);
        }
        Ok(data)
    }

    fn add(&self, ctx: &Ctx, p: WithFields) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        item::check_id(&p.id)?;
        if let Some((k, _)) = p.fields.iter().find(|(_, v)| v.is_null()) {
            return Err(Error::invalid_params(format!("field '{k}' is null; leave it out to leave it unset")));
        }
        c.check_values(&p.fields)?;
        c.check_required(&p.fields)?;
        let item = Item::new(&p.id, p.fields);
        let wire = item.to_wire(&c);

        // Under the write lock, so nothing can take the id or a unique value in between.
        let _g = self.write.lock();
        let path = item_path(&c.id, &p.id);
        if ctx.store.read_string(&path)?.is_some() {
            return Err(Error::conflict(format!("'{}' already has a record '{}'", c.id, p.id)));
        }
        check_unique(ctx, &c, &item.id, &item.fields_map())?;
        ctx.store.transaction(|tx| {
            tx.put(&path, item.to_toml())?;
            tx.emit("records.item.created", payload(&c, &item, &wire))
        })?;
        Ok(wire)
    }

    fn list(&self, ctx: &Ctx, p: ListParams) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        c.check_filter(&p.filter)?;
        let limit = p.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(Error::invalid_params(format!("limit must be between 1 and {MAX_LIMIT}")));
        }

        let (mut matching, mut skipped) = (Vec::new(), Vec::new());
        let dir = format!("items/{}", c.id);
        // Sorted by path, so by id.
        for path in ctx.store.list(&dir)? {
            let Some(id) = path.strip_prefix(&format!("{dir}/")).and_then(|p| p.strip_suffix(".toml")) else {
                continue;
            };
            // One bad hand-edit must not hide every other record.
            match ctx.store.read_string(&path)?.map(|text| Item::from_toml(id, &text)) {
                Some(Ok(item)) => {
                    let wire = item.to_wire(&c);
                    if item::matches(&wire, &p.filter) {
                        matching.push(wire);
                    }
                }
                Some(Err(e)) => skipped.push(e.message),
                None => {}
            }
        }
        let total = matching.len();
        let items: Vec<Value> = matching.into_iter().skip(p.offset).take(limit).collect();
        let mut out = json!({"items": items, "total": total});
        if !skipped.is_empty() {
            out["skipped"] = json!(skipped);
        }
        Ok(out)
    }

    fn update(&self, ctx: &Ctx, p: WithFields) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        c.check_changes(&p.fields)?;

        let _g = self.write.lock();
        let before = load_item(ctx, &c, &p.id)?;
        check_unique(ctx, &c, &p.id, &p.fields)?;
        let mut item = before.clone();
        item.apply(p.fields);
        c.check_required(&item.fields_map())?;
        let wire = item.to_wire(&c);
        let mut event = payload(&c, &item, &wire);
        // Only fields whose value actually changed (ADR 0017 §8).
        event["changed"] = json!(item::changed(&before.fields, &item.fields));
        ctx.store.transaction(|tx| {
            tx.put(&item_path(&c.id, &p.id), item.to_toml())?;
            tx.emit("records.item.updated", event)
        })?;
        Ok(wire)
    }

    /// Done, stamped, and any `fields` set, in one transaction with one event (ADR 0016 §2).
    fn complete(&self, ctx: &Ctx, p: WithFields) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        // Local calendar date, not the UTC date (ADR 0009): a late-evening completion west
        // of UTC must not stamp tomorrow.
        let today = shimmer_core::local_date(ctx.clock.now(), &ctx.local_tz).format("%Y-%m-%d").to_string();

        let _g = self.write.lock();
        let current = load_item(ctx, &c, &p.id)?;
        check_unique(ctx, &c, &p.id, &p.fields)?;
        let item = lifecycle::complete(&c, &current, p.fields, &today)?;
        self.save(ctx, &c, &item, "records.item.completed")
    }

    /// `done → todo` (ADR 0016 §1).
    fn reopen(&self, ctx: &Ctx, p: Reopen) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        let _g = self.write.lock();
        let item = lifecycle::reopen(&c, &load_item(ctx, &c, &p.id)?, p.clear_stamp)?;
        self.save(ctx, &c, &item, "records.item.reopened")
    }

    /// Write `item` and emit `topic` with its payload, in one transaction (§7.1).
    fn save(&self, ctx: &Ctx, c: &Collection, item: &Item, topic: &str) -> Result<Value> {
        let wire = item.to_wire(c);
        ctx.store.transaction(|tx| {
            tx.put(&item_path(&c.id, &item.id), item.to_toml())?;
            tx.emit(topic, payload(c, item, &wire))
        })?;
        Ok(wire)
    }

    fn remove(&self, ctx: &Ctx, t: Target) -> Result<Value> {
        let c = load_collection(ctx, &t.collection)?;
        let _g = self.write.lock();
        load_item(ctx, &c, &t.id)?;
        ctx.store.transaction(|tx| {
            tx.delete(&item_path(&c.id, &t.id))?;
            tx.emit("records.item.removed", json!({"collection": c.id, "id": t.id}))
        })?;
        Ok(json!({"removed": true}))
    }
}

/// An item event's payload: what changed, what to call it, and what is due (ADR 0017 §7), so a
/// listener needs nothing but the event.
fn payload(c: &Collection, item: &Item, wire: &Value) -> Value {
    json!({
        "collection": c.id,
        "id": item.id,
        "title": c.title_of(&item.id, &item.fields),
        "deadlines": c.deadlines_of(&item.fields),
        "item": wire,
    })
}

/// No other record in the collection already has a value a unique field in `values` would take
/// (ADR 0017 §4). Reads every record, like `records.list`; called under the write lock so two
/// requests can't both take a value. Unreadable files are skipped: they never block a write.
fn check_unique(ctx: &Ctx, c: &Collection, id: &str, values: &Map<String, Value>) -> Result<()> {
    let wanted = c.unique_values(values);
    if wanted.is_empty() {
        return Ok(());
    }
    let dir = format!("items/{}", c.id);
    for path in ctx.store.list(&dir)? {
        let Some(other) = path.strip_prefix(&format!("{dir}/")).and_then(|p| p.strip_suffix(".toml")) else {
            continue;
        };
        if other == id {
            continue;
        }
        let Some(Ok(record)) = ctx.store.read_string(&path)?.map(|text| Item::from_toml(other, &text)) else {
            continue;
        };
        for (field, value) in &wanted {
            if record.fields.get(*field) == Some(*value) {
                let shown = value.as_str().map_or_else(|| value.to_string(), str::to_owned);
                return Err(Error::conflict(format!("{field} '{shown}' is already used by '{other}' in '{}'", c.id)));
            }
        }
    }
    Ok(())
}

fn load_collection(ctx: &Ctx, id: &str) -> Result<Collection> {
    let missing = || Error::not_found(format!("no collection '{id}'"));
    if !is_valid_name(id) {
        return Err(missing());
    }
    let text = ctx.store.read_string(&format!("collections/{id}.toml"))?.ok_or_else(missing)?;
    Collection::parse(id, &text)
}

fn load_item(ctx: &Ctx, c: &Collection, id: &str) -> Result<Item> {
    item::check_id(id)?;
    match ctx.store.read_string(&item_path(&c.id, id))? {
        Some(text) => Item::from_toml(id, &text),
        None => Err(Error::not_found(format!("no record '{id}' in '{}'", c.id))),
    }
}

fn item_path(collection: &str, id: &str) -> String {
    format!("items/{collection}/{id}.toml")
}
