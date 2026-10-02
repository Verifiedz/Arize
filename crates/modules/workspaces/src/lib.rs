//! `shimmer-workspaces`: launching configured sessions from user scripts (CLAUDE.md §10).
//!
//! So far only the `ready`/`launching`/`active`/`dirty` state machine (§10.3): plain functions
//! over plain data, no `Ctx`, no I/O, callable from a test with no runtime (§12 rule 10). The
//! `workspace.toml` parser (ADR 0012) and the `Module` itself, which runs steps through
//! `ctx.launcher` (ADR 0010), come next (issue #32).
//!
//! Depends on `core` only.

pub mod state;

pub use state::{
    activate, cleanup_succeeded, force_relaunch, launch_succeeded, reset, start_cleanup, step_failed, FailedStep,
    WorkspaceState,
};
