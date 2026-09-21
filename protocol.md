# IPC protocol

Version `1`. Change-controlled — see `CLAUDE.md` §4.

This document is the complete contract between the daemon and any client. A client
implemented only against this file must work. If you need to read daemon source to build a
client, this file has a gap; fix the file.

---

## Transport

Unix domain socket, stream mode.

| Platform | Socket path |
|---|---|
| Linux | `$XDG_RUNTIME_DIR/swe/daemon.sock`, falling back to `$SWE_HOME/daemon.sock` |
| macOS | `$SWE_HOME/daemon.sock` |
| Windows | Named pipe `\\.\pipe\swe-daemon` (deferred; not before M6) |

The path is also exported to workspace scripts as `SWE_SOCKET`.

If the socket file exists but connecting returns `ECONNREFUSED`, the daemon died without
cleaning up. Clients should unlink it and start a daemon.

## Framing

Newline-delimited JSON. One complete JSON object per line, UTF-8, terminated by `\n`.
Objects must not contain literal newlines — JSON string escaping handles this.

Chosen for debuggability: a whole session can be driven from a shell.

```
$ nc -U ~/.local/share/swe/daemon.sock
{"v":1,"kind":"hello","client":"manual","client_version":"0.0.0"}
```

Max line length 8 MiB. A longer line is a protocol error and the daemon closes the
connection.

---

## Handshake

The client sends `hello` first, before anything else. The daemon replies `welcome`, or
`error` and closes.

```jsonc
// →
{"v": 1, "kind": "hello", "client": "tui", "client_version": "0.3.1"}

// ←
{"v": 1, "kind": "welcome", "daemon_version": "0.3.0", "session": "01JD2K…"}
```

- `v` is the **protocol** version, an integer, bumped only on breaking changes.
- Daemon rejects an unknown `v` with `unsupported_version` and closes.
- `daemon_version` and `client_version` are informational. Never branch on them.

---

## Message kinds

Client → daemon: `hello`, `request`, `subscribe`, `unsubscribe`.
Daemon → client: `welcome`, `response`, `event`.

### `request` / `response`

`id` is an opaque client-generated string, unique within the connection. The daemon echoes
it. Requests may be pipelined; responses may arrive out of order, so match on `id`.

```jsonc
// →
{"v":1,"kind":"request","id":"r1","op":"records.list",
 "params":{"collection":"leetcode","filter":{"status":"todo"},"limit":50}}

// ← success
{"v":1,"kind":"response","id":"r1","ok":true,
 "data":{"items":[…],"total":137}}

// ← failure
{"v":1,"kind":"response","id":"r1","ok":false,
 "error":{"code":"not_found","message":"no collection 'leetcode'","detail":null}}
```

`data` and `params` are arbitrary JSON, defined per-op. `error.detail` is an optional
object for structured context; clients must tolerate it being absent or unrecognised.

**All or nothing.** An inline request that returns `ok: false` has changed nothing — no
file, no event, no index row. A client may retry it without first checking whether it
partly applied. For queued ops the guarantee is per store transaction, not per task: a
failed task may have committed earlier transactions, and each one it did commit emitted
its own event. Clients learn exactly what happened from the event stream, never by
inference. See `CLAUDE.md` §7.1.

### Inline vs queued ops

Every op declares an `execution` in `core.manifest`:

- **`inline`** — answered synchronously. `data` is the result.
- **`queued`** — the daemon enqueues the work and responds immediately with a task
  handle. The result arrives later as events, not as this response.

```jsonc
// → a queued op
{"v":1,"kind":"request","id":"r2","op":"fetchers.fetch",
 "params":{"source":"hn-whoishiring"}}

// ← accepted, not completed
{"v":1,"kind":"response","id":"r2","ok":true,
 "data":{"queued":true,"task_id":"01JD2N…","lane":"fetchers","position":3}}
```

Clients must read `execution` from the manifest rather than assuming. An op that is inline
today may become queued later without a protocol version bump.

### Queue control on the request frame

Queue behaviour is orthogonal to the op, so it lives on the **frame**, not in `params`.
Ops with `execution: inline` ignore it.

```jsonc
{"v":1,"kind":"request","id":"r3","op":"fetchers.fetch",
 "params":{"source":"linkedin"},
 "queue":{"priority":"override","confirm":false}}
```

| Field | Values | Default |
|---|---|---|
| `priority` | `normal` \| `override` | `normal` |
| `confirm` | `true` \| `false` | `false` |
| `queue_version` | integer, echoed from a prior `confirmation_required` | absent |

`priority: "scheduled"` is not settable by clients. Only the scheduler produces it.

### `subscribe` / `event`

```jsonc
// →
{"v":1,"kind":"subscribe","id":"s1","topics":["records.*","queue.task.failed"]}
// ← acknowledgement
{"v":1,"kind":"response","id":"s1","ok":true,"data":{"subscribed":2}}

// ← pushed at any time thereafter, no id
{"v":1,"kind":"event","event":{
  "id":"01JD2K…","at":"2026-09-16T14:03:11Z","source":"fetchers",
  "topic":"fetchers.fetch.finished",
  "payload":{"source":"hn-whoishiring","new_items":12}}}
```

Topic patterns support a trailing `*` on a segment boundary: `records.*`,
`records.item.*`, or `*` for everything. No other globbing. `unsubscribe` takes the same
`topics` shape.

The event object is byte-identical to the one appended to
`$SWE_HOME/events/YYYY-MM-DD.jsonl`. One definition in `swe-core`.

**Backpressure:** each connection has a bounded event queue (1024). If a client does not
drain it, the daemon drops the oldest and sends
`{"kind":"event","event":{"topic":"core.stream.lagged","payload":{"dropped":N}}}`. A client
receiving this should refetch rather than assume its state is current.

---

## Priority promotion: the confirmation round-trip

Promoting a task ahead of scheduled work is never silent. It is a two-step exchange, with
no server-side pending state — the daemon holds nothing between the two requests.

```jsonc
// → 1. ask to jump the lane
{"v":1,"kind":"request","id":"p1","op":"fetchers.fetch",
 "params":{"source":"linkedin"},
 "queue":{"priority":"override"}}

// ← 2. refused pending confirmation, with what it would cost
{"v":1,"kind":"response","id":"p1","ok":false,
 "error":{"code":"confirmation_required",
  "message":"would displace 3 scheduled tasks in lane 'fetchers'",
  "detail":{"lane":"fetchers","queue_version":41,
    "would_displace":[
      {"task_id":"01JD2P…","op":"fetchers.fetch","priority":"scheduled"},
      {"task_id":"01JD2Q…","op":"fetchers.fetch","priority":"scheduled"},
      {"task_id":"01JD2R…","op":"records.reindex","priority":"scheduled"}]}}}

// → 3. user agreed; echo the queue_version you showed them
{"v":1,"kind":"request","id":"p2","op":"fetchers.fetch",
 "params":{"source":"linkedin"},
 "queue":{"priority":"override","confirm":true,"queue_version":41}}

// ← 4. accepted
{"v":1,"kind":"response","id":"p2","ok":true,
 "data":{"queued":true,"task_id":"01JD2S…","lane":"fetchers","position":0}}
```

`queue_version` increments on every mutation of that lane. If it changed between steps 2
and 3, the daemon returns `confirmation_required` again with the fresh list rather than
acting on a stale picture the user never saw. Optimistic concurrency, no locks.

Promotion emits `queue.task.promoted` carrying the displaced task ids — the audit trail
for "why did my scheduled fetch run late".

---

## Error codes

Stable strings. Clients may branch on `code`, never on `message`.

| Code | Meaning |
|---|---|
| `unsupported_version` | Protocol `v` not supported. Connection closes. |
| `bad_request` | Malformed frame, missing field, oversized line. |
| `unknown_op` | No module registered that op. |
| `invalid_params` | Op exists, params failed validation. |
| `not_found` | Referenced collection, record, workspace, task, trigger or source missing. |
| `conflict` | Concurrent modification, duplicate id, or stale `queue_version`. |
| `confirmation_required` | Action needs explicit user confirmation. See `detail`. |
| `workspace_dirty` | Workspace is in a half-configured state and cannot be activated. |
| `lane_unknown` | Requested lane is not declared by any module. |
| `not_cancellable` | Task already finished, or is running and does not honour cancel. |
| `module_error` | Module-specific failure. See `detail`. |
| `unavailable` | Module not initialised, or a dependency (network, script) failed. |
| `internal` | Bug. Always logged daemon-side with a backtrace. |

### `workspace_dirty` detail

```jsonc
{"code":"workspace_dirty",
 "message":"workspace 'deep-work' failed setup and was not cleaned up",
 "detail":{"workspace":"deep-work","failed_step":"3/7 tmux-session",
   "failed_at":"2026-09-16T09:12:44Z","log":"logs/deep-work-01JD2T.log",
   "has_cleanup_script":true}}
```

Clients should offer: run cleanup, force relaunch, or open the log. Never auto-force.

---

## Discovery

**Clients must not hardcode module knowledge.** Ask the daemon what exists.

```jsonc
// →
{"v":1,"kind":"request","id":"m1","op":"core.manifest","params":{}}

// ←
{"v":1,"kind":"response","id":"m1","ok":true,"data":{
  "protocol":1,
  "lanes":[
    {"id":"workspaces","max_concurrent":1},
    {"id":"fetchers","max_concurrent":8},
    {"id":"notify","max_concurrent":2},
    {"id":"index","max_concurrent":1},
    {"id":"default","max_concurrent":4}],
  "modules":[
    {"id":"records","version":"0.1.0","namespace":"records",
     "commands":[
       {"op":"records.list","summary":"List records in a collection",
        "execution":"inline","params_schema":{…}},
       {"op":"records.reindex","summary":"Rebuild the index for a collection",
        "execution":{"queued":{"lane":"index"}},"params_schema":{…}}],
     "topics":["records.item.created","records.item.updated",
               "records.item.completed"]}
  ]}}
```

`params_schema` is JSON Schema. Advisory before M3 — the TUI may render forms from it
later but must not depend on it being complete yet.

This op is what makes the platform extensible: a module written after the TUI shipped still
appears in it, in the right lane, with the right execution semantics.

---

## Core ops

Always present, independent of which modules are registered.

| Op | Execution | Params | Returns |
|---|---|---|---|
| `core.ping` | inline | `{}` | `{"pong":true,"uptime_s":N}` |
| `core.manifest` | inline | `{}` | See above. |
| `core.shutdown` | inline | `{}` | `{"ok":true}`, then a clean exit. |

## Queue ops

| Op | Execution | Params | Returns |
|---|---|---|---|
| `queue.list` | inline | `{"lane"?}` | Lanes, each with `running` and `queued` task arrays and `queue_version`. |
| `queue.task` | inline | `{"task_id"}` | One task with status, progress, attempts, origin. |
| `queue.reorder` | inline | `{"lane","task_id","before":TaskId\|null,"queue_version"}` | New `queue_version`. |
| `queue.cancel` | inline | `{"task_id"}` | `{"cancelled":true}` or `not_cancellable`. |

Reordering is within a lane only. The running task cannot be reordered or displaced; it can
be cancelled. A stale `queue_version` returns `conflict`.

## Scheduler ops

| Op | Execution | Params | Returns |
|---|---|---|---|
| `scheduler.list` | inline | `{}` | Triggers with `next_due`, `last_run`, `catch_up`, `paused`. |
| `scheduler.add` | inline | `{"schedule","op","params","catch_up","lane"?,"fallback"?}` | `{"trigger_id"}` |
| `scheduler.remove` | inline | `{"trigger_id"}` | `{"removed":true}` |
| `scheduler.pause` | inline | `{"trigger_id"}` | `{"paused":true}` |
| `scheduler.resume` | inline | `{"trigger_id"}` | `{"paused":false}` |

`catch_up` is **required** on `scheduler.add`. There is no default — `skip`, `run_once` and
`backfill` behave very differently after a laptop sleeps, and guessing sends duplicate
emails.

## Workspace ops

| Op | Execution | Params | Returns |
|---|---|---|---|
| `workspaces.list` | inline | `{}` | Each with `state`, and `dirty_reason` when dirty. |
| `workspaces.status` | inline | `{"id"}` | Full state, last session, log path. |
| `workspaces.activate` | queued (`workspaces`) | `{"id"}` | Task handle. Fails `workspace_dirty` if dirty. |
| `workspaces.cleanup` | queued (`workspaces`) | `{"id"}` | Task handle. On success, `dirty → ready`. |
| `workspaces.force_relaunch` | queued (`workspaces`) | `{"id"}` | Task handle. Activates despite `dirty`. Emits `workspaces.session.forced`. |
| `workspaces.reset` | inline | `{"id"}` | Clears `dirty` without running cleanup. Last resort. |

`state` is one of `ready`, `launching`, `active`, `dirty`. **`active` means the launch
succeeded and nothing more** — there is no liveness check, so a client must not present it
as "currently running".

Module ops are namespaced `<module>.<verb>`. The daemon routes on the prefix; a collision
between two modules is a startup failure, not a runtime surprise.

**Aliases never appear on the wire.** `bankai`, `arise` and every other command-pack name
are resolved to canonical ops by the client before a frame is sent. The daemon rejects an
alias as `unknown_op`. Swapping or installing a pack therefore never requires a protocol
change or a daemon restart. See `CLAUDE.md` §15.1.

---

## Topic catalogue

Non-exhaustive — modules may add their own. Clients subscribe to patterns and must
tolerate unknown topics.

| Topic | Emitted when |
|---|---|
| `core.stream.lagged` | This connection dropped events (see Backpressure). |
| `queue.task.enqueued` | Task accepted into a lane. |
| `queue.task.started` | Task began executing. |
| `queue.task.progress` | Module called `ctx.progress`. Payload `{fraction, note}`. |
| `queue.task.finished` | Task succeeded. |
| `queue.task.failed` | Task failed. Payload includes `error`, `attempts`, `lane`, `origin`. |
| `queue.task.cancelled` | Task cancelled before or during execution. |
| `queue.task.promoted` | A task jumped the lane. Payload lists displaced task ids. |
| `scheduler.trigger.fired` | Trigger fired and enqueued a task. |
| `scheduler.trigger.skipped` | Trigger fired while its previous task was still pending. |
| `workspaces.session.launched` | All launch steps succeeded. |
| `workspaces.session.dirty` | A launch step failed. Payload carries the failed step and log. |
| `workspaces.session.forced` | Force relaunch of a dirty workspace, with prior reason. |
| `workspaces.session.cleaned` | Cleanup script succeeded; state back to `ready`. |
| `records.item.created` / `.updated` / `.completed` | Record mutations. |
| `fetchers.item.found` | A source returned a new, deduplicated item. |
| `fetchers.fetch.finished` / `.failed` | A fetch run ended. |
| `calendar.date.registered` | A dated entry was stored. |
| `notify.sent` | Delivery succeeded, naming the sink used. |
| `notify.fallback_used` | Primary sink failed; the fallback delivered. |
| `notify.exhausted` | Every sink failed. Written to `notifications/failed.jsonl`. |

`queue.task.failed` is the topic a dashboard most needs: it is how failures become visible
without polling, and it is how the elevated-priority dirty-workspace warning reaches the
user.

---

## Mock daemon

`crates/tui` develops against `swe-mockd`, a fixture binary speaking this protocol, serving
canned responses from `crates/tui/fixtures/*.json` and replaying a scripted event timeline.
Built at M1.

It must be able to simulate, because these are the paths hardest to reach against a real
daemon: a `confirmation_required` round-trip, a `workspace_dirty` failure, a
`core.stream.lagged` drop, and a long queued task emitting progress.

```
$ swe-mockd --fixtures crates/tui/fixtures --socket /tmp/swe-mock.sock
$ swe tui --socket /tmp/swe-mock.sock
```

This is what decouples the TUI work. Dev C should never be blocked waiting for a
daemon-side op to land — write the fixture, build against it, swap to the real daemon at M5.

---

## Worked example

Connect, discover, subscribe, enqueue, watch it run.

```jsonc
→ {"v":1,"kind":"hello","client":"tui","client_version":"0.1.0"}
← {"v":1,"kind":"welcome","daemon_version":"0.1.0","session":"01JD2KQ…"}

→ {"v":1,"kind":"request","id":"1","op":"core.manifest","params":{}}
← {"v":1,"kind":"response","id":"1","ok":true,"data":{"protocol":1,"lanes":[…],"modules":[…]}}

→ {"v":1,"kind":"subscribe","id":"2","topics":["queue.*","fetchers.*"]}
← {"v":1,"kind":"response","id":"2","ok":true,"data":{"subscribed":2}}

→ {"v":1,"kind":"request","id":"3","op":"fetchers.fetch","params":{"source":"hn-whoishiring"}}
← {"v":1,"kind":"response","id":"3","ok":true,
   "data":{"queued":true,"task_id":"01JD2N…","lane":"fetchers","position":1}}

← {"v":1,"kind":"event","event":{"topic":"queue.task.started",
   "payload":{"task_id":"01JD2N…","lane":"fetchers"}, …}}
← {"v":1,"kind":"event","event":{"topic":"queue.task.progress",
   "payload":{"task_id":"01JD2N…","fraction":0.4,"note":"page 2 of 5"}, …}}
← {"v":1,"kind":"event","event":{"topic":"fetchers.item.found",
   "payload":{"source":"hn-whoishiring","id":"c-41883"}, …}}
← {"v":1,"kind":"event","event":{"topic":"queue.task.finished",
   "payload":{"task_id":"01JD2N…","new_items":12}, …}}

// meanwhile, a scheduled workspace launch fails in another lane
← {"v":1,"kind":"event","event":{"topic":"queue.task.failed",
   "payload":{"task_id":"01JD2M…","lane":"workspaces","origin":"scheduler",
     "error":"step 3/7 exited 1","workspace_dirty":true}, …}}
← {"v":1,"kind":"event","event":{"topic":"notify.sent",
   "payload":{"sink":"desktop","reason":"fallback","priority":"high"}, …}}
```

Two things to read out of the last four frames. The failing workspace task did not stall
the `fetchers` lane — different lanes, independent. And the scheduled trigger still reached
the user, as a notification instead of a launch, because its `fallback` fired. That is the
whole design working: decoupled, observable, and degrading rather than dropping.
