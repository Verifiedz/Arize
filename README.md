# Shimmer

## What it is

Shimmer is a local-first command center for software engineers. It keeps the things you track
(LeetCode problems, job applications, anything with fields) on your own machine, in plain files
you can read, edit, copy to another computer or commit to git. There is no server, no account
and no network requirement.

Today Shimmer is a background daemon plus a command-line client. You use it to define trackers
and add, list, filter, update and complete records in them.

The command is `swe` for now. It will be renamed to `shimmer` everywhere at once; until then,
every example here uses `swe`.

## Overview

Shimmer is built as a few general mechanisms rather than one feature per tracker. A LeetCode
tracker is not code: it is a short TOML file describing a collection. The mechanisms in the
code today:

| Mechanism | What it does |
|---|---|
| **records** | Typed collections. Each collection is a TOML file that declares its fields (`string`, `enum`, `date`, …); each record is its own TOML file. Handles add, list, filter, update, complete and remove. LeetCode ships built in. |
| **queue** | The single pipeline every unit of background work runs through. Work is split into lanes, each with its own concurrency limit. |
| **scheduler** | Owns *when* work happens: recurring and one-shot triggers that enqueue tasks on the queue. It never runs anything itself. |
| **event bus** | How the pieces react to each other. Every change is published as an event (`records.item.completed`, …) and appended to a log. Modules never call each other directly. |

`records` is a module. The queue, scheduler and event bus are core services inside the daemon.
The full design, including the mechanisms that aren't built yet, is in
[`CLAUDE.md`](CLAUDE.md) §1.1.

## How it works

One long-lived **daemon** owns all state. Every other program is a thin client that talks to
it over a Unix domain socket:

```
swe (CLI) ──→ Unix socket ──→ daemon ──→ files in $SWE_HOME ──→ SQLite index (derived)
```

- **The daemon is the only writer.** It hosts the modules, runs the queue and scheduler, and is
  the only process that changes your data.
- **Clients hold no logic.** The CLI sends a request such as `records.list`, then formats the
  reply. It finds out which modules and operations exist by asking the daemon (`swe manifest`),
  so nothing about a module is hardcoded into the client.
- **You never start the daemon yourself.** The first `swe` command starts it in the background
  if it isn't running, the same way `tmux` or `ssh-agent` do.
- **The protocol is newline-delimited JSON**, with requests and responses plus a stream of
  events a client can subscribe to. You can talk to the daemon by hand with `nc -U`. The
  contract is in [`docs/protocol.md`](docs/protocol.md).

**Your files are the truth.** Everything durable is a human-readable file under `$SWE_HOME`:

```
$SWE_HOME/
  config.toml                                Settings
  data/records/collections/<collection>.toml One file per collection definition
  data/records/items/<collection>/<id>.toml  One file per record
  events/YYYY-MM-DD.jsonl                    Append-only log of every event
  index.sqlite                               Derived from the files; safe to delete
  logs/daemon.log.<date>                     Daemon logs, rotated daily at UTC midnight; oldest
                                              deleted past 8 files. A leftover undated
                                              logs/daemon.log predates this and is safe to delete.
```

`$SWE_HOME` defaults to `~/.local/share/swe` on Linux and `~/Library/Application Support/swe` on
macOS. Set the `SWE_HOME` environment variable to use another folder. The SQLite index exists
only to make queries fast: delete it and the daemon rebuilds it from the event log on its next
start. Writes are atomic, so a crash leaves either the old file or the new one, never half of
one. The folder ships with a `.gitignore` for the derived and machine-local parts, so you can
commit it as-is.

## Running it

You need Rust (installed with [rustup](https://rustup.rs)) and git, on Linux or macOS. Windows
isn't supported yet. The repository pins the stable toolchain, so rustup fetches the right
version on the first build.

```sh
git clone https://github.com/Verifiedz/Shimmer.git
cd Shimmer
cargo build
cargo run -q -- ping          # starts the daemon if needed, then: "pong (daemon up 0s)"
```

`cargo run -q --` is the `swe` command, and it only works from inside the repository. To type
`swe` from any folder, run `.dev/install-dev-launcher.sh` once (see
[CONTRIBUTING.md](CONTRIBUTING.md#the-dev-launcher)). The examples below assume you have.

### Track records

```sh
swe records collections                  # list collections and their fields
swe records add leetcode two-sum --title "Two Sum" --difficulty easy
swe records list leetcode --status todo  # --FIELD VALUE filters on an exact value
swe records complete leetcode/two-sum    # mark done; stamps last_solved with today's date
swe records get leetcode/two-sum
swe records update leetcode/two-sum --url https://leetcode.com/problems/two-sum/
swe records remove leetcode/two-sum
```

`list` also takes `--limit N` and `--offset N`, and `update` takes `--unset FIELD`. You can
write a record as `leetcode/two-sum` or `leetcode two-sum`. Run `swe records --help` for the
details.

### Add your own tracker

A new tracker is a TOML file in `$SWE_HOME/data/records/collections/`. For example,
`jobs.toml`:

```toml
[collection]
id = "jobs"
label = "Job applications"
stamp_on_complete = "applied_on"   # `complete` sets this date field to today

[[field]]
name = "company"
type = "string"
required = true

[[field]]
name = "stage"
type = "enum"
values = ["saved", "applied", "interview", "offer"]

[[field]]
name = "applied_on"
type = "date"
```

It's usable straight away, with no restart:

```sh
swe records add jobs acme --company Acme --stage saved
swe records list jobs --stage saved
```

The built-in `leetcode.toml` sits in the same folder and is a good reference. You can edit it
too.

### Other commands

```sh
swe manifest              # registered modules, their operations and lanes
swe call OP '{...}'       # send any operation with JSON params, e.g. swe call queue.list
swe shutdown              # stop the daemon
swe daemon                # run the daemon in the foreground with its logs (stop any running one first)
swe --help                # every command
```

Add `--json` to any command to see the daemon's raw reply, and `--socket PATH` to talk to a
daemon on a socket other than the default.

### Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for setup, the checks to run before pushing, and the
branch workflow. [`CLAUDE.md`](CLAUDE.md) is the source of truth for the design, and
[`docs/decisions/`](docs/decisions/) holds one numbered file per design decision.
