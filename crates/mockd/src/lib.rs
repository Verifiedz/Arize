//! `shimmer-mockd`: the mock daemon behind `shimmer mockd`.
//!
//! It speaks `docs/protocol.md` over a Unix socket and answers from fixture files instead of
//! real modules: canned responses, and events replayed on a script. Clients built against it
//! (the TUI first) are then not blocked on a daemon-side op landing.
//!
//! Depends on `core` and `proto` only, like a client. It has no store, no queue and no
//! scheduler, so it cannot grow behaviour: anything it does is stated in a fixture.

mod fixtures;
mod options;
mod server;

pub use fixtures::{Fixtures, Lookup, Rule, TimedEvent};
pub use options::{Command, Options, USAGE};
pub use server::{Mockd, ShutdownHandle};
