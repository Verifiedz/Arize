//! M1's definition of done (CLAUDE.md §14): a completed item survives a daemon restart.
//! Drives the real binary through `swe call` until the records CLI commands land.

mod common;

use common::{stderr, wait_gone, Home};
use serde_json::{json, Value};

fn call(home: &Home, op: &str, params: Value) -> Value {
    let o = home.swe(&["call", op, &params.to_string(), "--json"]);
    assert!(o.status.success(), "{op} failed: {}", stderr(&o));
    serde_json::from_slice(&o.stdout).unwrap()
}

#[test]
fn a_completed_item_survives_a_daemon_restart() {
    let home = Home::new();

    // The daemon registers records, which seeds the LeetCode collection.
    let m = call(&home, "core.manifest", json!({}));
    assert!(m["modules"].as_array().unwrap().iter().any(|m| m["id"] == "records"), "{m}");

    call(
        &home,
        "records.add",
        json!({"collection": "leetcode", "id": "two-sum", "fields": {"title": "Two Sum", "difficulty": "easy"}}),
    );
    let done = call(&home, "records.complete", json!({"collection": "leetcode", "id": "two-sum"}));
    assert_eq!(done["status"], "done");

    // Restart: stop the daemon, and let the next command start a fresh one.
    assert!(home.swe(&["shutdown"]).status.success());
    wait_gone(&home.socket());

    let after = call(&home, "records.list", json!({"collection": "leetcode", "filter": {"status": "done"}}));
    assert_eq!(after["total"], 1);
    assert_eq!(after["items"][0], done);

    // Files are the truth: the record is a readable file in $SWE_HOME.
    let file = home.dir.path().join("home/data/records/items/leetcode/two-sum.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("status = \"done\""), "{text}");
}
