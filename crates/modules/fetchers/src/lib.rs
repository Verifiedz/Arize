//! `shimmer-fetchers`: reaching outward -- scheduling, dedup, backoff (CLAUDE.md §8, ADR
//! 0028). Rate limiting and caching are not this module's job; they live behind `ctx.http`
//! (ADR 0027).
//!
//! [`dedup`] has the per-source "seen" bookkeeping and [`RunGuard`](dedup::RunGuard), the
//! overlap guard (ADR 0028 §4, §5). [`module`]'s `Fetchers` wires `RunGuard` and the
//! heartbeat trigger/commands from [`schedule`] together, so scheduling runs end to end:
//! `fetchers.tick` finds due sources and enqueues `fetchers.fetch` for each, which records
//! the run. [`http`] is the retry wrapper around `ctx.http.get` (ADR 0028 §6); [`html`] and
//! [`robots`] are the scraping and robots.txt helpers a `Method::Scrape` source will use
//! (ADR 0028 §7). None of these four have a real caller yet -- there is still no `Source`
//! (ADR 0028 §10 lands in a later change), so `fetchers.fetch` does nothing beyond recording
//! that it ran.
//!
//! Depends on `core` only.

pub mod dedup;
pub mod html;
pub mod http;
mod module;
pub mod robots;
mod schedule;

pub use module::Fetchers;
