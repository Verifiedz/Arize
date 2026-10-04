//! The `workspaces` module (CLAUDE.md §10): everything that touches `ctx`. Decisions are made
//! by the pure parts ([`crate::state`], [`crate::manifest`], [`crate::persist`]); this file
//! reads files, calls them, and writes the result with its event in one transaction (§7.1).
//!
//! Layout, inside the module's namespace `data/workspaces/` (ADR 0010, ADR 0012):
//! `<id>/workspace.toml`, `<id>/steps/…`, `<id>/cleanup.{sh,ps1}` (all written by the user)
//! and `<id>/state.toml` (written only here).
//!
//! The ops that run scripts (`activate`, `force_relaunch`, `cleanup`) live in [`crate::launch`].

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use shimmer_core::{CommandSpec, Ctx, Error, Execution, LaneConfig, Manifest, Module, Result, SpawnMode};

use crate::manifest::{self, Workspace};
use crate::persist::{self, Saved, STATE_FILE};
use crate::state::{self, WorkspaceState};

/// Launches touch shared state (a desktop, a terminal multiplexer), so they run one at a time
/// (CLAUDE.md §6.1).
pub const LANE: &str = "workspaces";

#[derive(Default)]
pub struct Workspaces {
    /// Inline requests run concurrently; this serialises each read-check-write of a
    /// `state.toml`. Never held across an `.await`.
    write: Mutex<()>,
}

#[async_trait]
impl Module for Workspaces {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "workspaces".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            namespace: "workspaces".into(),
            topics: [
                "workspaces.session.launched",
                "workspaces.session.dirty",
                "workspaces.session.forced",
                "workspaces.session.cleaned",
                "workspaces.session.abandoned",
                "workspaces.workspace.reset",
            ]
            .map(String::from)
            .to_vec(),
            // Gets the real `ctx.launcher` (ADR 0010 §4).
            capabilities: vec!["process".into()],
        }
    }

    /// A workspace still saved as `launching` belongs to a launch the daemon never finished
    /// (it crashed or was killed). Mark it dirty: its earlier steps already ran.
    async fn init(&self, ctx: &Ctx) -> Result<()> {
        let _g = self.lock();
        for (id, folder) in folders(ctx)? {
            let Some(Ok(saved)) = folder.saved(ctx, &id) else { continue };
            let Some(dirty) = persist::recover_after_restart(&saved, ctx.clock.now()) else { continue };
            ctx.store.transaction(|tx| {
                let next = Saved::new(dirty.clone()).with_last_session(saved.last_session.clone());
                tx.put(&state_path(&id), persist::encode(&next))?;
                tx.emit("workspaces.session.dirty", dirty_payload(&id, &dirty))
            })?;
        }
        Ok(())
    }

    fn lanes(&self) -> Vec<LaneConfig> {
        vec![LaneConfig::new(LANE, 1)]
    }

    fn commands(&self) -> Vec<CommandSpec> {
        let id = json!({"type": "object", "required": ["id"], "properties": {"id": {"type": "string"}}});
        let queued = || Execution::Queued { lane: LANE.into() };
        [
            ("workspaces.list", "List workspaces and their state", json!({"type": "object"}), Execution::Inline),
            ("workspaces.status", "Show one workspace in full", id.clone(), Execution::Inline),
            ("workspaces.activate", "Launch a workspace's steps in order", id.clone(), queued()),
            ("workspaces.cleanup", "Run a dirty workspace's cleanup script", id.clone(), queued()),
            ("workspaces.force_relaunch", "Launch a dirty workspace anyway (logged as forced)", id.clone(), queued()),
            ("workspaces.reset", "Clear dirty without running cleanup (last resort)", id, Execution::Inline),
        ]
        .into_iter()
        .map(|(op, summary, params_schema, execution)| CommandSpec {
            op: op.into(),
            summary: summary.into(),
            params_schema,
            execution,
        })
        .collect()
    }

    async fn handle(&self, op: &str, params: Value, ctx: &Ctx) -> Result<Value> {
        match op {
            "workspaces.list" => self.list(ctx),
            "workspaces.status" => self.status(ctx, &parse::<Target>(params)?.id),
            "workspaces.reset" => self.reset(ctx, &parse::<Target>(params)?.id),
            "workspaces.activate" => self.activate(ctx, &parse::<Target>(params)?.id).await,
            "workspaces.force_relaunch" => self.force_relaunch(ctx, &parse::<Target>(params)?.id).await,
            "workspaces.cleanup" => self.cleanup(ctx, &parse::<Target>(params)?.id).await,
            _ => Err(Error::unknown_op(format!("workspaces has no op '{op}'"))),
        }
    }
}

#[derive(Deserialize)]
struct Target {
    id: String,
}

impl Workspaces {
    pub(crate) fn lock(&self) -> MutexGuard<'_, ()> {
        // The lock guards no data, so a poisoned one carries no information.
        self.write.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every workspace, by id. A broken one is listed as `invalid` with the reason; it never
    /// hides the others (ADR 0012 §6).
    fn list(&self, ctx: &Ctx) -> Result<Value> {
        let mut out = Vec::new();
        for (id, folder) in folders(ctx)? {
            let mut entry = Map::new();
            entry.insert("id".into(), json!(id));
            let (workspace, saved, errors) = folder.read(ctx, &id);
            if let Some(w) = &workspace {
                entry.insert("label".into(), json!(w.label));
            }
            match saved {
                Some(saved) if errors.is_empty() => add_state(&mut entry, &saved.state),
                _ => add_invalid(&mut entry, &errors),
            }
            out.push(Value::Object(entry));
        }
        Ok(json!({"workspaces": out}))
    }

    /// One workspace in full: its definition, state, and why it is dirty or invalid.
    fn status(&self, ctx: &Ctx, id: &str) -> Result<Value> {
        let folder = folder(ctx, id)?;
        let mut out = Map::new();
        out.insert("id".into(), json!(id));
        out.insert("has_cleanup_script".into(), json!(folder.has_cleanup_script()));
        let (workspace, saved, errors) = folder.read(ctx, id);
        if let Some(w) = &workspace {
            describe(&mut out, w);
        }
        // `null` until the first launch, as in crates/mockd/fixtures/workspace-dirty.json.
        let last = saved.as_ref().and_then(|s| s.last_session.as_ref()).map(|last| {
            json!({"id": last.id, "started_at": rfc3339(&last.started_at),
                   "forced": last.forced, "outcome": last.outcome.as_str()})
        });
        out.insert("last_session".into(), json!(last));
        match saved {
            // Dirty or running details only for a workspace that can be read: an invalid one shows
            // what's wrong, not a stale reason from before it broke.
            Some(saved) if errors.is_empty() => {
                add_state(&mut out, &saved.state);
                if let WorkspaceState::Dirty { reason, failed_step, failed_at, log } = &saved.state {
                    out.insert("log".into(), json!(log));
                    out.insert(
                        "dirty".into(),
                        json!({"reason": reason, "failed_step": failed_step.to_string(),
                               "failed_at": rfc3339(failed_at), "log": log}),
                    );
                }
                if let (WorkspaceState::Launching, Some(step)) = (&saved.state, &saved.running_step) {
                    out.insert("running_step".into(), json!(step.to_string()));
                }
            }
            _ => add_invalid(&mut out, &errors),
        }
        Ok(Value::Object(out))
    }

    /// `dirty → ready` without running cleanup (§10.3: the last resort when there is no
    /// cleanup script). Also clears a `state.toml` Shimmer can't read, which is the one way to
    /// recover from a hand-edit gone wrong.
    fn reset(&self, ctx: &Ctx, id: &str) -> Result<Value> {
        let _g = self.lock();
        let folder = folder(ctx, id)?;
        let (prior, last_session) = match folder.saved(ctx, id) {
            Some(Err(e)) => (json!({"unreadable_state": e.message}), None),
            saved => {
                let saved = saved.unwrap_or_else(|| Ok(Saved::ready()))?;
                state::reset(&saved.state)?;
                (dirty_payload(id, &saved.state), saved.last_session)
            }
        };
        ctx.store.transaction(|tx| {
            tx.put(&state_path(id), persist::encode(&Saved::ready().with_last_session(last_session)))?;
            tx.emit("workspaces.workspace.reset", json!({"workspace": id, "prior": prior}))
        })?;
        Ok(json!({"id": id, "state": "ready"}))
    }
}

// ---------------------------------------------------------------- reading the folders

/// One workspace folder's file list, relative to the folder (`workspace.toml`,
/// `steps/01-setup.sh`, `state.toml`, …), as `manifest::parse` takes it.
pub(crate) struct Folder {
    pub(crate) files: Vec<String>,
}

impl Folder {
    pub(crate) fn has(&self, name: &str) -> bool {
        self.files.iter().any(|f| f == name)
    }

    pub(crate) fn has_cleanup_script(&self) -> bool {
        self.has("cleanup.sh") || self.has("cleanup.ps1")
    }

    /// The checked `workspace.toml`, or why it isn't valid.
    pub(crate) fn load(&self, ctx: &Ctx, id: &str) -> Result<Workspace> {
        let text = ctx
            .store
            .read_string(&format!("{id}/workspace.toml"))?
            .ok_or_else(|| Error::invalid_params(format!("{id}: there is no workspace.toml")))?;
        manifest::parse(id, &text, &self.files)
    }

    /// Both files at once, with every problem found: `workspace.toml` and `state.toml` can each
    /// be broken, and the user should see both rather than fix one only to meet the other.
    /// `saved` is `Ready` when there's no `state.toml` yet, and `None` only if it can't be read.
    pub(crate) fn read(&self, ctx: &Ctx, id: &str) -> (Option<Workspace>, Option<Saved>, Vec<String>) {
        let mut errors = Vec::new();
        let workspace = self.load(ctx, id).map_err(|e| errors.push(e.message)).ok();
        let saved = match self.saved(ctx, id) {
            None => Some(Saved::ready()),
            Some(Ok(saved)) => Some(saved),
            Some(Err(e)) => {
                errors.push(e.message);
                None
            }
        };
        (workspace, saved, errors)
    }

    /// `None` when there is no `state.toml` yet.
    pub(crate) fn saved(&self, ctx: &Ctx, id: &str) -> Option<Result<Saved>> {
        if !self.has(STATE_FILE) {
            return None;
        }
        Some(ctx.store.read_string(&state_path(id)).and_then(|text| persist::decode(id, &text.unwrap_or_default())))
    }
}

/// Every workspace folder under the namespace, by id. Loose files at the top are ignored.
fn folders(ctx: &Ctx) -> Result<BTreeMap<String, Folder>> {
    let mut out: BTreeMap<String, Folder> = BTreeMap::new();
    for path in ctx.store.list("")? {
        if let Some((id, rest)) = path.split_once('/') {
            out.entry(id.to_owned()).or_insert_with(|| Folder { files: Vec::new() }).files.push(rest.to_owned());
        }
    }
    Ok(out)
}

/// The folder for `id`, listing only that folder. The id is checked against the workspace-id
/// charset first, so a request can never name a path outside it: anything else (`../records`,
/// `deep-work/steps`, empty) is simply `not_found`, as is an id with no folder.
pub(crate) fn folder(ctx: &Ctx, id: &str) -> Result<Folder> {
    let missing = || Error::not_found(format!("no workspace '{id}'"));
    if !manifest::valid_name(id) {
        return Err(missing());
    }
    let prefix = format!("{id}/");
    let files: Vec<String> =
        ctx.store.list(id)?.into_iter().filter_map(|p| p.strip_prefix(&prefix).map(str::to_owned)).collect();
    if files.is_empty() {
        return Err(missing());
    }
    Ok(Folder { files })
}

pub(crate) fn state_path(id: &str) -> String {
    format!("{id}/{STATE_FILE}")
}

// ---------------------------------------------------------------- output

/// `state: "invalid"` and every reason why, one per line.
fn add_invalid(out: &mut Map<String, Value>, errors: &[String]) {
    out.insert("state".into(), json!("invalid"));
    out.insert("error".into(), json!(errors.join("\n")));
}

fn add_state(out: &mut Map<String, Value>, state: &WorkspaceState) {
    if let WorkspaceState::Dirty { reason, failed_step, .. } = state {
        out.insert("dirty_reason".into(), json!(format!("step {failed_step}: {reason}")));
    }
    out.insert("state".into(), json!(state_name(state)));
}

/// The state's name on the wire (docs/protocol.md, "Workspace ops").
pub(crate) fn state_name(state: &WorkspaceState) -> &'static str {
    match state {
        WorkspaceState::Ready => "ready",
        WorkspaceState::Launching => "launching",
        WorkspaceState::Active => "active",
        WorkspaceState::Dirty { .. } => "dirty",
    }
}

fn describe(out: &mut Map<String, Value>, w: &Workspace) {
    out.insert("label".into(), json!(w.label));
    if let Some(d) = &w.description {
        out.insert("description".into(), json!(d));
    }
    let steps: Vec<Value> = w
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut step = json!({"index": i + 1, "name": s.name});
            match s.mode {
                SpawnMode::Detached => step["mode"] = json!("detached"),
                SpawnMode::Supervised { timeout } => {
                    step["mode"] = json!("supervised");
                    step["timeout_s"] = json!(timeout.as_secs());
                }
            }
            if let Some(d) = &s.description {
                step["description"] = json!(d);
            }
            step
        })
        .collect();
    out.insert("steps".into(), json!(steps));
    if let Some(t) = w.cleanup_timeout {
        out.insert("cleanup_timeout_s".into(), json!(t.as_secs()));
    }
}

/// `workspaces.session.dirty`'s payload, and `prior` in `workspaces.workspace.reset`.
pub(crate) fn dirty_payload(id: &str, state: &WorkspaceState) -> Value {
    match state {
        WorkspaceState::Dirty { reason, failed_step, failed_at, log } => json!({
            "workspace": id, "reason": reason, "failed_step": failed_step.to_string(),
            "failed_at": rfc3339(failed_at), "log": log,
        }),
        _ => json!({"workspace": id}),
    }
}

pub(crate) fn rfc3339(t: &chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T> {
    // Absent params decode as null.
    let params = if params.is_null() { json!({}) } else { params };
    serde_json::from_value(params).map_err(|e| Error::invalid_params(e.to_string()))
}
