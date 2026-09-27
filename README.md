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

Tracking LeetCode (the collection is built in; any other tracker is a TOML file, see ADR 0008):

```sh
swe records add leetcode two-sum --title "Two Sum" --difficulty easy
swe records list leetcode --status todo
swe records complete leetcode/two-sum
swe records --help
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
| M1 — records vertical slice | Done. `records` module with the LeetCode collection and `swe records` add, list, complete and filter (Dev B, ADRs 0007–0008); a completed item survives a restart. |
| M2 — queue and scheduler | In progress (Dev A). Lanes, fallbacks and scheduler landed; see ADRs 0003–0006. |
| M3 — workspaces | Not started. |
| M4 — index rebuild + heatmap | Not started. |
| M5 — TUI usable | Not started. |
| M6 — fetchers, notify, calendar | Not started. |
