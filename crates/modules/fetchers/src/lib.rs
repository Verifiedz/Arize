//! `shimmer-fetchers`: reaching outward -- scheduling, dedup, backoff (CLAUDE.md §8, ADR
//! 0028). Rate limiting and caching are not this module's job; they live behind `ctx.http`
//! (ADR 0027).
//!
//! This is the skeleton only: an inert, registered `Module` with no commands, no lanes, no
//! triggers and no capabilities yet. `Source`, the dedup store, scheduling and the two
//! reference sources (ADR 0028 §10) land in later changes.
//!
//! Depends on `core` only.

mod module;

pub use module::Fetchers;
