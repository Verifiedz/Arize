//! `shimmer workspaces …`: typed commands over the `workspaces.*` ops (docs/protocol.md,
//! "Workspace ops"; issue #33).
//!
//! A thin client (CLAUDE.md §2): the daemon decides everything (whether a workspace may launch,
//! what dirty means). The CLI only sends ops, shows what comes back, and asks before the two
//! actions that skip safety: `reset` and `force-relaunch`.
//!
//! `activate`, `cleanup` and `force-relaunch` are queued ops: the request returns a task handle
//! at once. With `--wait` the CLI follows the task over the subscription stream (docs/protocol.md,
//! `subscribe`) and prints how it ended. Without it, it returns straight away; that is the
//! default because a workspace's own step calling `shimmer workspaces activate … --wait` would
//! wait on a launch that cannot start until its own launch ends (one lane slot, §6.1).

use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal, Write as _};

use serde_json::{json, Value};
use shimmer_core::{Error, ErrorCode, Result};

use crate::client::Client;
use crate::render::{self, cell, table};

pub const USAGE: &str = "usage: shimmer workspaces <command>

commands:
  list                                  list workspaces and their state
  status NAME                           show one workspace in full: steps, last session, why it is dirty
  activate NAME [--wait]                launch its steps in order
  cleanup NAME [--wait]                 run a dirty workspace's cleanup script
  force-relaunch NAME [--yes] [--wait]  launch a dirty workspace anyway (asks first)
  reset NAME [--yes]                    clear dirty without running cleanup (asks first)

--wait  follow the launch until it ends and show how it went. Without it the command
        returns once the launch is queued; check on it with 'shimmer workspaces status NAME'.
        Don't use --wait from a workspace's own step scripts: launches run one at a time.
--yes   skip the question. Needed when no one is at the keyboard (scripts).

A workspace is a folder under data/workspaces/ in your Shimmer folder, holding a
workspace.toml and its steps/ scripts (ADR 0012). Add --json to any command for raw output.";

#[derive(Clone, Debug, PartialEq)]
pub enum WorkspacesCmd {
    Help,
    List,
    Status { id: String },
    Activate { id: String, wait: bool },
    Cleanup { id: String, wait: bool },
    ForceRelaunch { id: String, yes: bool, wait: bool },
    Reset { id: String, yes: bool },
}

// ---------------------------------------------------------------- parsing

/// `words` are what follows `shimmer workspaces`, with global flags already removed.
pub fn parse(words: Vec<String>) -> std::result::Result<WorkspacesCmd, String> {
    let mut words = words.into_iter();
    let Some(sub) = words.next() else { return Ok(WorkspacesCmd::Help) };
    let mut positional = Vec::new();
    let (mut yes, mut wait) = (false, false);
    for word in words {
        match word.as_str() {
            "--yes" | "-y" => yes = true,
            "--wait" => wait = true,
            w if w.starts_with('-') && w.len() > 1 => {
                return Err(format!("unknown option '{}'", w.split('=').next().unwrap_or(w)));
            }
            _ => positional.push(word),
        }
    }
    // Which command takes which flag; anything else is a mistake worth naming.
    let (takes_yes, takes_wait) = match sub.as_str() {
        "force-relaunch" => (true, true),
        "activate" | "cleanup" => (false, true),
        "reset" => (true, false),
        _ => (false, false),
    };
    for (given, allowed, flag) in [(yes, takes_yes, "--yes"), (wait, takes_wait, "--wait")] {
        if given && !allowed {
            return Err(format!("'workspaces {sub}' takes no {flag}"));
        }
    }
    let name = |positional: Vec<String>| -> std::result::Result<String, String> {
        match <[String; 1]>::try_from(positional) {
            Ok([id]) => Ok(id),
            Err(_) => Err(format!("usage: shimmer workspaces {sub} NAME")),
        }
    };
    match sub.as_str() {
        "help" => Ok(WorkspacesCmd::Help),
        "list" => match positional.is_empty() {
            true => Ok(WorkspacesCmd::List),
            false => Err("'workspaces list' takes no arguments".into()),
        },
        "status" => Ok(WorkspacesCmd::Status { id: name(positional)? }),
        "activate" => Ok(WorkspacesCmd::Activate { id: name(positional)?, wait }),
        "cleanup" => Ok(WorkspacesCmd::Cleanup { id: name(positional)?, wait }),
        "force-relaunch" => Ok(WorkspacesCmd::ForceRelaunch { id: name(positional)?, yes, wait }),
        "reset" => Ok(WorkspacesCmd::Reset { id: name(positional)?, yes }),
        other => Err(format!("unknown workspaces command '{other}'; see 'shimmer workspaces --help'")),
    }
}

// ---------------------------------------------------------------- asking first

/// How the CLI asks "are you sure?". A real terminal, or a scripted answer in tests.
pub trait Prompt {
    /// Is a person there to answer? False in scripts, pipes, and workspace steps calling
    /// `shimmer` through `SHIMMER_SOCKET`: the CLI must never wait on a question nobody sees.
    fn interactive(&self) -> bool;
    /// Ask a yes/no question; anything but y/yes is no.
    fn confirm(&mut self, question: &str) -> bool;
}

/// The real one: asks on stderr, so `--json` output on stdout stays clean.
pub struct Terminal;

impl Prompt for Terminal {
    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal()
    }

    fn confirm(&mut self, question: &str) -> bool {
        eprint!("{question} [y/N] ");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        if std::io::stdin().lock().read_line(&mut answer).is_err() {
            return false;
        }
        matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }
}

/// Decide whether to go ahead with an action that skips safety. `status` is what
/// `workspaces.status` returned, shown so the person knows what they're overriding.
/// `Ok(false)` means they said no.
fn confirm(prompt: &mut dyn Prompt, yes: bool, action: &str, question: &str, id: &str, status: &Value) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !prompt.interactive() {
        return Err(Error::invalid_params(format!(
            "refusing to {action} '{id}' without confirmation; pass --yes to do it anyway"
        )));
    }
    eprintln!("{}", summary(status));
    Ok(prompt.confirm(question))
}

// ---------------------------------------------------------------- running

pub async fn run(client: &mut Client, cmd: &WorkspacesCmd, json: bool, prompt: &mut dyn Prompt) -> Result<String> {
    let id_params = |id: &str| json!({"id": id});
    let out = |data: &Value, text: String| if json { render::json(data) } else { text };
    match cmd {
        WorkspacesCmd::Help => Ok(USAGE.into()),
        WorkspacesCmd::List => {
            let data = client.call("workspaces.list", json!({})).await?;
            Ok(out(&data, list(&data)))
        }
        WorkspacesCmd::Status { id } => {
            let data = client.call("workspaces.status", id_params(id)).await?;
            Ok(out(&data, status(&data)))
        }
        WorkspacesCmd::Activate { id, wait } => queued(client, Queued::Activate, id, *wait, json).await,
        WorkspacesCmd::Cleanup { id, wait } => queued(client, Queued::Cleanup, id, *wait, json).await,
        WorkspacesCmd::ForceRelaunch { id, yes, wait } => {
            if !*yes {
                let current = client.call("workspaces.status", id_params(id)).await?;
                let question = "Launch it anyway, without cleaning up?";
                if !confirm(prompt, false, "force a relaunch of", question, id, &current)? {
                    return Ok(format!("{id} was not relaunched"));
                }
            }
            queued(client, Queued::ForceRelaunch, id, *wait, json).await
        }
        WorkspacesCmd::Reset { id, yes } => {
            if !*yes {
                let current = client.call("workspaces.status", id_params(id)).await?;
                let question = "Reset it to ready without running its cleanup script?";
                if !confirm(prompt, false, "reset", question, id, &current)? {
                    return Ok(format!("{id} was not reset"));
                }
            }
            let data = client.call("workspaces.reset", id_params(id)).await?;
            Ok(out(&data, format!("✓ {id} reset to ready")))
        }
    }
}

/// The three queued ops, for what they print.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Queued {
    Activate,
    Cleanup,
    ForceRelaunch,
}

impl Queued {
    fn op(self) -> &'static str {
        match self {
            Self::Activate => "workspaces.activate",
            Self::Cleanup => "workspaces.cleanup",
            Self::ForceRelaunch => "workspaces.force_relaunch",
        }
    }

    fn what(self) -> &'static str {
        match self {
            Self::Activate => "launch",
            Self::Cleanup => "cleanup",
            Self::ForceRelaunch => "forced relaunch",
        }
    }

    fn done(self, id: &str) -> String {
        match self {
            Self::Activate | Self::ForceRelaunch => format!("✓ {id} is active"),
            Self::Cleanup => format!("✓ {id} is cleaned up and ready"),
        }
    }
}

/// Send a queued op; with `wait`, follow its task to the end.
async fn queued(client: &mut Client, kind: Queued, id: &str, wait: bool, json: bool) -> Result<String> {
    if wait {
        // Before the request, so not even the first event can be missed.
        client.subscribe(&["queue.task.*"]).await?;
    }
    let handle = client.call(kind.op(), json!({"id": id})).await?;
    let task = handle["task_id"].as_str().ok_or_else(|| Error::internal("no task_id in the queued response"))?;
    if !wait {
        return Ok(if json { render::json(&handle) } else { queued_message(kind, id, task) });
    }
    match follow(client, task, json).await {
        Ok(result) => Ok(if json { render::json(&result) } else { kind.done(id) }),
        Err(e) if json => Err(e),
        Err(e) => Err(explain(e, id)),
    }
}

fn queued_message(kind: Queued, id: &str, task: &str) -> String {
    format!("queued the {} of {id} (task {task})\ncheck on it with: shimmer workspaces status {id}", kind.what())
}

/// Follow task `task` over the subscription stream until it ends: its result, or the error it
/// failed with (e.g. `workspace_dirty`, with its detail). Progress notes go to stderr.
async fn follow(client: &mut Client, task: &str, json: bool) -> Result<Value> {
    let mut last_note = String::new();
    loop {
        let ev = client.next_event().await?;
        if ev.topic == "core.stream.lagged" {
            // Events were dropped, maybe the one we wait for: ask directly instead.
            if let Some(outcome) = finished_task(client, task).await? {
                return outcome;
            }
            continue;
        }
        if ev.payload["task_id"] != task {
            continue;
        }
        match ev.topic.as_str() {
            "queue.task.progress" => {
                let note = ev.payload["note"].as_str().unwrap_or_default();
                if !json && !note.is_empty() && note != last_note {
                    eprintln!("  {note} …");
                    last_note = note.to_owned();
                }
            }
            "queue.task.finished" => return Ok(ev.payload["result"].clone()),
            "queue.task.failed" => return Err(task_error(&ev.payload)),
            "queue.task.cancelled" => return Err(cancelled()),
            _ => {}
        }
    }
}

/// After `core.stream.lagged`: has the task ended? `None` while it is still queued or running.
async fn finished_task(client: &mut Client, task: &str) -> Result<Option<Result<Value>>> {
    let t = client.call("queue.task", json!({"task_id": task})).await?;
    Ok(match t["status"].as_str() {
        Some("succeeded") => Some(Ok(t["result"].clone())),
        Some("failed") => {
            Some(Err(Error::new(ErrorCode::ModuleError, t["error"].as_str().unwrap_or("the task failed").to_owned())))
        }
        Some("cancelled") => Some(Err(cancelled())),
        _ => None,
    })
}

/// The error a `queue.task.failed` event carries, rebuilt as the daemon sent it.
fn task_error(payload: &Value) -> Error {
    let code: ErrorCode = serde_json::from_value(payload["code"].clone()).unwrap_or(ErrorCode::ModuleError);
    let mut e = Error::new(code, payload["error"].as_str().unwrap_or("the task failed").to_owned());
    if let Some(detail) = payload.get("detail").filter(|d| !d.is_null()) {
        e = e.with_detail(detail.clone());
    }
    e
}

fn cancelled() -> Error {
    Error::new(ErrorCode::ModuleError, "the task was cancelled")
}

/// For people: a dirty workspace's error says where to look and what to do (§10.3), instead
/// of raw JSON detail. Other errors are shown as they are.
fn explain(e: Error, id: &str) -> Error {
    if e.code != ErrorCode::WorkspaceDirty {
        return e;
    }
    let detail = e.detail.clone().unwrap_or(Value::Null);
    let mut message = e.message.clone();
    if let Some(log) = detail["log"].as_str().filter(|l| !l.is_empty()) {
        let _ = write!(message, "\n  log: {log}");
    }
    let _ = write!(message, "\n{}", ways_out(id, detail["has_cleanup_script"] == true));
    Error::new(ErrorCode::WorkspaceDirty, message)
}

// ---------------------------------------------------------------- output

/// `workspaces.list` as a table. Everything shown comes from the daemon (§2).
pub fn list(data: &Value) -> String {
    let items = data["workspaces"].as_array().map(Vec::as_slice).unwrap_or_default();
    if items.is_empty() {
        return "no workspaces yet: add a folder with a workspace.toml under data/workspaces/ \
                in your Shimmer folder (ADR 0012)"
            .into();
    }
    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|w| {
            // The error first: for an invalid workspace, that is what needs fixing.
            let note = w.get("error").or(w.get("dirty_reason")).map(first_line).unwrap_or_default();
            vec![cell(&w["id"]), cell(&w["state"]), cell(&w["label"]), note]
        })
        .collect();
    let mut out = table(&["ID".into(), "STATE".into(), "LABEL".into(), "WHY".into()], &rows);
    if items.iter().any(|w| matches!(w["state"].as_str(), Some("dirty" | "invalid"))) {
        out.push_str("\n\nmore detail: shimmer workspaces status NAME");
    }
    out
}

/// `workspaces.status` in full.
pub fn status(data: &Value) -> String {
    let id = data["id"].as_str().unwrap_or_default();
    let mut out = format!("{id}  ({})", cell(&data["state"]));
    if let Some(label) = data["label"].as_str() {
        let _ = write!(out, "  {label}");
    }
    if let Some(d) = data["description"].as_str() {
        let _ = write!(out, "\n  {d}");
    }
    if let Some(e) = data["error"].as_str() {
        let _ = write!(out, "\n  invalid: {}", e.trim_end().replace('\n', "\n    "));
    }
    if let Some(steps) = data["steps"].as_array() {
        out.push_str("\n  steps:");
        let width = steps.iter().filter_map(|s| s["name"].as_str()).map(str::len).max().unwrap_or(0);
        for s in steps {
            let mode = match s["timeout_s"].as_u64() {
                Some(t) => format!("{}, timeout {t}s", cell(&s["mode"])),
                None => cell(&s["mode"]),
            };
            let _ = write!(out, "\n    {}  {:width$}  {mode}", cell(&s["index"]), cell(&s["name"]));
            if let Some(d) = s["description"].as_str() {
                let _ = write!(out, "  ({d})");
            }
        }
    }
    let _ = match (data["has_cleanup_script"].as_bool(), data["cleanup_timeout_s"].as_u64()) {
        (Some(true), Some(t)) => write!(out, "\n  cleanup: cleanup script, timeout {t}s"),
        (Some(true), None) => write!(out, "\n  cleanup: cleanup script"),
        _ => write!(out, "\n  cleanup: none (reset clears dirty)"),
    };
    if let Some(step) = data["running_step"].as_str() {
        let _ = write!(out, "\n  running: step {step}");
    }
    if let Some(d) = data.get("dirty").filter(|d| d.is_object()) {
        let _ = write!(
            out,
            "\n  dirty: step {}: {} (at {})",
            cell(&d["failed_step"]),
            d["reason"].as_str().unwrap_or_default(),
            cell(&d["failed_at"])
        );
        if let Some(log) = d["log"].as_str().filter(|l| !l.is_empty()) {
            let _ = write!(out, "\n  log: {log}");
        }
    }
    match data.get("last_session").filter(|l| l.is_object()) {
        Some(l) => {
            let forced = if l["forced"] == true { ", forced" } else { "" };
            let _ = write!(out, "\n  last session: {} at {}{forced}", cell(&l["outcome"]), cell(&l["started_at"]));
        }
        None => out.push_str("\n  last session: none yet"),
    }
    if data["state"] == "dirty" {
        let _ = write!(out, "\n{}", ways_out(id, data["has_cleanup_script"] == true));
    }
    out
}

/// What to do about a dirty workspace (§10.3), shown wherever the CLI reports one.
pub fn ways_out(id: &str, has_cleanup_script: bool) -> String {
    let mut out = String::from("  fix it with one of:");
    if has_cleanup_script {
        let _ = write!(out, "\n    shimmer workspaces cleanup {id}          run its cleanup script");
    }
    let _ = write!(
        out,
        "\n    shimmer workspaces force-relaunch {id}   launch it anyway\n    \
         shimmer workspaces reset {id}            mark it ready without cleanup"
    );
    out
}

/// One line about the workspace's state, shown before asking to confirm.
fn summary(status: &Value) -> String {
    let id = status["id"].as_str().unwrap_or_default();
    match status["state"].as_str() {
        Some("dirty") => {
            let reason = status["dirty_reason"].as_str().unwrap_or("failed");
            let mut s = format!("{id} is dirty: {reason}");
            if let Some(log) = status["log"].as_str().filter(|l| !l.is_empty()) {
                let _ = write!(s, "\n  log: {log}");
            }
            s
        }
        Some(state) => format!("{id} is {state}"),
        None => id.to_owned(),
    }
}

fn first_line(v: &Value) -> String {
    let full = v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string());
    cell(&Value::String(full.lines().next().unwrap_or_default().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> std::result::Result<WorkspacesCmd, String> {
        parse(words.iter().map(|w| w.to_string()).collect())
    }

    #[test]
    fn commands_parse() {
        assert_eq!(parse_words(&[]).unwrap(), WorkspacesCmd::Help);
        assert_eq!(parse_words(&["help"]).unwrap(), WorkspacesCmd::Help);
        assert_eq!(parse_words(&["list"]).unwrap(), WorkspacesCmd::List);
        assert_eq!(parse_words(&["status", "deep-work"]).unwrap(), WorkspacesCmd::Status { id: "deep-work".into() });
        assert_eq!(
            parse_words(&["reset", "deep-work"]).unwrap(),
            WorkspacesCmd::Reset { id: "deep-work".into(), yes: false }
        );
        for yes in [&["reset", "--yes", "deep-work"][..], &["reset", "deep-work", "-y"]] {
            assert_eq!(parse_words(yes).unwrap(), WorkspacesCmd::Reset { id: "deep-work".into(), yes: true });
        }
    }

    #[test]
    fn queued_commands_take_wait_and_force_relaunch_takes_yes() {
        assert_eq!(
            parse_words(&["activate", "deep-work"]).unwrap(),
            WorkspacesCmd::Activate { id: "deep-work".into(), wait: false }
        );
        assert_eq!(
            parse_words(&["activate", "--wait", "deep-work"]).unwrap(),
            WorkspacesCmd::Activate { id: "deep-work".into(), wait: true }
        );
        assert_eq!(
            parse_words(&["cleanup", "deep-work", "--wait"]).unwrap(),
            WorkspacesCmd::Cleanup { id: "deep-work".into(), wait: true }
        );
        assert_eq!(
            parse_words(&["force-relaunch", "deep-work", "--yes", "--wait"]).unwrap(),
            WorkspacesCmd::ForceRelaunch { id: "deep-work".into(), yes: true, wait: true }
        );
        for (words, expected) in [
            (&["activate", "x", "--yes"][..], "'workspaces activate' takes no --yes"),
            (&["reset", "x", "--wait"], "'workspaces reset' takes no --wait"),
            (&["status", "x", "--wait"], "'workspaces status' takes no --wait"),
            (&["force_relaunch", "x"], "unknown workspaces command 'force_relaunch'"),
        ] {
            let e = parse_words(words).unwrap_err();
            assert!(e.contains(expected), "{words:?}: {e}");
        }
    }

    #[test]
    fn without_wait_it_points_to_status_never_to_a_second_launch() {
        // Re-running `activate --wait` would queue another launch, so the hint is `status`.
        let m = queued_message(Queued::ForceRelaunch, "deep-work", "01TASK");
        assert_eq!(
            m,
            "queued the forced relaunch of deep-work (task 01TASK)\ncheck on it with: shimmer workspaces status deep-work"
        );
        assert!(!m.contains("--wait"), "{m}");
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

    #[test]
    fn a_dirty_failure_is_explained_for_people() {
        let e = task_error(
            &json!({"error": "workspace 'deep-work' failed at step 1/3 setup (exit code 1) and was not cleaned up",
                                    "code": "workspace_dirty",
                                    "detail": {"log": "logs/x.log", "has_cleanup_script": false}}),
        );
        let explained = explain(e, "deep-work");
        assert_eq!(explained.code, ErrorCode::WorkspaceDirty);
        assert_eq!(explained.detail, None, "the detail is in the message now, not as raw JSON");
        assert_eq!(
            explained.message,
            format!(
                "workspace 'deep-work' failed at step 1/3 setup (exit code 1) and was not cleaned up\n  log: logs/x.log\n{}",
                ways_out("deep-work", false)
            )
        );
        // Anything else is passed through untouched.
        let other = explain(Error::unavailable("no launch backend registered"), "deep-work");
        assert_eq!(other.message, "no launch backend registered");
    }

    #[test]
    fn misuse_is_explained() {
        let cases: [(&[&str], &str); 6] = [
            (&["status"], "usage: shimmer workspaces status NAME"),
            (&["status", "a", "b"], "usage: shimmer workspaces status NAME"),
            (&["list", "x"], "'workspaces list' takes no arguments"),
            (&["list", "--yes"], "'workspaces list' takes no --yes"),
            (&["reset", "x", "--force"], "unknown option '--force'"),
            (&["launch", "x"], "unknown workspaces command 'launch'"),
        ];
        for (words, expected) in cases {
            let e = parse_words(words).unwrap_err();
            assert!(e.contains(expected), "{words:?}: {e}");
        }
    }

    fn sample_list() -> Value {
        json!({"workspaces": [
            {"id": "broken", "state": "invalid", "error": "broken/workspace.toml: TOML parse error\n  | …"},
            {"id": "deep-work", "label": "Deep Work", "state": "dirty", "dirty_reason": "step 1/3 setup: exit code 1"},
            {"id": "focus", "label": "Focus", "state": "ready"},
        ]})
    }

    #[test]
    fn list_is_a_table_with_why() {
        let out = list(&sample_list());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "ID         STATE    LABEL      WHY");
        assert_eq!(lines[1], "broken     invalid  -          broken/workspace.toml: TOML parse error");
        assert_eq!(lines[2], "deep-work  dirty    Deep Work  step 1/3 setup: exit code 1");
        assert_eq!(lines[3], "focus      ready    Focus      ");
        assert_eq!(lines.last().unwrap(), &"more detail: shimmer workspaces status NAME");
    }

    #[test]
    fn an_empty_list_says_how_to_add_one() {
        assert!(list(&json!({"workspaces": []})).starts_with("no workspaces yet"));
        let healthy = list(&json!({"workspaces": [{"id": "a", "label": "A", "state": "active"}]}));
        assert!(!healthy.contains("more detail"), "{healthy}");
    }

    #[test]
    fn status_of_a_dirty_workspace_shows_the_failure_and_the_ways_out() {
        let data = json!({
            "id": "deep-work", "label": "Deep Work", "description": "Shimmer on Hyprland", "state": "dirty",
            "steps": [
                {"index": 1, "name": "setup", "mode": "supervised", "timeout_s": 60, "description": "pull and start db"},
                {"index": 2, "name": "editor", "mode": "detached"},
            ],
            "has_cleanup_script": true, "cleanup_timeout_s": 30,
            "dirty_reason": "step 1/2 setup: exit code 1", "log": "logs/deep-work-01.log",
            "dirty": {"reason": "exit code 1", "failed_step": "1/2 setup",
                      "failed_at": "2026-09-16T09:12:44Z", "log": "logs/deep-work-01.log"},
            "last_session": {"id": "01", "started_at": "2026-09-16T09:12:30Z", "forced": true, "outcome": "dirty"},
        });
        assert_eq!(
            status(&data),
            "deep-work  (dirty)  Deep Work
  Shimmer on Hyprland
  steps:
    1  setup   supervised, timeout 60s  (pull and start db)
    2  editor  detached
  cleanup: cleanup script, timeout 30s
  dirty: step 1/2 setup: exit code 1 (at 2026-09-16T09:12:44Z)
  log: logs/deep-work-01.log
  last session: dirty at 2026-09-16T09:12:30Z, forced
  fix it with one of:
    shimmer workspaces cleanup deep-work          run its cleanup script
    shimmer workspaces force-relaunch deep-work   launch it anyway
    shimmer workspaces reset deep-work            mark it ready without cleanup"
        );
    }

    #[test]
    fn status_of_a_new_or_invalid_workspace() {
        let new = json!({"id": "focus", "label": "Focus", "state": "ready", "has_cleanup_script": false,
                         "steps": [{"index": 1, "name": "setup", "mode": "detached"}], "last_session": null});
        let out = status(&new);
        assert!(out.contains("cleanup: none (reset clears dirty)"), "{out}");
        assert!(out.ends_with("last session: none yet"), "{out}");

        let invalid = json!({"id": "broken", "state": "invalid", "has_cleanup_script": false,
                             "error": "broken/workspace.toml: line 1\n  | [workspace\n", "last_session": null});
        let out = status(&invalid);
        assert!(
            out.starts_with("broken  (invalid)\n  invalid: broken/workspace.toml: line 1\n      | [workspace"),
            "{out}"
        );
    }

    /// A scripted person at the keyboard.
    struct Answer {
        interactive: bool,
        yes: bool,
        asked: Vec<String>,
    }

    impl Prompt for Answer {
        fn interactive(&self) -> bool {
            self.interactive
        }
        fn confirm(&mut self, question: &str) -> bool {
            self.asked.push(question.to_owned());
            self.yes
        }
    }

    fn answer(interactive: bool, yes: bool) -> Answer {
        Answer { interactive, yes, asked: Vec::new() }
    }

    #[test]
    fn confirming_asks_a_person_and_respects_no() {
        let dirty = json!({"id": "deep-work", "state": "dirty", "dirty_reason": "step 1/3 setup: exit code 1"});
        let mut p = answer(true, true);
        assert!(confirm(&mut p, false, "reset", "Reset?", "deep-work", &dirty).unwrap());
        assert_eq!(p.asked, ["Reset?"]);

        let mut p = answer(true, false);
        assert!(!confirm(&mut p, false, "reset", "Reset?", "deep-work", &dirty).unwrap());
    }

    #[test]
    fn yes_skips_the_question_and_scripts_never_wait_on_one() {
        let s = json!({"id": "deep-work", "state": "dirty"});
        let mut p = answer(false, false);
        assert!(confirm(&mut p, true, "reset", "Reset?", "deep-work", &s).unwrap());
        assert!(p.asked.is_empty());

        // No person and no --yes: a clear error, never a hang.
        let e = confirm(&mut p, false, "reset", "Reset?", "deep-work", &s).unwrap_err();
        assert_eq!(e.message, "refusing to reset 'deep-work' without confirmation; pass --yes to do it anyway");
        assert!(p.asked.is_empty());
    }

    #[test]
    fn ways_out_only_offers_cleanup_when_there_is_a_script() {
        assert!(ways_out("deep-work", true).contains("workspaces cleanup deep-work"));
        let without = ways_out("deep-work", false);
        assert!(!without.contains("cleanup deep-work"), "{without}");
        assert!(without.contains("force-relaunch deep-work") && without.contains("reset deep-work"), "{without}");
    }

    #[test]
    fn the_summary_says_what_is_being_overridden() {
        let dirty = json!({"id": "deep-work", "state": "dirty", "dirty_reason": "step 1/3 setup: exit code 1",
                           "log": "logs/x.log"});
        assert_eq!(summary(&dirty), "deep-work is dirty: step 1/3 setup: exit code 1\n  log: logs/x.log");
        assert_eq!(summary(&json!({"id": "focus", "state": "ready"})), "focus is ready");
    }
}
