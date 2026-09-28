# 0010. Workspaces: the `Ctx` launcher capability and where its data lives

Status: proposed · Raised by Dev A · Needs sign-off: Dev A, Dev B (CLAUDE.md §4, changes `core`)

## Context

Spawning a real OS process is `core`'s one sanctioned exception to "no raw I/O in a module"
(§2: "workspace launching spawns real OS processes... the one exception"), but `Ctx` (§5)
has no capability for it. Every other side effect a module can have goes through a narrow,
trait-backed wrapper — `ctx.store`, `ctx.http`, `ctx.bus`, `ctx.queue` (ADR 0001) — precisely
so a module never calls the raw primitive itself (§12 rule 4) and a fake can stand in for
tests (§12 rule 13). Launching `launch.sh`/`cleanup.sh` needs the same treatment: a
`Launcher` on `Ctx`, backed by a `LaunchBackend` trait `core` defines and the daemon
implements.

Separately, `$SWE_HOME/workspaces/<name>/` is not under `data/<module>/` — §7 lists it as a
sibling of `data/`, outside every module's reach through `ctx.store` (`NamespacedStore` is
scoped to exactly `data/<namespace>/`). This ADR proposes moving it, same shape and reason
as ADR 0008's `collections/` move.

**This ADR covers only Dev A's side: the launcher, its `core` interface, and the storage
move.** `crates/modules/workspaces` is Dev B's crate (CLAUDE.md §13); the state machine and
`workspace.toml` parsing are pure logic that need no `Ctx` and are entirely Dev B's design to
make — see "Dev B's side" below. No `core` or daemon code lands with this ADR; it is proposed
here for sign-off, same as ADR 0009 was before its implementation.

## Decision — Dev A's side

### 1. The capability

```rust
// core
#[async_trait]
pub trait LaunchBackend: Send + Sync {
    async fn run(&self, step: &LaunchStep, cancel: &CancellationToken) -> Result<StepOutcome>;
}

#[derive(Clone)]
pub struct Launcher(Arc<dyn LaunchBackend>);
```

`Ctx` gains one field, `pub launcher: Launcher`, alongside `store`/`http`/`bus`/`queue`. The
daemon implements `LaunchBackend` for real (§3 below); module tests use `FakeLauncher` (§6).
A module never calls `std::process::Command`, `setsid`, or any spawning primitive directly —
same rule as every other capability (§12 rule 4).

### 2. Interface

```rust
pub enum Step { Launch, Cleanup }

pub enum SpawnMode {
    Detached,
    Supervised { timeout: Duration },
}

pub struct LaunchStep {
    pub workspace_id: String,   // becomes SWE_WORKSPACE_ID
    pub workspace_dir: String,  // namespace-relative, e.g. "deep-work" — see §5 below
    pub step: Step,
    pub mode: SpawnMode,
    pub session_id: String,     // becomes SWE_SESSION_ID; caller-generated so it can
                                 // correlate its own events to this run
    pub user_env: Vec<(String, String)>,  // workspace.toml's [env], nothing else
}

pub struct StepOutcome {
    pub exit_code: Option<i32>,  // None for Detached (not waited on) or killed-by-signal
    pub timed_out: bool,
    pub log_path: String,        // relative to $SWE_HOME — see "Logs" below
}
```

`LaunchStep` carries a *logical* step, not a `PathBuf`. The `LaunchBackend` implementation
resolves `launch.sh` vs `launch.ps1` by platform itself. **`Ctx` gets no `platform` field —
it doesn't need one.** `SWE_PLATFORM` is a value the launcher injects into the child's
environment; nothing upstream of the launcher needs to branch on platform, so nothing
upstream needs to carry it.

The module supplies exactly three things: `workspace_id`, `workspace_dir`, and the user's
`[env]` from `workspace.toml`. The launcher injects `SWE_HOME`, `SWE_SOCKET`, `SWE_PLATFORM`
and `SWE_SESSION_ID` itself, from values only the daemon has. **A module-supplied env var
must not override an injected one** — the launcher builds the child's environment
injected-first, then applies `user_env` only for keys not already set, so a
`workspace.toml` that (accidentally or otherwise) declares `SWE_SOCKET = "..."` cannot
redirect a script's daemon connection.

**The `SWE_` prefix lives in one constant** (`const ENV_PREFIX: &str = "SWE_"` or an enum of
the five names), used everywhere a name is built or checked — the override guard above, the
env table in §10.1, and the launcher's own injection code. These names are a permanent
script ABI (§10.1: "whatever we pass on day one, we are stuck with"); a rename is coming
(CLAUDE.md's header: "Binary name `swe` and crate prefix `swe-` are placeholders. Rename
once, early, everywhere") and having one constant is what makes that a one-line change
instead of a grep-and-pray.

### 3. Behaviour

**Detached** (`setsid` on Unix, `DETACHED_PROCESS` on Windows, per §10.2): stdin/stdout/
stderr to null (same as `crates/cli/src/autostart.rs`'s existing daemon-autostart spawn —
reuse that pattern, don't reinvent it), own process group so the launcher's own signals
never reach it, and `run` returns as soon as the process is spawned — it does not wait for
an exit code, matching "no handle retained" (§10.2). **Reaping, so a launched process never
zombies while the daemon keeps running:** immediately after spawning, the backend
`tokio::spawn`s a detached task that does nothing but `child.wait().await` and discards the
result. That task is internal — it is not the "handle" §10.2 forbids retaining; nothing
external can query or cancel it, it exists purely so the kernel's exit status gets collected
the moment the process exits rather than sitting as a zombie until the daemon itself exits.

**Supervised**: spawned into its own process group (same mechanism as detached), then
`tokio::select!` over three futures — the child exiting, a `tokio::time::sleep(timeout)`,
and `cancel.cancelled()`. On timeout **or** on cancellation, kill the whole process group
(negative PID signal on Unix, so a script's own children — e.g. a build tool's compiler
subprocesses — die with it, not just the immediate child) and set `timed_out: true` only for
the timeout case (a cancelled step is not "timed out", it is cancelled — callers distinguish
via `cancel.is_cancelled()` on return, same pattern `ctx.retry_with_backoff` already uses).
Returns `StepOutcome { exit_code, timed_out, log_path }`.

**Logs.** The launcher captures the child's stdout/stderr and writes them to `logs/`
itself — a module never gets write access to `logs/`; it only ever receives `log_path` back.
`log_path` is relative to `$SWE_HOME`, the same shape `docs/protocol.md`'s `workspace_dirty`
detail already documents (`"log":"logs/deep-work-01JD2T.log"`). Naming note for whoever
wires this up: protocol.md's JSON key is `log`; this ADR's Rust field is `log_path` for
clarity in code, and a module surfacing it on the wire (in a `workspace_dirty` detail, or
`workspaces.status`) puts that string under the `log` key to match. (The already-pushed
`workspaces-state-machine` branch's `Dirty` variant also calls its field `log_path` — that
branch is untouched by this ADR, per "Dev B's side" below, but the same rename-at-the-wire
point applies whenever it is wired to a real `workspace_dirty` response.)

**Scripts run through their interpreter** — `sh launch.sh` (or the platform's `/bin/sh`
equivalent), `powershell -File launch.ps1` — never via the executable bit. An atomic write
(§7.1: temp file + `fsync` + `rename`) does not reliably preserve `chmod +x` across every
filesystem, and a workspace's scripts are typically hand-edited in place, not delivered by
the daemon's own atomic-write path anyway — but the launcher must not silently no-op because
a user forgot `chmod +x` after editing. Invoking through the interpreter removes the
question entirely.

### 4. Capability scoping

A module gets a populated, usable `Launcher` only if its `Manifest.capabilities` (the field
already exists — reused, not invented, per `crates/core/src/manifest.rs`) declares
`"process"`. The daemon constructs `Ctx.launcher` as a no-op/erroring stub (`unavailable`)
for any module that didn't declare it, the same "fails closed" pattern `ctx.http` already
uses before the HTTP gateway exists (`crates/daemon/src/lib.rs` doc comment: "`ctx.http`
fails closed").

**Which scripts it may execute: only `launch.sh`/`cleanup.sh` (or `.ps1`) inside its own
`data/workspaces/<workspace_dir>/`, never an arbitrary path — including one named in
`workspace.toml`.** This is not a policy bolted on top; it is what `LaunchStep`'s shape
above already forces, since the type carries a `Step` enum (`Launch` | `Cleanup`) and a
`workspace_dir` string, never a `PathBuf` or arbitrary string naming a script. The backend
resolves the real path as `$SWE_HOME/data/workspaces/<workspace_dir>/{launch,cleanup}.{sh,ps1}`
after validating `workspace_dir` with `swe_core::store::validate_path` (already rejects `..`,
absolute paths, empty segments — the same check `NamespacedStore` applies to every other
module's paths, reused rather than reinvented). A module cannot ask the launcher to run
"anything named in workspace.toml" because the interface gives it no field to name anything
with. This is the safer of the two choices ADR 0010's task asked to decide between, and it
costs nothing: §10.1's script ABI has never included a mechanism for extra script paths, so
there is nothing to design in order to disallow it.

### 5. Storage move

`workspaces/<name>/` becomes `data/workspaces/<name>/` — same reason and shape as ADR
0008's `collections/` move: a module's own storage must be reachable through `ctx.store`,
and `NamespacedStore` only ever sees `data/<namespace>/`.

**Edits made in CLAUDE.md alongside this proposal** (same as ADR 0008's and ADR 0009's own
storage/`Ctx` changes: written to reflect the design while it is proposed, not held back
until sign-off lands — the ADR's status line, not CLAUDE.md's wording, is what tracks
whether it is actually agreed):

- §7, the layout listing: `workspaces/<name>/` → `data/workspaces/<name>/`, annotated
  `(ADR 0010)`.
- §10.1, the Script ABI's opening line, same path change, plus a new bullet stating scripts
  run through their interpreter rather than the executable bit (§3 above).
- §5's `Ctx` struct gains the `launcher` field (§1 above), and its "Forbidden in `Ctx`" list
  gains a line naming what the launcher deliberately cannot expose (§2/§4 above).

**This still needs Dev B's explicit agreement, separately from the rest of this ADR** — it
changes the data location of a module Dev B owns (§13), even though Dev A is the one
proposing it for the same structural reason (`Ctx` cannot reach it otherwise) that forced
ADR 0008's equivalent move for `records`. If Dev B wants a different path, that is a CLAUDE.md
correction, not a revert of code — nothing outside this ADR and CLAUDE.md's wording exists
yet for it to disturb.

### 6. `FakeLauncher`, for Dev B's module tests

```rust
// core::testing, alongside MemBackend/StubQueue/StubHttp
pub struct FakeLauncher {
    pub outcomes: Mutex<VecDeque<Result<StepOutcome>>>,  // consumed in order, one per run()
    pub received: Mutex<Vec<LaunchStep>>,                 // every step it was asked to run
}
```

Spawns nothing. `run` pops the next scripted outcome (or panics with a clear message if none
was scripted — a test that calls it more times than it scripted is a bug in the test, not a
silent pass) and records the step it was given. `TestEnv::new` wires it in behind
`Ctx.launcher` the same way `StubQueue`/`StubHttp` are wired today.

Example — driving `ready → launching → dirty` on a scripted timeout, so Dev B can see the
shape without waiting on a real ADR 0010 implementation:

```rust
let env = TestEnv::new("workspaces");
env.launcher.outcomes.lock().unwrap().push_back(Ok(StepOutcome {
    exit_code: None, timed_out: true, log_path: "logs/deep-work.log".into(),
}));
// module's own `activate` handler calls ctx.launcher.run(...), sees timed_out: true,
// and is the one that decides this means `step_failed` — the launcher never decides that.
```

### 7. Platforms

Linux and macOS first — both are `cfg(unix)`, and `crates/cli/src/autostart.rs` already
proves the `setsid`/process-group pattern works across both (`swe_proto::paths` already
branches on `target_os = "macos"` for socket/home paths, so this is a well-trodden seam).

**Windows — sketched, not implemented:** `DETACHED_PROCESS` (a `CREATE_NEW_PROCESS_GROUP` /
`CREATE_NO_WINDOW` combination via `std::os::windows::process::CommandExt::creation_flags`)
for detached spawns; a **Job Object** (`CreateJobObject` +
`JOBOBJECT_EXTENDED_LIMIT_INFORMATION` with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) in place of
a process-group kill for supervised timeout/cancel, since Windows has no `killpg` — a Job
Object is the standard substitute and kills every process it was assigned, transitively.
Left out of this change entirely; `docs/protocol.md`'s own transport table already defers
Windows to M6, and `LaunchBackend` is a trait specifically so a `WindowsLaunchBackend` can
arrive later without touching `core`'s interface again.

### 8. Atomicity and failure

Process side effects cannot be rolled back (§7.1: "A launched editor cannot be un-launched.
That is exactly why workspaces have the `dirty` state — where atomicity is impossible, we
make the partial state explicit and block on it instead of pretending"). Nothing here
changes that; the launcher's job ends at reporting what happened.

- A failed `Supervised` step (nonzero exit or `timed_out`) is **data returned from `run`**,
  not an `Err`. The launcher does not know what a bad exit code means for a given
  workspace — only the calling module does.
- **Flagging the workspace `dirty` is the module's job, not the launcher's.** The module's
  `workspaces.activate` handler inspects the `StepOutcome`, and if it indicates failure,
  calls the state machine's `step_failed` (Dev B's code) and returns an `Err` from its own
  `handle`. Because `workspaces.activate` is a queued op, that `Err` is exactly what makes
  the *queue task* fail (§11.1) — the launcher itself never touches the queue or the
  module's state; it has no reason to know either exists.
- A launcher-level `Err` (not a `StepOutcome`) is reserved for the backend genuinely being
  unable to attempt the step at all — the script file is missing, unreadable, or the
  platform's spawn syscall itself failed. That is `unavailable`, the same code `ctx.http`
  already uses for "a dependency... failed" (`docs/protocol.md`'s error table).

### 9. Shutdown, crashes, and session ids

**Graceful `core.shutdown`.** A running queued task's `ctx.cancel` is already a *child* of
the daemon's shutdown token (`crates/daemon/src/queue/mod.rs`: `self.shutdown.child_token()`
— existing code, unchanged by this ADR), so shutdown cancels an in-flight `Supervised` step
through the exact same path as a manual cancel: `run`'s `tokio::select!` wakes on
`cancel.cancelled()` immediately (it is a notification, not a poll interval) and issues the
process-group kill. `SIGKILL` cannot be blocked or ignored by the child, so this completes
in effectively zero time — well inside the daemon's existing `SHUTDOWN_GRACE` (5s,
`crates/daemon/src/lib.rs`) regardless of what the script itself was doing. Nothing new is
needed here beyond what §3's "Supervised" behaviour and the existing shutdown-token wiring
already give for free.

**A `Detached` step is unaffected by shutdown, deliberately** — its reaping task (§3) is a
plain `tokio::spawn`, never added to the daemon's `TaskTracker`/shutdown-drain set, so
`wait()`'s drain never waits for it and shutdown is never blocked by "is the terminal still
open." When the daemon process itself exits — gracefully or by crashing — any child it
spawned that is still running (detached, or a supervised one that somehow escaped the kill)
is reparented to init by the kernel, exactly as it would be for any orphaned Unix process;
init reaps it on exit, so there is no zombie leak in the crash case either. The in-process
reaping task's only job is covering the window *while the daemon is alive*; the OS covers
the window after.

**A daemon crash** (no graceful shutdown, so no cancellation ever fires) can leave a
`Supervised` step's workspace persisted in `Launching` — the state that, in the normal
running case, only exists for the instant between "the queued task called `run`" and "the
task recorded the outcome." On restart, seeing `Launching` in a freshly-loaded workspace
state is itself the signal that the previous run never got to record anything, since a live
daemon transitions out of `Launching` before its queued task returns. **Reconciling this is
the module's job, in its own `init`** (Dev B's code, `crates/modules/workspaces` — not
implemented by this ADR, only specified as a contract here): any workspace found `Launching`
at `init` is treated as `Dirty` with a reason naming the restart, the same pattern
`records`' `init` already uses for one-time startup work (seed-if-missing) and ADR 0004's
"[triggers are] reconciled with `Module::triggers()` on every start." **The daemon does not
attempt to locate and kill whatever process that step spawned** — it may have already
exited, may still be running, or its PID may since have been reused by an unrelated process
(the classic Unix PID-reuse hazard), so guessing is actively unsafe. `Dirty` is the correct
and sufficient response: it blocks re-activation and surfaces the ambiguity to the user
(§10.3), who can inspect the log and decide, same as any other dirty reason.

**`SWE_SESSION_ID`.** The calling module generates it — a ULID (same shape as `TaskId`,
`crates/core/src/ids.rs`), fresh **per launch attempt**, not reused across a later `cleanup`
or `force_relaunch` for the same workspace (CLAUDE.md §10.1: "unique per launch"; a cleanup
run is a different attempt with its own log window). No new `core` type is needed for this —
a module may depend on the `ulid` crate directly, the same ordinary third-party dependency
`core` itself already uses, and pass `Ulid::new().to_string()` in as `LaunchStep.session_id`.
`Launcher`/`LaunchBackend` only ever receive the string; they do not generate or validate it.

### 10. Tests Dev A will write against the real `LaunchBackend`

- A `Supervised` step whose script spawns a grandchild that sleeps past the timeout: the
  timeout kills the **grandchild** too (proves process-group kill, not just the immediate
  child).
- A `Detached` step's process is still running after the `Launcher`/`LaunchBackend` value
  that spawned it is dropped (proves "no handle retained" doesn't mean "process dies with
  the handle").
- An env var the script echoes back matches what `LaunchStep.user_env` set, and a
  `user_env` entry named `SWE_SOCKET` does **not** override the injected one (proves the
  override guard).
- A script's stdout appears verbatim in the file at the returned `log_path`.
- Cancelling `ctx.cancel` mid-`Supervised` kills the process group exactly as a timeout
  would, and does so well within `SHUTDOWN_GRACE` (proves §9's shutdown claim, not just the
  timeout path).

## Dev B's side

Everything in `crates/modules/workspaces` — the `ready`/`launching`/`active`/`dirty` state
machine (CLAUDE.md §10.3) and `workspace.toml` parsing (§10.1) — is Dev B's design and code
(CLAUDE.md §13: Dev A must not touch `modules/*`). Both are pure logic: plain functions over
plain data, no `Ctx`, no I/O (§12 rule 10), so **none of it needs this ADR to proceed.**

The branch `workspaces-state-machine` (off `workspaces-m3`) already has one possible
implementation of both pieces, offered as a starting point only. **It is not part of this
ADR, is not merged, and Dev A will not extend it or open a PR for it.** Dev B decides
whether to take it over, rework it, or discard it entirely and start fresh — it carries no
authority either way.

The only thing that waits on this ADR is any call from `crates/modules/workspaces` into
`ctx.launcher` — i.e., the real body of `workspaces.activate`/`cleanup`/`force_relaunch`.
Everything else (the state machine, the parser, and their tests, written against
`FakeLauncher` per §6 above) can proceed now.

## Not done here

- No `core` or daemon code lands with this ADR — the launcher, `FakeLauncher`, and the
  storage move are all proposed above and reflected in CLAUDE.md's wording, but no Rust is
  implemented until Dev A and Dev B both sign off.
- Windows `LaunchBackend` (§7) — sketched, not built.
- Locked-in mode, the separate privileged binary (§10.4) — unrelated, still out of scope
  per §1.7.
