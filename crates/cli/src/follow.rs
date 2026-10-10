//! Waiting for a queued task to end, for `--wait`: one copy for every command that queues work
//! (#142). Follows the task's events on the subscription stream; when the stream lags and events
//! were dropped, asks `queue.task` instead.

use serde_json::{json, Value};
use shimmer_core::{Error, ErrorCode, Result};

use crate::client::Client;

/// How a followed task ended, when it didn't fail.
#[derive(Debug, PartialEq)]
pub enum Followed {
    /// Its `queue.task.finished` event arrived: the op's result.
    Finished(Value),
    /// It succeeded, but its result went with events the stream dropped: `queue.task` has no
    /// `result`. Carries what `queue.task` did say. Still a success: the caller says the output
    /// was missed, never that the work failed.
    ResultMissed(Value),
}

/// Follow task `task` until it ends: how it ended, or the error it failed with (e.g.
/// `workspace_dirty`, with its detail). With `notes`, progress notes go to stderr. `look` is the
/// command that shows what happened, named when events were dropped.
pub async fn follow(client: &mut Client, task: &str, notes: bool, look: &str) -> Result<Followed> {
    let mut last_note = String::new();
    loop {
        let ev = client.next_event().await?;
        if ev.topic == "core.stream.lagged" {
            // Events were dropped, maybe the one we wait for: ask directly instead.
            let t = match client.call("queue.task", json!({"task_id": task})).await {
                Ok(t) => t,
                // The daemon keeps only its most recent finished tasks: this one is gone too.
                Err(e) if e.code == ErrorCode::NotFound => return Err(forgotten(task, look)),
                Err(e) => return Err(e),
            };
            match ended(&t, look) {
                Some(outcome) => return outcome,
                None => continue,
            }
        }
        if ev.payload["task_id"] != task {
            continue;
        }
        match ev.topic.as_str() {
            "queue.task.progress" => {
                let note = ev.payload["note"].as_str().unwrap_or_default();
                if notes && !note.is_empty() && note != last_note {
                    eprintln!("  {note} …");
                    last_note = note.to_owned();
                }
            }
            "queue.task.finished" => return Ok(Followed::Finished(ev.payload["result"].clone())),
            "queue.task.failed" => return Err(task_error(&ev.payload)),
            "queue.task.cancelled" => return Err(cancelled()),
            _ => {}
        }
    }
}

/// From `queue.task`'s reply `t` after a lag: how the task ended, or `None` while it is still
/// queued or running. `queue.task` carries neither the result nor the error's detail, so a
/// failure names `look` for the rest.
fn ended(t: &Value, look: &str) -> Option<Result<Followed>> {
    match t["status"].as_str() {
        Some("succeeded") => Some(Ok(Followed::ResultMissed(t.clone()))),
        Some("failed") => Some(Err(Error::new(
            ErrorCode::ModuleError,
            format!(
                "{} (some events were dropped, so details may be missing: see '{look}')",
                bare_message(t["error"].as_str().unwrap_or("the task failed"))
            ),
        ))),
        Some("cancelled") => Some(Err(cancelled())),
        _ => None,
    }
}

/// The error a `queue.task.failed` event carries, rebuilt as the daemon sent it.
pub(crate) fn task_error(payload: &Value) -> Error {
    let code: ErrorCode = serde_json::from_value(payload["code"].clone()).unwrap_or(ErrorCode::ModuleError);
    let mut e = Error::new(code, payload["error"].as_str().unwrap_or("the task failed").to_owned());
    if let Some(detail) = payload.get("detail").filter(|d| !d.is_null()) {
        e = e.with_detail(detail.clone());
    }
    e
}

/// `queue.task`'s `error` is the error as the daemon prints it, `"<code>: <message>"`; the
/// `queue.task.failed` event carries the message alone. Strip the code, so a failure reads the
/// same whether or not the stream lagged.
fn bare_message(error: &str) -> &str {
    match error.split_once(": ") {
        Some((code, message)) if serde_json::from_value::<ErrorCode>(json!(code)).is_ok() => message,
        _ => error,
    }
}

/// After a lag, when the daemon no longer remembers the task either.
fn forgotten(task: &str, look: &str) -> Error {
    Error::new(
        ErrorCode::ModuleError,
        format!(
            "some events were dropped and the daemon no longer remembers task {task}, so how it ended \
             is unknown: see '{look}'"
        ),
    )
}

fn cancelled() -> Error {
    Error::new(ErrorCode::ModuleError, "the task was cancelled")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOK: &str = "shimmer workspaces status site";

    #[test]
    fn after_a_lag_a_success_is_still_a_success_with_its_result_missed() {
        let t = json!({"task_id": "t1", "status": "succeeded", "error": null});
        assert_eq!(ended(&t, LOOK).unwrap().unwrap(), Followed::ResultMissed(t.clone()));
    }

    #[test]
    fn after_a_lag_a_failure_says_where_to_look() {
        let t = json!({"task_id": "t1", "status": "failed", "error": "workspace 'site' is dirty"});
        let e = ended(&t, LOOK).unwrap().unwrap_err();
        assert_eq!(
            e.message,
            "workspace 'site' is dirty (some events were dropped, so details may be missing: \
             see 'shimmer workspaces status site')"
        );
        let cancelled = ended(&json!({"status": "cancelled"}), LOOK).unwrap().unwrap_err();
        assert_eq!(cancelled.message, "the task was cancelled");
    }

    #[test]
    fn after_a_lag_a_failure_reads_as_it_would_without_one() {
        // queue.task prints the error with its code in front; the event has the message alone.
        let t = json!({"status": "failed", "error": "workspace_dirty: workspace 'site' failed at step 1/3 setup"});
        let e = ended(&t, LOOK).unwrap().unwrap_err();
        assert!(e.message.starts_with("workspace 'site' failed at step 1/3 setup (some events"), "{}", e.message);
        // Only a real code is stripped.
        assert_eq!(bare_message("step 2: it broke"), "step 2: it broke");
        assert_eq!(bare_message("no code here"), "no code here");
    }

    #[test]
    fn after_a_lag_a_task_still_going_is_followed_further() {
        for status in ["queued", "running"] {
            assert!(ended(&json!({"status": status}), LOOK).is_none(), "{status}");
        }
    }

    #[test]
    fn a_failed_task_keeps_its_code_and_detail() {
        let payload = json!({"task_id": "01", "error": "workspace 'deep-work' failed at step 1/3 setup (exit code 1)",
                             "code": "workspace_dirty",
                             "detail": {"workspace": "deep-work", "log": "logs/x.log", "has_cleanup_script": true}});
        let e = task_error(&payload);
        assert_eq!(e.code, ErrorCode::WorkspaceDirty);
        assert_eq!(e.detail.as_ref().unwrap()["log"], "logs/x.log");

        let other =
            task_error(&json!({"task_id": "01", "error": "no launch backend registered", "code": "unavailable"}));
        assert_eq!((other.code, other.detail), (ErrorCode::Unavailable, None));
        assert_eq!(task_error(&json!({"task_id": "01"})).code, ErrorCode::ModuleError, "unknown code still fails");
    }

    /// A daemon that, once connected, sends `events`, then answers `queue.task` with `task`, as
    /// a lagging stream does: the events that mattered are gone.
    async fn lagging_daemon(sock: &std::path::Path, events: Vec<shimmer_core::Event>, task: Result<Value>) {
        use shimmer_proto::{decode_client, encode, ClientFrame, ServerFrame};
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::UnixListener::bind(sock).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (rd, mut wr) = stream.into_split();
            let mut lines = BufReader::new(rd).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let frames = match decode_client(&line).unwrap() {
                    ClientFrame::Hello { .. } => std::iter::once(ServerFrame::welcome("test", "s1"))
                        .chain(events.iter().cloned().map(ServerFrame::event))
                        .collect(),
                    ClientFrame::Request { id, op, .. } if op == "queue.task" => {
                        vec![match &task {
                            Ok(v) => ServerFrame::ok(id, v.clone()),
                            Err(e) => ServerFrame::err(id, e.clone()),
                        }]
                    }
                    _ => vec![],
                };
                for frame in frames {
                    wr.write_all(encode(&frame).unwrap().as_bytes()).await.unwrap();
                }
            }
        });
    }

    fn event(topic: &str, payload: Value) -> shimmer_core::Event {
        shimmer_core::Event::new(shimmer_core::ModuleId::new("queue"), topic, payload, Default::default())
    }

    #[tokio::test]
    async fn a_success_lost_to_a_lag_is_reported_as_a_success_through_a_real_client() {
        let dir = tempfile::TempDir::new().unwrap();
        let sock = dir.path().join("s.sock");
        // Another task's event, then the lag that swallowed ours; queue.task says it succeeded,
        // with no result (the daemon's `view` has none).
        let events = vec![
            event("queue.task.progress", json!({"task_id": "other", "note": "x"})),
            event(shimmer_proto::topics::STREAM_LAGGED, json!({"dropped": 12})),
        ];
        let view = json!({"task_id": "t1", "status": "succeeded", "error": null});
        lagging_daemon(&sock, events, Ok(view.clone())).await;
        let mut client = Client::connect(&sock).await.unwrap();
        let outcome = follow(&mut client, "t1", false, LOOK).await.unwrap();
        assert_eq!(outcome, Followed::ResultMissed(view));
    }

    #[tokio::test]
    async fn a_failure_lost_to_a_lag_is_an_error_naming_where_to_look() {
        let dir = tempfile::TempDir::new().unwrap();
        let sock = dir.path().join("s.sock");
        let events = vec![event(shimmer_proto::topics::STREAM_LAGGED, json!({"dropped": 3}))];
        lagging_daemon(&sock, events, Ok(json!({"task_id": "t1", "status": "failed", "error": "step 1 failed"}))).await;
        let mut client = Client::connect(&sock).await.unwrap();
        let e = follow(&mut client, "t1", false, LOOK).await.unwrap_err();
        assert!(
            e.message.starts_with("step 1 failed (some events were dropped") && e.message.contains(LOOK),
            "{}",
            e.message
        );
    }

    #[tokio::test]
    async fn a_task_the_daemon_has_forgotten_says_so_instead_of_no_task() {
        // The daemon keeps only its most recent finished tasks; after a long lag it may not know
        // this one any more.
        let dir = tempfile::TempDir::new().unwrap();
        let sock = dir.path().join("s.sock");
        let events = vec![event(shimmer_proto::topics::STREAM_LAGGED, json!({"dropped": 5000}))];
        lagging_daemon(&sock, events, Err(Error::not_found("no task t1"))).await;
        let mut client = Client::connect(&sock).await.unwrap();
        let e = follow(&mut client, "t1", false, LOOK).await.unwrap_err();
        assert!(e.message.contains("no longer remembers task t1") && e.message.contains(LOOK), "{}", e.message);
    }
}
