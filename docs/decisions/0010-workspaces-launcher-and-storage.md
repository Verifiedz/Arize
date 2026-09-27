# 0010. Workspaces: the `Ctx` launcher capability and where `workspace.toml` lives

Status: proposed · Raised by Dev A · Needs sign-off: Dev A, Dev B (CLAUDE.md §4, changes `core`)

## Context

§10 fixes the shape of workspaces (script ABI, spawn modes, the `ready`/`launching`/
`active`/`dirty` state machine) but leaves two things unresolved that block writing the
actual launcher, though not the pure logic around it:

**1. Spawning a real OS process is `core`'s one sanctioned exception to "no raw I/O in a
module" (§2: "workspace launching spawns real OS processes... the one exception"), but
`Ctx` (§5) has no capability for it.** Every other side effect a module can have goes
through `ctx.store`, `ctx.http`, `ctx.bus` or `ctx.queue` — each a narrow trait-backed
wrapper (ADR 0001). Launching `launch.sh` needs the same treatment: a `Launcher` capability
on `Ctx`, backed by a trait the daemon implements, so a module never calls
`std::process::Command` directly (§12 rule 4) and the fake used in module tests (§12 rule
13) can record what would have been spawned instead of spawning it.

**2. `$SWE_HOME/workspaces/<name>/` is not under `data/<module>/` (§7's layout lists it as a
sibling of `data/`, not inside it).** Every other module's storage is `ctx.store`, scoped by
`NamespacedStore` to exactly `data/<namespace>/` — it cannot read `workspaces/` at all. So
either `workspaces/` moves under `data/workspaces/` (a §7 change, same shape as ADR 0008's
`collections/` move and for the identical reason), or reading it needs a new mechanism.
Moving it is the smaller change and keeps the "one storage model" property, but scripts
under `workspaces/<name>/launch.sh` are the *permanent public script ABI* (§10.1: "whatever
we pass on day one, we are stuck with") — moving the directory is not a script-ABI break
(scripts don't reference their own path), so there is no reason to prefer the exception.
**Proposed: move it to `data/workspaces/<name>/`, matching every other module.**

Neither question blocks the state machine or `workspace.toml` parsing below — both are pure
functions over data already in memory, with no `Ctx` and no filesystem access. They are
implemented now, on `workspaces-state-machine` (off `workspaces-m3`). Only actually reading
a workspace directory and spawning `launch.sh`/`cleanup.sh` waits on this ADR.

## Decision (the launcher — proposed, not yet implemented)

```rust
// core: a new Ctx capability, alongside http/store/bus/queue.
pub struct Launcher(Arc<dyn LaunchBackend>);

pub enum SpawnMode {
    /// Outlives the daemon. No handle retained (§10.2).
    Detached,
    /// Handle retained, timeout enforced, whole process group killed on timeout,
    /// stdout/stderr captured to `logs/` (§10.2).
    Supervised { timeout: Duration },
}

pub struct LaunchStep {
    pub name: String,
    pub script: PathBuf,       // launch.sh / launch.ps1, resolved by ctx.platform
    pub env: Vec<(String, String)>,  // the §10.1 table, plus workspace.toml's own [env]
    pub mode: SpawnMode,
}

impl Launcher {
    pub async fn run(&self, step: &LaunchStep) -> Result<StepOutcome>;
}
```

`LaunchBackend` is the trait the daemon implements (real `setsid`/`DETACHED_PROCESS`
spawning) and tests fake (record the step, return a canned outcome) — same pattern as
`StoreBackend`/`HttpBackend` (ADR 0001).

**Open for Dev B**, since `modules/*` is their crate: whether `workspaces` needs any other
new `Ctx` surface (e.g. a way to read the resolved `SWE_PLATFORM`, or whether that is just a
field on `LaunchStep`/`Ctx` computed once at daemon startup like `local_tz`, ADR 0009). Not
deciding it here — flagging that it is the same shape of question ADR 0009 already answered
for timezone, and should probably be answered the same way (resolved once, carried on `Ctx`,
never re-detected per call).

## Decided now (pure logic, implemented on `workspaces-state-machine`)

**State machine** — exactly §10.3's diagram, as an enum and transition functions with no
I/O:

```rust
pub enum WorkspaceState {
    Ready,
    Launching,
    Active,
    Dirty { reason: String, failed_step: String, log_path: String },
}
```

Transitions (`activate`, `step_failed`, `cleanup_succeeded`, `force_relaunch`, `reset`) are
plain functions `WorkspaceState -> Result<WorkspaceState>` a test calls with no runtime
(§12 rule 10). `activate` on `Dirty` returns `workspace_dirty` rather than a new state — it
is a refusal, not a transition (§10.3: "an error, not a warning the user can click through by
accident"). `force_relaunch` is the only way out of `Dirty` besides a successful cleanup, and
is modeled as its own transition, distinct from `activate`, so the audit trail (`emits
workspaces.session.forced`) has something distinct to hang off later.

**`workspace.toml` schema.** One launch step per workspace, matching §10.1's file layout
literally: exactly one `launch.sh`/`launch.ps1` and one `cleanup.sh`/`cleanup.ps1` per
workspace, not a list of per-line steps within a script (the daemon runs a script as one
process; it cannot see "steps" inside a shell file the user wrote by hand). "Every launch
step declares one [spawn mode]" (§10.2) is satisfied by workspace.toml declaring the mode
for the one step that exists — running `launch.sh`. `cleanup.sh` is always `supervised`
(§10.3 says so explicitly), so it is not declared:

```toml
[workspace]
id = "deep-work"
label = "Deep Work"
mode = "detached"      # or "supervised" — how launch.sh itself is spawned
timeout_s = 30          # required when mode = "supervised"; rejected otherwise

[env]
PROJECT_DIR = "~/code/thing"   # merged with the §10.1 table; workspace.toml wins on collision
```

If a real use case ever needs multiple independently-supervised steps per workspace, that is
a bigger change (probably multiple scripts, not one) and gets its own ADR then — not
speculatively designed now (CLAUDE.md's general-before-specific rule cuts the other way when
there is no second example yet).

Parsing is `parse_manifest(text: &str) -> Result<WorkspaceManifest>`: validates `id` (same
`[a-z0-9][a-z0-9_-]*` rule as a record id, ADR 0008), `mode` is `detached` or `supervised`,
`timeout_s` present iff `mode = "supervised"`. No file I/O in this function — the caller
reads the file.

## Not done here

* Actually reading `workspace.toml` or spawning anything — waits on this ADR's launcher
  decision above.
* The `data/workspaces/` move — proposed above, not yet made; §7 still says `workspaces/` at
  top level until this is signed off, since moving it is a `core`/store-layout change same
  as the launcher.
* Locked-in mode, the separate privileged binary (§10.4) — unrelated, still out of scope
  per §1.7.
