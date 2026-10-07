//! `shimmer-fetchers`: reaching outward -- scheduling, dedup, backoff (CLAUDE.md §8, ADR
//! 0028). Rate limiting and caching are not this module's job; they live behind `ctx.http`
//! (ADR 0027).
//!
//! [`dedup`] has the per-source "seen" bookkeeping and the overlap guard (ADR 0028 §4, §5),
//! but `Module` itself ([`module`]) is still inert: no commands, no lanes, no triggers and
//! no capabilities yet. `Source`, scheduling and the two reference sources (ADR 0028 §10)
//! land in later changes, which is also where `dedup` gets its first real caller.
//!
//! Depends on `core` only.

pub mod dedup;
mod module;

pub use module::Fetchers;
