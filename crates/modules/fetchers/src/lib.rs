//! `shimmer-fetchers`: reaching outward -- scheduling, dedup, backoff (CLAUDE.md §8, ADR
//! 0028). Rate limiting and caching are not this module's job; they live behind `ctx.http`
//! (ADR 0027).
//!
//! [`source`] is the `Source` trait (ADR 0028 §2); [`sources`] holds one file per
//! implementation, starting with `hn-whoishiring` (ADR 0028 §10). [`dedup`] has the
//! per-source "seen" bookkeeping and [`RunGuard`](dedup::RunGuard), the overlap guard
//! (ADR 0028 §4, §5). [`http`] is the retry wrapper around `ctx.http.get` (ADR 0028 §6);
//! [`html`] and [`robots`] are the scraping and robots.txt helpers a future `Method::Scrape`
//! source will use (ADR 0028 §7) -- neither has a real caller yet, since `hn-whoishiring`
//! is `Method::Api`. [`module`]'s `Fetchers` wires all of this together: `fetchers.tick`
//! finds due sources and enqueues `fetchers.fetch` for each, which runs the matching
//! `Source`, dedupes its items, and emits `fetchers.item.found` for each new one.
//!
//! Depends on `core` only.

pub mod dedup;
pub mod html;
pub mod http;
mod module;
pub mod robots;
mod schedule;
pub mod source;
mod sources;

pub use module::Fetchers;
