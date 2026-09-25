# PlatformDashboard

A local-first software engineering platform: workspaces, a task queue, a scheduler, trackers,
fetchers, a calendar and notifications, on one machine with no server and no account.

`CLAUDE.md` is the source of truth for the design. `docs/protocol.md` is the daemon/client
contract. `docs/decisions/` records design choices the two leave open, one numbered file each.

## Quickstart

```sh
cargo build
cargo run -q -- ping        # starts the daemon if needed: "pong (daemon up 0s)"
cargo run -q -- manifest    # lanes and registered modules
cargo run -q -- --help      # every command
cargo run -q -- shutdown
```

`swe daemon` runs the daemon in the foreground with its logs; other `swe` commands in a second
terminal talk to it. `swe mockd --fixtures crates/mockd/fixtures --socket /tmp/mock.sock` runs
the mock daemon, and `--socket /tmp/mock.sock` points any command at it.

Before pushing, the same checks CI runs:

```sh
python3 .dev/check-deps.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Status

Milestones are defined in `CLAUDE.md` §14.

| Milestone | Status |
|---|---|
| M0 — skeleton | Done. Workspace, `core`, `proto`, daemon, CI (Dev A); CLI round-trip with auto-start (Dev B, ADR 0007). |
| M1 — records vertical slice | Not started. Store and mock daemon exist; `records` module and its CLI commands next. |
| M2 — queue and scheduler | In progress (Dev A). Lanes, fallbacks and scheduler landed; see ADRs 0003–0006. |
| M3 — workspaces | Not started. |
| M4 — index rebuild + heatmap | Not started. |
| M5 — TUI usable | Not started. |
| M6 — fetchers, notify, calendar | Not started. |
