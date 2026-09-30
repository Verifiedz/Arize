# Shimmer

A local-first command center for software engineers: trackers, workspaces, scheduler; no
account, your data in plain files.

Shimmer runs on your machine as one small background daemon with a CLI in front of it. What you
track (LeetCode problems today; job applications, OSS repos, anything with fields tomorrow) lives
as TOML files under one data folder you can copy between machines or commit to git. Nothing
needs a server, a network connection or a sign-up.

The command is `swe` for now. It will be renamed to `shimmer` everywhere at once; until then,
every example here uses `swe`.

## Status

Early and moving fast. Milestones are defined in `CLAUDE.md` §14.

**Works today:** the daemon (started automatically by the first command you run), the `swe` CLI,
and record tracking: add, list, filter, update and complete LeetCode problems, with your data
surviving restarts. Under the hood the task queue and scheduler run, and the mock daemon lets
client work proceed without a real one.

**Coming:** workspaces (one command to launch your editor, terminals and browser for a task),
scheduled reminders, activity heatmaps, a terminal dashboard (TUI), and fetchers, notifications
and a calendar that tie them together.

| Milestone | Status |
|---|---|
| M0 — skeleton | Done. Workspace, `core`, `proto`, daemon, CI; CLI round-trip with daemon auto-start (ADR 0007). |
| M1 — records vertical slice | Done. `records` module with the LeetCode collection; `swe records` add, list, complete and filter (ADRs 0007–0008). A completed item survives a restart. |
| M2 — queue and scheduler | In progress. Lanes, fallbacks and scheduler landed (ADRs 0003–0006); queue reordering is proposed (ADR 0006). |
| M3 — workspaces | Designed (ADR 0010, proposed), not built. |
| M4 — index rebuild + heatmap | Not started. |
| M5 — TUI usable | Not started. |
| M6 — fetchers, notify, calendar | Not started. |

## Quickstart

You need Rust (via [rustup](https://rustup.rs)) and git, on Linux or macOS. Windows isn't
supported yet.

```sh
git clone https://github.com/Verifiedz/Shimmer.git
cd Shimmer
cargo build
cargo run -q -- ping        # starts the daemon if needed: "pong (daemon up 0s)"
```

Track a LeetCode problem (the collection is built in; any other tracker is a TOML file, see
ADR 0008):

```sh
cargo run -q -- records add leetcode two-sum --title "Two Sum" --difficulty easy
cargo run -q -- records list leetcode --status todo
cargo run -q -- records complete leetcode/two-sum
cargo run -q -- records --help
```

`cargo run -q --` is the `swe` command. To type `swe` from any folder instead, install the dev
launcher: `.dev/install-dev-launcher.sh` (see [CONTRIBUTING.md](CONTRIBUTING.md#the-dev-launcher)).

Other useful commands:

```sh
swe manifest     # registered modules, their ops and lanes
swe --help       # every command
swe daemon       # run the daemon in the foreground with its logs (stop any running one first)
swe shutdown     # stop the daemon
```

Your data lives in `~/.local/share/swe` on Linux and `~/Library/Application Support/swe` on
macOS. Set `SWE_HOME` to use another folder.

## Contributing and design

- [CONTRIBUTING.md](CONTRIBUTING.md): setup, the checks to run before pushing, branch workflow,
  ADRs, where to start reading.
- `CLAUDE.md`: the source of truth for the design.
- `docs/protocol.md`: the daemon/client contract.
- `docs/decisions/`: design decisions the two leave open, one numbered file each.
