//! `shimmer-records`: typed collections of records (CLAUDE.md §8). A LeetCode tracker, a job tracker
//! or any user-defined tracker is a collection TOML; this module is the one implementation of
//! storage, validation and filtering behind all of them. Layout and ops: ADR 0008.
//!
//! Depends on `core` only. Everything durable goes through `ctx.store`, one transaction per
//! change, with its event committed alongside (§7.1).

mod collections;
mod item;
mod lifecycle;
mod query;
mod schema;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use shimmer_core::ids::is_valid_name;
use shimmer_core::params::decode;
use shimmer_core::{CommandSpec, Ctx, Error, ErrorCode, Execution, Manifest, Module, Result, WriteLock};

use crate::item::Item;
use crate::schema::Collection;

/// Built-in collection templates (ADR 0022), by id. Each is an ordinary collection file whose
/// `[collection] id` is the template's id; creating one copies it with only the id (and label)
/// changed.
const TEMPLATES: &[(&str, &str)] = &[
    ("addresses", include_str!("../templates/addresses.toml")),
    ("certifications", include_str!("../templates/certifications.toml")),
    ("charity", include_str!("../templates/charity.toml")),
    ("education", include_str!("../templates/education.toml")),
    ("employment", include_str!("../templates/employment.toml")),
    ("interview-questions", include_str!("../templates/interview-questions.toml")),
    ("interviews", include_str!("../templates/interviews.toml")),
    ("job-applications", include_str!("../templates/job-applications.toml")),
    ("leetcode", include_str!("../templates/leetcode.toml")),
    ("networking-events", include_str!("../templates/networking-events.toml")),
    ("offers", include_str!("../templates/offers.toml")),
    ("outreach", include_str!("../templates/outreach.toml")),
    ("projects", include_str!("../templates/projects.toml")),
    ("stories", include_str!("../templates/stories.toml")),
    ("subscriptions", include_str!("../templates/subscriptions.toml")),
];
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
                "records.item.renamed",
                "records.item.restored",
                "records.field.renamed",
                "records.collection.renamed",
                "records.collection.removed",
                "records.collection.restored",
                "records.item.removed",
                "records.collection.created",
            ]
            .map(String::from)
            .to_vec(),
            capabilities: vec![],
        }
    }

    /// Nothing to set up: a fresh install starts with no collections, and the person creates the
    /// ones they want from templates (ADR 0023, replacing ADR 0008's seeding).
    async fn init(&self, _ctx: &Ctx) -> Result<()> {
        Ok(())
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
            (
                "records.add",
                "Add a record to a collection; without an id, one is made from its title",
                json!({"type": "object", "required": ["collection"], "properties": {
                    "collection": {"type": "string"}, "id": {"type": "string"}, "fields": {"type": "object"}}}),
            ),
            ("records.get", "Get one record", with(json!({}))),
            (
                "records.list",
                "List records in a collection",
                json!({"type": "object", "required": ["collection"], "properties": {
                    "collection": {"type": "string"}, "filter": {"type": "object"},
                    "search": {"type": "string"}, "sort": {"type": "array", "items": {"type": "string"}},
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
            (
                "records.rename",
                "Give a record a new id",
                json!({"type": "object", "required": ["collection", "id", "new_id"], "properties": {
                    "collection": {"type": "string"}, "id": {"type": "string"}, "new_id": {"type": "string"}}}),
            ),
            (
                "records.remove",
                "Remove a record (it goes to the trash; records.restore brings it back)",
                with(json!({})),
            ),
            ("records.restore", "Bring a removed record back from the trash", with(json!({}))),
            (
                "records.trash",
                "List a collection's removed records",
                json!({"type": "object", "required": ["collection"], "properties": {"collection": {"type": "string"}}}),
            ),
            ("records.templates", "List the built-in collection templates", json!({"type": "object"})),
            (
                "records.create_collection",
                "Create a collection from a template",
                json!({"type": "object", "required": ["id", "template"], "properties": {
                    "id": {"type": "string"}, "template": {"type": "string"}, "label": {"type": "string"}}}),
            ),
            (
                "records.check",
                "Report records that don't fit their collection; changes nothing",
                json!({"type": "object", "required": ["collection"], "properties": {"collection": {"type": "string"}}}),
            ),
            (
                "records.rename_field",
                "Rename a field in a collection and every record",
                json!({"type": "object", "required": ["collection", "from", "to"], "properties": {
                    "collection": {"type": "string"}, "from": {"type": "string"}, "to": {"type": "string"}}}),
            ),
            (
                "records.rename_collection",
                "Give a collection a new id",
                json!({"type": "object", "required": ["id", "new_id"], "properties": {
                    "id": {"type": "string"}, "new_id": {"type": "string"}}}),
            ),
            (
                "records.remove_collection",
                "Remove a collection and its records (they can be restored); needs confirmation",
                json!({"type": "object", "required": ["id"], "properties": {
                    "id": {"type": "string"}, "confirm": {"type": "object"}}}),
            ),
            (
                "records.restore_collection",
                "Bring a removed collection back with its records",
                json!({"type": "object", "required": ["id"], "properties": {"id": {"type": "string"}}}),
            ),
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
            "records.rename" => self.rename(ctx, decode(params)?),
            "records.remove" => self.remove(ctx, decode(params)?),
            "records.restore" => self.restore(ctx, decode(params)?),
            "records.trash" => self.trash(ctx, decode(params)?),
            "records.check" => self.check(ctx, decode(params)?),
            "records.templates" => self.templates(),
            "records.create_collection" => self.create_collection(ctx, decode(params)?),
            "records.rename_field" => self.rename_field(ctx, decode(params)?),
            "records.rename_collection" => self.rename_collection(ctx, decode(params)?),
            "records.remove_collection" => self.remove_collection(ctx, decode(params)?),
            "records.restore_collection" => self.restore_collection(ctx, decode(params)?),
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

/// `records.add`: the id is optional (ADR 0018 §1).
#[derive(Deserialize)]
struct Add {
    collection: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    fields: Map<String, Value>,
}

#[derive(Deserialize)]
struct CreateCollection {
    id: String,
    template: String,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Deserialize)]
struct CollectionOnly {
    collection: String,
}

#[derive(Deserialize)]
struct RenameField {
    collection: String,
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct CollectionId {
    id: String,
}

#[derive(Deserialize)]
struct RenameCollection {
    id: String,
    new_id: String,
}

#[derive(Deserialize)]
struct RemoveCollection {
    id: String,
    #[serde(default)]
    confirm: Option<Value>,
}

#[derive(Deserialize)]
struct Rename {
    collection: String,
    id: String,
    new_id: String,
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
    search: Option<String>,
    #[serde(default)]
    sort: Vec<String>,
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

    fn add(&self, ctx: &Ctx, p: Add) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        if let Some(id) = &p.id {
            item::check_id(id)?;
        }
        if let Some((k, _)) = p.fields.iter().find(|(_, v)| v.is_null()) {
            return Err(Error::invalid_params(format!("field '{k}' is null; leave it out to leave it unset")));
        }
        c.check_values(&p.fields)?;
        c.check_required(&p.fields)?;

        // Under the write lock, so nothing can take the id or a unique value in between.
        let _g = self.write.lock();
        let exists = |id: &str| Ok(ctx.store.read_string(&item_path(&c.id, id))?.is_some());
        let id = match p.id {
            Some(id) if exists(&id)? => {
                return Err(Error::conflict(format!("'{}' already has a record '{id}'", c.id)));
            }
            Some(id) => id,
            // ADR 0018 §1: from the title, else from today's date; never clashing.
            None => {
                let fields = p.fields.clone().into_iter().collect();
                match c.render_title(&fields).map(|t| item::slug(&t)).filter(|s| !s.is_empty()) {
                    Some(base) => item::first_free(&base, false, exists)?,
                    None => item::first_free(&today(ctx), true, exists)?,
                }
            }
        };
        let mut item = Item::new(&id, p.fields);
        item.status_line = c.completable;
        let wire = item.to_wire(&c);
        let path = item_path(&c.id, &id);
        check_unique(ctx, &c, &item.id, &item.fields_map())?;
        ctx.store.transaction(|tx| {
            tx.put(&path, item.to_toml())?;
            tx.emit("records.item.created", payload(&c, &item, &wire))
        })?;
        Ok(wire)
    }

    fn list(&self, ctx: &Ctx, p: ListParams) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        let query = query::Query::new(&c, &p.filter, p.search.as_deref(), &p.sort)?;
        let limit = p.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(Error::invalid_params(format!("limit must be between 1 and {MAX_LIMIT}")));
        }

        let (mut matching, mut skipped) = (Vec::new(), Vec::new());
        let dir = format!("items/{}", c.id);
        for path in ctx.store.list(&dir)? {
            let Some(id) = path.strip_prefix(&format!("{dir}/")).and_then(|p| p.strip_suffix(".toml")) else {
                continue;
            };
            // One bad hand-edit must not hide every other record.
            match ctx.store.read_string(&path)?.map(|text| Item::from_toml(id, &text)) {
                Some(Ok(item)) => {
                    let wire = item.to_wire(&c);
                    if query.matches(&wire) {
                        matching.push(wire);
                    }
                }
                Some(Err(e)) => skipped.push(e.message),
                None => {}
            }
        }
        // By the requested keys, then by id. Never by path: `two-sum-again.toml` sorts before
        // `two-sum.toml` ('-' < '.'), which is not id order (ADR 0019 §3).
        query.sort(&mut matching);
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
        let today = today(ctx);

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

    /// Move a record to a new id: one transaction writes the new file, deletes the old one, and
    /// emits `records.item.renamed` (ADR 0018 §2). Values are untouched, so no unique check.
    fn rename(&self, ctx: &Ctx, p: Rename) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        item::check_id(&p.new_id)?;
        if p.new_id == p.id {
            return Err(Error::invalid_params(format!("'{}' is already called that", p.id)));
        }
        let _g = self.write.lock();
        let mut item = load_item(ctx, &c, &p.id)?;
        let new_path = item_path(&c.id, &p.new_id);
        if ctx.store.read_string(&new_path)?.is_some() {
            return Err(Error::conflict(format!("'{}' already has a record '{}'", c.id, p.new_id)));
        }
        item.id = p.new_id.clone();
        let wire = item.to_wire(&c);
        let mut event = payload(&c, &item, &wire);
        event["id"] = json!(p.id);
        event["new_id"] = json!(p.new_id);
        ctx.store.transaction(|tx| {
            tx.put(&new_path, item.to_toml())?;
            tx.delete(&item_path(&c.id, &p.id))?;
            tx.emit("records.item.renamed", event)
        })?;
        Ok(wire)
    }

    /// Every built-in template, as `records.collections` shows a collection (ADR 0022 §2).
    fn templates(&self) -> Result<Value> {
        let templates = TEMPLATES
            .iter()
            .map(|(id, text)| Collection::parse(id, text).map(|c| serde_json::to_value(c).unwrap_or_default()))
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({"templates": templates}))
    }

    /// A new collection from a template: its text copied with only the id (and label) changed,
    /// checked, and written with its event in one transaction. Never overwrites (ADR 0022 §2).
    fn create_collection(&self, ctx: &Ctx, p: CreateCollection) -> Result<Value> {
        if !is_valid_name(&p.id) {
            return Err(Error::invalid_params(format!(
                "'{}' is not a valid collection id: lowercase letters, digits, '-' or '_', starting with a letter",
                p.id
            )));
        }
        let Some((_, text)) = TEMPLATES.iter().find(|(id, _)| *id == p.template) else {
            let names: Vec<&str> = TEMPLATES.iter().map(|(id, _)| *id).collect();
            return Err(Error::not_found(format!("no template '{}' (there are: {})", p.template, names.join(", "))));
        };
        let mut text = collections::set_id(&p.template, text, &p.id)?;
        if let Some(label) = &p.label {
            if label.trim().is_empty() {
                return Err(Error::invalid_params("label must not be empty"));
            }
            text = collections::set_label(&p.id, &text, label)?;
        }
        // A template that can't produce a valid collection is a bug: fail, don't write it.
        let c = Collection::parse(&p.id, &text)?;

        let _g = self.write.lock();
        if ctx.store.read_string(&collection_path(&p.id))?.is_some() {
            return Err(Error::conflict(format!("there is already a collection '{}'", p.id)));
        }
        ctx.store.transaction(|tx| {
            tx.put(&collection_path(&p.id), text)?;
            tx.emit("records.collection.created", json!({"collection": p.id, "template": p.template}))
        })?;
        Ok(serde_json::to_value(c).unwrap_or_default())
    }

    /// Every record that doesn't fit the collection, and why. Writes nothing (ADR 0021 §2).
    fn check(&self, ctx: &Ctx, p: CollectionOnly) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        let dir = format!("items/{}", c.id);
        let (mut checked, mut out) = (0, Vec::new());
        for path in ctx.store.list(&dir)? {
            let Some(id) = path.strip_prefix(&format!("{dir}/")).and_then(|p| p.strip_suffix(".toml")) else {
                continue;
            };
            checked += 1;
            let found = match ctx.store.read_string(&path)?.map(|text| Item::from_toml(id, &text)) {
                Some(Ok(item)) => collections::problems(&c, &item),
                Some(Err(e)) => vec![e.message],
                None => continue,
            };
            if !found.is_empty() {
                out.push(json!({"id": id, "problems": found}));
            }
        }
        Ok(json!({"checked": checked, "problems": out}))
    }

    /// Rename a field in the collection file (comments kept) and in every record that has it,
    /// live or trashed, in one transaction (ADR 0021 §3).
    fn rename_field(&self, ctx: &Ctx, p: RenameField) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        if c.field(&p.from).is_none() {
            return Err(Error::invalid_params(format!("'{}' has no field '{}'", c.id, p.from)));
        }
        if !is_valid_name(&p.to) || schema::RESERVED.contains(&p.to.as_str()) {
            return Err(Error::invalid_params(format!("'{}' is not a valid field name", p.to)));
        }
        if c.field(&p.to).is_some() {
            return Err(Error::conflict(format!("'{}' already has a field '{}'", c.id, p.to)));
        }
        let _g = self.write.lock();
        let text = collection_text(ctx, &c.id)?;
        let renamed = collections::rename_field(&c.id, &text, &p.from, &p.to)?;
        let new = Collection::parse(&c.id, &renamed)?;

        let mut writes = Vec::new();
        for dir in [format!("items/{}", c.id), format!("trash/{}", c.id)] {
            for path in ctx.store.list(&dir)? {
                let Some(id) = path.strip_prefix(&format!("{dir}/")).and_then(|p| p.strip_suffix(".toml")) else {
                    continue;
                };
                let Some(Ok(mut item)) = ctx.store.read_string(&path)?.map(|text| Item::from_toml(id, &text)) else {
                    continue;
                };
                let Some(value) = item.fields.remove(&p.from) else { continue };
                if item.fields.contains_key(&p.to) {
                    return Err(Error::conflict(format!(
                        "record '{id}' already has a key '{}'; rename or remove it first",
                        p.to
                    )));
                }
                item.fields.insert(p.to.clone(), value);
                writes.push((path, item.to_toml()));
            }
        }
        let updated = writes.len();
        ctx.store.transaction(|tx| {
            tx.put(&collection_path(&c.id), renamed)?;
            for (path, text) in writes {
                tx.put(&path, text)?;
            }
            tx.emit("records.field.renamed", json!({"collection": c.id, "from": p.from, "to": p.to}))
        })?;
        Ok(json!({"collection": new, "updated": updated}))
    }

    /// Move a collection, its records and its trash to a new id (ADR 0021 §4).
    fn rename_collection(&self, ctx: &Ctx, p: RenameCollection) -> Result<Value> {
        let c = load_collection(ctx, &p.id)?;
        if !is_valid_name(&p.new_id) {
            return Err(Error::invalid_params(format!("'{}' is not a valid collection id", p.new_id)));
        }
        let _g = self.write.lock();
        if ctx.store.read_string(&collection_path(&p.new_id))?.is_some() {
            return Err(Error::conflict(format!("there is already a collection '{}'", p.new_id)));
        }
        let text = collections::set_id(&c.id, &collection_text(ctx, &c.id)?, &p.new_id)?;
        let new = Collection::parse(&p.new_id, &text)?;
        let moves = [
            subtree(ctx, &format!("items/{}", c.id), &format!("items/{}", p.new_id))?,
            subtree(ctx, &format!("trash/{}", c.id), &format!("trash/{}", p.new_id))?,
        ]
        .concat();
        ctx.store.transaction(|tx| {
            tx.put(&collection_path(&p.new_id), text)?;
            tx.delete(&collection_path(&c.id))?;
            apply_moves(tx, moves)?;
            tx.emit("records.collection.renamed", json!({"collection": c.id, "new_id": p.new_id}))
        })?;
        Ok(serde_json::to_value(new).unwrap_or_default())
    }

    /// Move a collection and everything in it to `removed-collections/<id>/`, after the caller
    /// confirms the record count it was shown (ADR 0021 §5).
    fn remove_collection(&self, ctx: &Ctx, p: RemoveCollection) -> Result<Value> {
        if !is_valid_name(&p.id) || ctx.store.read_string(&collection_path(&p.id))?.is_none() {
            return Err(Error::not_found(format!("no collection '{}'", p.id)));
        }
        let _g = self.write.lock();
        let items = format!("items/{}", p.id);
        let records = ctx.store.list(&items)?.len();
        let expected = json!({"records": records});
        if p.confirm.as_ref() != Some(&expected) {
            return Err(Error::new(
                ErrorCode::ConfirmationRequired,
                format!("removing '{}' removes its {records} record(s); confirm with {{\"records\": {records}}}", p.id),
            )
            .with_detail(expected));
        }
        let to = format!("removed-collections/{}", p.id);
        let mut moves = vec![(collection_path(&p.id), format!("{to}/collection.toml"))];
        moves.extend(subtree(ctx, &items, &format!("{to}/items"))?);
        moves.extend(subtree(ctx, &format!("trash/{}", p.id), &format!("{to}/trash"))?);
        // A collection removed earlier under the same id is replaced: the latest wins, as in the
        // record trash (ADR 0020 §1).
        let stale = ctx.store.list(&to)?;
        ctx.store.transaction(|tx| {
            for path in &stale {
                tx.delete(path)?;
            }
            apply_moves(tx, moves)?;
            tx.emit("records.collection.removed", json!({"collection": p.id, "records": records}))
        })?;
        Ok(json!({"removed": true, "records": records}))
    }

    /// Bring a removed collection back with everything it had (ADR 0021 §5).
    fn restore_collection(&self, ctx: &Ctx, p: CollectionId) -> Result<Value> {
        let from = format!("removed-collections/{}", p.id);
        let missing = || Error::not_found(format!("no removed collection '{}'", p.id));
        if !is_valid_name(&p.id) {
            return Err(missing());
        }
        let _g = self.write.lock();
        let text = ctx.store.read_string(&format!("{from}/collection.toml"))?.ok_or_else(missing)?;
        if ctx.store.read_string(&collection_path(&p.id))?.is_some()
            || !ctx.store.list(&format!("items/{}", p.id))?.is_empty()
        {
            return Err(Error::conflict(format!("there is a collection '{}' again; rename it first", p.id)));
        }
        let c = Collection::parse(&p.id, &text)?;
        let mut moves = vec![(format!("{from}/collection.toml"), collection_path(&p.id))];
        moves.extend(subtree(ctx, &format!("{from}/items"), &format!("items/{}", p.id))?);
        moves.extend(subtree(ctx, &format!("{from}/trash"), &format!("trash/{}", p.id))?);
        ctx.store.transaction(|tx| {
            apply_moves(tx, moves)?;
            tx.emit("records.collection.restored", json!({"collection": p.id}))
        })?;
        Ok(serde_json::to_value(c).unwrap_or_default())
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

    /// Move the record to the trash and say what went, in one transaction (ADR 0020 §1–2).
    fn remove(&self, ctx: &Ctx, t: Target) -> Result<Value> {
        let c = load_collection(ctx, &t.collection)?;
        let _g = self.write.lock();
        let item = load_item(ctx, &c, &t.id)?;
        let event = payload(&c, &item, &item.to_wire(&c));
        ctx.store.transaction(|tx| {
            tx.put(&trash_path(&c.id, &t.id), item.to_toml())?;
            tx.delete(&item_path(&c.id, &t.id))?;
            tx.emit("records.item.removed", event)
        })?;
        Ok(json!({"removed": true}))
    }

    /// Bring a removed record back exactly as it was (ADR 0020 §3).
    fn restore(&self, ctx: &Ctx, t: Target) -> Result<Value> {
        let c = load_collection(ctx, &t.collection)?;
        item::check_id(&t.id)?;
        let _g = self.write.lock();
        let text = ctx
            .store
            .read_string(&trash_path(&c.id, &t.id))?
            .ok_or_else(|| Error::not_found(format!("no removed record '{}' in '{}'", t.id, c.id)))?;
        if ctx.store.read_string(&item_path(&c.id, &t.id))?.is_some() {
            return Err(Error::conflict(format!(
                "'{}' already has a record '{}' again; rename one of them first",
                c.id, t.id
            )));
        }
        let item = Item::from_toml(&t.id, &text)?;
        check_unique(ctx, &c, &t.id, &item.fields_map())?;
        let wire = item.to_wire(&c);
        let event = payload(&c, &item, &wire);
        ctx.store.transaction(|tx| {
            tx.put(&item_path(&c.id, &t.id), text)?;
            tx.delete(&trash_path(&c.id, &t.id))?;
            tx.emit("records.item.restored", event)
        })?;
        Ok(wire)
    }

    /// The removed records that could be restored, by id (ADR 0020 §4).
    fn trash(&self, ctx: &Ctx, p: CollectionOnly) -> Result<Value> {
        let c = load_collection(ctx, &p.collection)?;
        let dir = format!("trash/{}", c.id);
        let mut items: Vec<Value> = Vec::new();
        for path in ctx.store.list(&dir)? {
            let Some(id) = path.strip_prefix(&format!("{dir}/")).and_then(|p| p.strip_suffix(".toml")) else {
                continue;
            };
            if let Some(Ok(item)) = ctx.store.read_string(&path)?.map(|text| Item::from_toml(id, &text)) {
                items.push(item.to_wire(&c));
            }
        }
        items.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        Ok(json!({"items": items}))
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

/// Today's local date (ADR 0009), `YYYY-MM-DD`: a late-evening action west of UTC must not
/// count as tomorrow.
fn today(ctx: &Ctx) -> String {
    shimmer_core::local_date(ctx.clock.now(), &ctx.local_tz).format("%Y-%m-%d").to_string()
}

fn load_collection(ctx: &Ctx, id: &str) -> Result<Collection> {
    let missing = || Error::not_found(format!("no collection '{id}'"));
    if !is_valid_name(id) {
        return Err(missing());
    }
    let text = ctx.store.read_string(&collection_path(id))?.ok_or_else(missing)?;
    Collection::parse(id, &text)
}

fn load_item(ctx: &Ctx, c: &Collection, id: &str) -> Result<Item> {
    item::check_id(id)?;
    match ctx.store.read_string(&item_path(&c.id, id))? {
        Some(text) => Item::from_toml(id, &text),
        None => Err(Error::not_found(format!("no record '{id}' in '{}'", c.id))),
    }
}

fn collection_path(id: &str) -> String {
    format!("collections/{id}.toml")
}

/// The collection file as written, for edits that keep its comments (ADR 0021).
fn collection_text(ctx: &Ctx, id: &str) -> Result<String> {
    ctx.store.read_string(&collection_path(id))?.ok_or_else(|| Error::not_found(format!("no collection '{id}'")))
}

/// Every file under `from`, paired with where it goes under `to`, contents read now.
fn subtree(ctx: &Ctx, from: &str, to: &str) -> Result<Vec<(String, String)>> {
    let prefix = format!("{from}/");
    Ok(ctx
        .store
        .list(from)?
        .into_iter()
        .filter_map(|path| path.strip_prefix(&prefix).map(|rest| (path.clone(), format!("{to}/{rest}"))))
        .collect())
}

/// Move files inside a transaction: write each at its new path, delete the old one.
fn apply_moves(tx: &mut shimmer_core::Tx<'_>, moves: Vec<(String, String)>) -> Result<()> {
    for (from, to) in moves {
        let bytes = tx.read(&from)?.unwrap_or_default();
        tx.put(&to, bytes)?;
        tx.delete(&from)?;
    }
    Ok(())
}

/// Where a removed record waits to be restored (ADR 0020 §1).
fn trash_path(collection: &str, id: &str) -> String {
    format!("trash/{collection}/{id}.toml")
}

fn item_path(collection: &str, id: &str) -> String {
    format!("items/{collection}/{id}.toml")
}
