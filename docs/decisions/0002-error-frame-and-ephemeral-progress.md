# 0002. An `error` frame, and progress events that are not logged

Status: accepted (M0) · Needs sign-off: Dev A, Dev B, Dev C notified (protocol change)

## Context

`docs/protocol.md` says a rejected handshake gets "`error` and closes", and that malformed
frames are `bad_request`, yet lists no `error` message kind. Neither case has a request
`id` to hang a `response` on. Separately, the protocol says events on the wire are
byte-identical to the JSONL log, but `queue.task.progress` can fire many times a second and
each log append is fsynced.

## Decision

1. Add a daemon → client frame `{"v":1,"kind":"error","error":{code,message,detail}}` for
   failures with no request to answer. After a rejected handshake or an oversized line the
   daemon closes; after an unparseable line it keeps the connection.
2. `queue.task.progress` is published on the bus (so subscribers see it) but **not**
   appended to the event log or index. It is transient status, not history. Every other
   event is logged before it is published.

Both are additive, so `v` stays `1`; the protocol has not shipped to a client yet.
