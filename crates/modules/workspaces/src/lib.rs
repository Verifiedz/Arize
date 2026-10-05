//! `shimmer-workspaces`: launching configured sessions from user scripts (CLAUDE.md §10).
//!
//! So far the pure parts, plain functions over plain data with no `Ctx` and no I/O, callable
//! from a test with no runtime (§12 rule 10):
//! - [`state`]: the `ready`/`launching`/`active`/`dirty` state machine (§10.3);
//! - [`manifest`]: `workspace.toml` and the `steps/` folder, checked per ADR 0012;
//! - [`template`]: workspace templates and their questions (ADR 0025).
//!
//! The `Module` itself, which runs steps through `ctx.launcher` (ADR 0010), comes next (#32).
//!
//! Depends on `core` only.

mod builtin;
mod launch;
pub mod manifest;
mod module;
pub mod persist;
pub mod state;
pub mod template;

pub use manifest::{parse, script_name, ScriptExt, StepSpec, Workspace};
pub use module::{Workspaces, LANE};
pub use state::{
    activate, cleanup_succeeded, dirty_error, force_relaunch, launch_abandoned, launch_succeeded, reset, start_cleanup,
    step_failed, FailedStep, WorkspaceState,
};
