# 0007. CLI command surface and daemon auto-start

Status: proposed (M0) · Raised by Dev B · Needs sign-off: Dev A (touches `crates/app`)

## Context

CLAUDE.md §14 defines M0 as "a `swe` command reaches the daemon and back" without naming the
command, and §2 says the CLI auto-starts the daemon "if the socket is dead, then retries once"
without saying how, given that `cli` may not depend on `daemon` (§3 rule 2). Both are now
answered in code in `crates/cli`; this records the choices so they are not rediscovered.

## Decision

**Commands.** The first commands map one-to-one onto the always-present core ops:
`swe ping` (`core.ping`), `swe manifest` (`core.manifest`), `swe shutdown` (`core.shutdown`).
`swe call OP [PARAMS]` sends any op with a JSON-object `params` and prints the raw `data`, so
every module op is reachable before it has a dedicated command, with no module knowledge in
the client (§9). These are canonical names; pack aliases (§15.1) will rewrite the command word
before parsing and never reach the wire.

**Flags.** `--json` prints the daemon's `data` unformatted, for scripts. `--socket PATH` talks
to that socket instead of `swe_proto::paths::socket_path()`.

**Exit codes.** `0` success; `1` the daemon returned an error, or could not be reached; `2`
bad usage. Errors print to stderr as `swe: <code>: <message>`, plus `detail` when present.
Scripts branch on the exit code and, if they need more, on `code` from `--json` output, never
on the message.

**Auto-start.**

* Triggered only when connecting fails with "nobody listening" (`NotFound` or
  `ConnectionRefused`). A daemon that answers but refuses the handshake is reported, never
  replaced.
* The daemon is the same binary: the CLI runs `std::env::current_exe()` with `daemon`,
  setting `SWE_SOCKET` to the socket it is about to poll so both agree on the path.
* The child gets its own process group (Ctrl-C on the command does not reach it), null
  stdio, and no retained handle beyond checking whether it exited during startup.
* The CLI polls for a completed handshake for up to 10 s. If the child exits first, it tries
  once more (a concurrent `swe` may have won the race to start one), then reports
  `unavailable` and tells the user to run `swe daemon` to see why.
* **`--socket` disables auto-start.** An explicit socket usually means the mock daemon; a real
  daemon must never be started on the mock's path.
* **`swe shutdown` never auto-starts.** With nobody listening it says so and exits `0`.

## Consequences

* The binary dispatches `daemon` and `mockd` itself and hands every other command to
  `swe_cli::run`. That is an edit to `crates/app`, which has no owner in §13; flagged for
  Dev A.
* An auto-started daemon's stderr goes nowhere, so its logs are lost unless it is run in the
  foreground. The daemon should log to `$SWE_HOME/logs/` itself; that is daemon-side work.
* The CLI does not unlink a stale socket, as `docs/protocol.md` suggests clients should; the
  daemon already clears a stale socket when it binds, so doing it in both places buys nothing.
* M0's definition of done is a test: `crates/app/tests/m0.rs` runs the real binary from no
  daemon, pings it, checks the manifest and an error code, and shuts it down.
