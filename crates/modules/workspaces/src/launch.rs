//! The queued ops that run scripts: `activate`, `force_relaunch` and `cleanup` (CLAUDE.md §10).
//!
//! Scripts run only through `ctx.launcher` (ADR 0010), one step at a time, in order. The first
//! step that fails stops the launch and leaves the workspace `dirty` (ADR 0010 §2a, §10.3), so
//! a detached step never starts after a failed setup step. Nothing is retried (§11.2).
//!
//! Each change of state is written with its event in one transaction (§7.1). The write lock is
//! taken only around those writes, never across `launcher.run`, which can take minutes.

use std::time::Duration;

use serde_json::{json, Value};
use shimmer_core::{Ctx, Error, ErrorCode, LaunchStep, Result, SpawnMode, Step, StepOutcome};

use crate::manifest::{ScriptExt, Workspace};
use crate::module::{dirty_payload, folder, state_path, Workspaces};
use crate::persist::{self, LastSession, Outcome, Saved};
use crate::state::{self, FailedStep, WorkspaceState};

impl Workspaces {
    /// `workspaces.activate`: `ready`/`active → launching →` `active`, or `dirty` if a step fails.
    pub(crate) async fn activate(&self, ctx: &Ctx, id: &str) -> Result<Value> {
        let start = self.prepare(ctx, id, |saved, has_cleanup| state::activate(&saved.state, id, has_cleanup))?;
        self.launch(ctx, id, start, false).await
    }

    /// `workspaces.force_relaunch`: the explicit override for a dirty workspace (§10.3). It is
    /// recorded before anything runs (`workspaces.session.forced`, with the dirty reason it
    /// overrides), then it is an ordinary launch that can fail again.
    pub(crate) async fn force_relaunch(&self, ctx: &Ctx, id: &str) -> Result<Value> {
        let start = self.prepare(ctx, id, |saved, _| state::force_relaunch(&saved.state))?;
        self.launch(ctx, id, start, true).await
    }

    /// `workspaces.cleanup`: run `cleanup.{sh,ps1}`, supervised (§10.3). Success moves
    /// `dirty → ready`; failure leaves it dirty and fails the task with the log.
    pub(crate) async fn cleanup(&self, ctx: &Ctx, id: &str) -> Result<Value> {
        let (workspace, timeout) = {
            let _g = self.lock();
            let folder = folder(ctx, id)?;
            let saved = folder.saved(ctx, id).unwrap_or_else(|| Ok(Saved::ready()))?;
            state::start_cleanup(&saved.state)?;
            if !folder.has_cleanup_script() {
                return Err(Error::invalid_params(format!(
                    "{id} has no cleanup script; workspaces.reset clears dirty without one"
                )));
            }
            let script = format!("cleanup.{}", ScriptExt::this_platform().as_str());
            if !folder.has(&script) {
                return Err(Error::invalid_params(format!("{id}: missing {script} for this platform")));
            }
            let workspace = folder.load(ctx, id)?;
            let timeout =
                workspace.cleanup_timeout.ok_or_else(|| Error::internal("cleanup script without a timeout"))?;
            (workspace, timeout)
        };

        ctx.progress(0.0, "cleanup");
        let step = launch_step(&workspace, Step::Cleanup, SpawnMode::Supervised { timeout });
        let out = ctx.launcher.run(&step, &ctx.cancel).await?;
        if ctx.cancel.is_cancelled() {
            return Err(Error::new(ErrorCode::ModuleError, format!("{id}: cleanup was cancelled; still dirty")));
        }
        if let Some(reason) = failure(step.mode, &out) {
            return Err(Error::new(ErrorCode::ModuleError, format!("{id}: cleanup failed ({reason}); still dirty"))
                .with_detail(json!({"workspace": id, "reason": reason, "log": out.log_path})));
        }

        let _g = self.lock();
        // Re-read: a `reset` may have run while the script did.
        let saved = folder(ctx, id)?.saved(ctx, id).unwrap_or_else(|| Ok(Saved::ready()))?;
        let next = state::cleanup_succeeded(&saved.state)?;
        ctx.store.transaction(|tx| {
            tx.put(&state_path(id), persist::encode(&Saved::new(next).with_last_session(saved.last_session.clone())))?;
            tx.emit("workspaces.session.cleaned", json!({"workspace": id, "log": out.log_path}))
        })?;
        ctx.progress(1.0, "cleaned up");
        Ok(json!({"id": id, "state": "ready"}))
    }

    /// Check the workspace can launch and claim it: under the lock, read its state, apply the
    /// transition (`activate` or `force_relaunch`), check every step has its script for this
    /// platform, and save `launching`. Nothing runs if any of that fails.
    fn prepare(
        &self,
        ctx: &Ctx,
        id: &str,
        transition: impl FnOnce(&Saved, bool) -> Result<WorkspaceState>,
    ) -> Result<Start> {
        let _g = self.lock();
        let folder = folder(ctx, id)?;
        let prior = folder.saved(ctx, id).unwrap_or_else(|| Ok(Saved::ready()))?;
        let has_cleanup = folder.has_cleanup_script();
        // State first: "it's dirty" matters more than a typo in workspace.toml.
        let next = transition(&prior, has_cleanup)?;
        let workspace = folder.load(ctx, id)?;
        workspace.check_scripts(&folder.files, ScriptExt::this_platform())?;
        let forced = matches!(prior.state, WorkspaceState::Dirty { .. });
        ctx.store.transaction(|tx| {
            let launching = Saved { state: next, running_step: None, last_session: prior.last_session.clone() };
            tx.put(&state_path(id), persist::encode(&launching))?;
            if forced {
                tx.emit(
                    "workspaces.session.forced",
                    json!({"workspace": id, "prior": dirty_payload(id, &prior.state)}),
                )?;
            }
            Ok(())
        })?;
        Ok(Start { workspace, prior, has_cleanup, started_at: ctx.clock.now() })
    }

    /// Run every step in order. Saved as `launching` (with the running step) throughout, then
    /// `active`, or `dirty` at the first failure, cancel, or step that can't be started.
    async fn launch(&self, ctx: &Ctx, id: &str, start: Start, forced: bool) -> Result<Value> {
        let Start { workspace, prior, has_cleanup, started_at } = start;
        let steps: Vec<(Step, SpawnMode)> = workspace.launch_steps().collect();
        let count = steps.len() as u32;
        let mut session_id: Option<String> = None;

        for (i, (step, mode)) in steps.into_iter().enumerate() {
            let index = i as u32 + 1;
            let current = FailedStep { index, count, name: workspace.steps[i].name.clone() };
            let fail = |reason: String, log: &str, outcome: Outcome, session_id: &Option<String>| {
                self.finish_dirty(
                    ctx,
                    id,
                    current.clone(),
                    reason,
                    log,
                    outcome,
                    session_id,
                    started_at,
                    forced,
                    has_cleanup,
                )
            };

            if ctx.cancel.is_cancelled() {
                if i == 0 {
                    // Nothing has run yet: put the workspace back exactly as it was.
                    self.restore(ctx, id, &prior)?;
                    return Err(Error::new(ErrorCode::ModuleError, format!("{id}: cancelled before any step ran")));
                }
                return Err(fail(format!("cancelled before step {current}"), "", Outcome::Cancelled, &session_id)?);
            }

            ctx.progress(i as f32 / count as f32, &format!("step {current}"));
            self.save_running(ctx, id, &prior, &current)?;

            let out = match ctx.launcher.run(&launch_step(&workspace, step, mode), &ctx.cancel).await {
                Ok(out) => out,
                Err(e) if i == 0 => {
                    // The first step couldn't even start (e.g. no launch backend yet): nothing
                    // ran, so the workspace is not half-configured. Restore it and pass the error on.
                    self.restore(ctx, id, &prior)?;
                    return Err(e);
                }
                Err(e) => {
                    let reason = format!("could not start: {}", e.message);
                    return Err(fail(reason, "", Outcome::Dirty, &session_id)?);
                }
            };
            session_id.get_or_insert_with(|| out.session_id.clone());

            if ctx.cancel.is_cancelled() {
                let reason = format!("cancelled during step {current}");
                return Err(fail(reason, &out.log_path, Outcome::Cancelled, &session_id)?);
            }
            if let Some(reason) = failure(mode, &out) {
                return Err(fail(reason, &out.log_path, Outcome::Dirty, &session_id)?);
            }
        }

        let _g = self.lock();
        let active = state::launch_succeeded(&WorkspaceState::Launching)?;
        let session = session_id.unwrap_or_default();
        let last = LastSession { id: session.clone(), started_at, forced, outcome: Outcome::Launched };
        ctx.store.transaction(|tx| {
            tx.put(&state_path(id), persist::encode(&Saved::new(active).with_last_session(Some(last))))?;
            tx.emit(
                "workspaces.session.launched",
                json!({"workspace": id, "session_id": session, "forced": forced, "steps": count}),
            )
        })?;
        ctx.progress(1.0, "launched");
        Ok(json!({"id": id, "state": "active", "session_id": session}))
    }

    /// Save which step is about to run, so a daemon that dies now restarts knowing it.
    fn save_running(&self, ctx: &Ctx, id: &str, prior: &Saved, step: &FailedStep) -> Result<()> {
        let _g = self.lock();
        let saved = Saved {
            state: WorkspaceState::Launching,
            running_step: Some(step.clone()),
            last_session: prior.last_session.clone(),
        };
        ctx.store.write(&state_path(id), persist::encode(&saved))
    }

    /// Put back the state from before the launch was claimed.
    fn restore(&self, ctx: &Ctx, id: &str, prior: &Saved) -> Result<()> {
        let _g = self.lock();
        ctx.store.write(&state_path(id), persist::encode(prior))
    }

    /// Save `dirty` with its event, and return the error the task fails with: the same
    /// `workspace_dirty` a later `activate` gets, so the client can offer cleanup or the log.
    #[allow(clippy::too_many_arguments)]
    fn finish_dirty(
        &self,
        ctx: &Ctx,
        id: &str,
        failed_step: FailedStep,
        reason: String,
        log: &str,
        outcome: Outcome,
        session_id: &Option<String>,
        started_at: chrono::DateTime<chrono::Utc>,
        forced: bool,
        has_cleanup: bool,
    ) -> Result<Error> {
        let _g = self.lock();
        let dirty = state::step_failed(&WorkspaceState::Launching, reason, failed_step, ctx.clock.now(), log)?;
        let session = session_id.clone().unwrap_or_default();
        let last = LastSession { id: session.clone(), started_at, forced, outcome };
        ctx.store.transaction(|tx| {
            tx.put(&state_path(id), persist::encode(&Saved::new(dirty.clone()).with_last_session(Some(last))))?;
            let mut payload = dirty_payload(id, &dirty);
            payload["session_id"] = json!(session);
            payload["forced"] = json!(forced);
            payload["has_cleanup_script"] = json!(has_cleanup);
            // CLAUDE.md §10.3 wants the user told at elevated priority. `notify` doesn't exist
            // yet (M6): it will subscribe to this event.
            tx.emit("workspaces.session.dirty", payload)
        })?;
        Ok(state::dirty_error(&dirty, id, has_cleanup))
    }
}

/// What `prepare` hands to `launch`.
struct Start {
    workspace: Workspace,
    prior: Saved,
    has_cleanup: bool,
    started_at: chrono::DateTime<chrono::Utc>,
}

fn launch_step(workspace: &Workspace, step: Step, mode: SpawnMode) -> LaunchStep {
    LaunchStep {
        workspace_id: workspace.id.clone(),
        workspace_dir: workspace.id.clone(),
        step,
        mode,
        user_env: workspace.env.clone(),
    }
}

/// Did this step fail? A supervised step fails on a timeout, a non-zero exit, or being killed
/// by a signal. A detached step succeeds once it has started: Shimmer never waits on it
/// (ADR 0012 §5). Deciding this is the module's job, not the launcher's (ADR 0010 §8).
pub(crate) fn failure(mode: SpawnMode, out: &StepOutcome) -> Option<String> {
    let SpawnMode::Supervised { timeout } = mode else { return None };
    if out.timed_out {
        return Some(format!("timed out after {}", seconds(timeout)));
    }
    match out.exit_code {
        Some(0) => None,
        Some(code) => Some(format!("exit code {code}")),
        None => Some("killed by a signal".into()),
    }
}

fn seconds(d: Duration) -> String {
    format!("{}s", d.as_secs())
}
