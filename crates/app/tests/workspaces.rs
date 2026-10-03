//! The workspaces module in the real daemon, through the real binary (issue #32). No launch
//! backend is wired in yet (ADR 0010, Dev A), so this covers what works today: the module is
//! registered with its lane, `list`/`status` read real folders, and `activate` fails cleanly
//! with `unavailable` without leaving the workspace half-configured.

mod common;

use std::time::{Duration, Instant};

use common::{stderr, wait_gone, Home};
use serde_json::{json, Value};

fn call(home: &Home, op: &str, params: Value) -> Value {
    let o = home.shimmer(&["call", op, &params.to_string(), "--json"]);
    assert!(o.status.success(), "{op} failed: {}", stderr(&o));
    serde_json::from_slice(&o.stdout).unwrap()
}

/// `data/workspaces/<id>/` with `files` as (relative path, contents).
fn workspace(home: &Home, id: &str, files: &[(&str, &str)]) {
    let dir = home.dir.path().join("home/data/workspaces").join(id);
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

const DEEP_WORK: &str =
    "[workspace]\nlabel = \"Deep Work\"\n\n[[step]]\nname = \"setup\"\nmode = \"supervised\"\ntimeout_s = 30\n";

/// Poll `queue.task` until the task has finished.
fn finished_task(home: &Home, task_id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let t = call(home, "queue.task", json!({"task_id": task_id}));
        if !matches!(t["status"].as_str(), Some("queued" | "running")) {
            return t;
        }
        assert!(Instant::now() < deadline, "task never finished: {t}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn workspaces_are_registered_with_their_lane() {
    let home = Home::new();
    let m = call(&home, "core.manifest", json!({}));
    let module = m["modules"].as_array().unwrap().iter().find(|m| m["id"] == "workspaces").expect("registered");
    let ops: Vec<_> = module["commands"].as_array().unwrap().iter().map(|c| c["op"].as_str().unwrap()).collect();
    assert_eq!(
        ops,
        [
            "workspaces.list",
            "workspaces.status",
            "workspaces.activate",
            "workspaces.cleanup",
            "workspaces.force_relaunch",
            "workspaces.reset"
        ]
    );
    let lanes = m["lanes"].as_array().unwrap();
    assert!(lanes.iter().any(|l| l["id"] == "workspaces" && l["max_concurrent"] == 1), "{m}");
}

#[test]
fn list_and_status_read_real_folders_and_a_broken_one_hides_nothing() {
    let home = Home::new();
    call(&home, "core.ping", json!({})); // starts the daemon, which creates the data dir
    workspace(&home, "deep-work", &[("workspace.toml", DEEP_WORK), ("steps/01-setup.sh", "echo hi\n")]);
    workspace(&home, "broken", &[("workspace.toml", "[workspace\n")]);

    let list = call(&home, "workspaces.list", json!({}));
    let list = list["workspaces"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!((list[0]["id"].as_str(), list[0]["state"].as_str()), (Some("broken"), Some("invalid")));
    assert!(list[0]["error"].as_str().unwrap().starts_with("broken/workspace.toml: "));
    assert_eq!(list[1], json!({"id": "deep-work", "label": "Deep Work", "state": "ready"}));

    let status = call(&home, "workspaces.status", json!({"id": "deep-work"}));
    assert_eq!(status["steps"][0], json!({"index": 1, "name": "setup", "mode": "supervised", "timeout_s": 30}));
    assert_eq!(status["last_session"], Value::Null);
}

#[test]
fn activate_without_a_launch_backend_fails_cleanly_and_survives_a_restart() {
    let home = Home::new();
    call(&home, "core.ping", json!({}));
    workspace(&home, "deep-work", &[("workspace.toml", DEEP_WORK), ("steps/01-setup.sh", "echo hi\n")]);

    // Queued: the request returns a task handle; the outcome arrives on the task.
    let handle = call(&home, "workspaces.activate", json!({"id": "deep-work"}));
    assert_eq!(handle["lane"], "workspaces");
    let task = finished_task(&home, handle["task_id"].as_str().unwrap());
    assert_eq!(task["status"], "failed", "{task}");
    assert!(task["error"].as_str().unwrap().starts_with("unavailable: "), "{task}");

    // Nothing ran, so it is not dirty: still ready, before and after a restart.
    let state = |home: &Home| call(home, "workspaces.status", json!({"id": "deep-work"}))["state"].clone();
    assert_eq!(state(&home), "ready");
    assert!(home.shimmer(&["shutdown"]).status.success());
    wait_gone(&home.socket());
    assert_eq!(state(&home), "ready");
}
