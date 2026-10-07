//! `shimmer-fetchers`: reaching outward -- scheduling, dedup, backoff (CLAUDE.md §8, ADR
//! 0028). Rate limiting and caching are not this module's job; they live behind `ctx.http`
//! (ADR 0027).
//!
//! [`dedup`] has the per-source "seen" bookkeeping and [`RunGuard`](dedup::RunGuard), the
//! overlap guard (ADR 0028 §4, §5). [`module`]'s `Fetchers` wires `RunGuard` and the
//! heartbeat trigger/commands from [`schedule`] together, so scheduling now runs end to
//! end: `fetchers.tick` finds due sources and enqueues `fetchers.fetch` for each, which
//! records the run -- but there is still no `Source`, so a fetch does nothing yet. `SeenSet`
//! (`dedup`) gets its first real caller once one exists (ADR 0028 §10, a later change).
//!
//! Depends on `core` only.

pub mod dedup;
mod module;
mod schedule;

pub use module::Fetchers;
