//! Command packs (CLAUDE.md §15.1, ADR 0013): themed aliases that stand for a whole command, so
//! `ikuzo deep-work` runs `shimmer workspaces activate deep-work`. Everything here lives in the
//! client: an alias is rewritten to canonical command words before anything is parsed or sent,
//! and the daemon never sees one (§12 rule 16).
//!
//! - [`pack`]: a pack's `pack.toml`, and every check a pack must pass (ADR 0013 §1, §6, §7).
//! - [`settings`]: the CLI's own `cli.toml`, which records the active pack (ADR 0013 §3).

pub mod pack;
pub mod settings;
