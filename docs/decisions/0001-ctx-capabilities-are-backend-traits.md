# 0001. `Ctx` capabilities are thin wrappers over backend traits defined in `core`

Status: accepted (M0) · Needs sign-off: Dev A, Dev B (CLAUDE.md §4)

> **Renamed since:** `swe`, `swe-*` and `SWE_*` in this ADR are now `shimmer`, `shimmer-*` and
> `SHIMMER_*` (ADR 0011). The text below is kept as written.

## Context

CLAUDE.md §5 fixes `Ctx` as a struct of concrete `NamespacedStore`, `HttpGateway`,
`Emitter`, `QueueHandle` and `Clock`, and says `core` depends on nothing internal. But the
real store lives in `crates/store` and the queue in `crates/daemon`, and modules must be
testable against an in-memory `Ctx` (§12 rule 13).

## Decision

Keep the concrete types from §5 in `core`, each wrapping a small trait object the daemon
implements: `StoreBackend`, `HttpBackend`, `EventSink`, `TaskSubmitter`. Modules only ever
see the wrapper, so the "forbidden in Ctx" list (§5) still holds: no path, no raw client.

Consequences worth knowing:

* The wrappers enforce rules at the boundary, not the daemon: store paths cannot escape
  the namespace (`validate_path`); a module can only emit topics under its own id
  (`validate_topic`); `QueueHandle::enqueue` always stamps `Origin::Module` and
  `Priority::Normal`, so a module cannot impersonate the scheduler or request a promotion.
* `Ctx` has one private field beyond §5, `progress`. It is a reporting callback the queue
  installs per task (`Ctx::for_task`), not a capability.
* `NamespacedStore` is synchronous. Modules do small local file I/O; the transaction
  closure is a plain function (§12 rule 10). Revisit if a backend ever becomes remote.
* `swe-core` exposes in-memory fakes behind the `testing` feature (`testing::TestEnv`).
* `retry_with_backoff` takes `FnMut(attempt) -> Future` and retries only
  `ErrorCode::Unavailable`; no jitter, so tests are deterministic.

## Alternatives rejected

Generic `Ctx<S, H, ...>`: infects every module signature. `Arc<dyn Ctx>`: contradicts the
fixed struct in §5.
