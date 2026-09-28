//! A corrupt `index.sqlite` must not stop the daemon from starting (CLAUDE.md §1.4/§7:
//! "delete it, corrupt it... and the daemon rebuilds it from the event log"). The recovery
//! mechanism lives in `crates/store/src/index.rs::Index::open`, unit-tested there against
//! synthetic error codes and real garbage/truncated/empty files in isolation — these tests
//! are the end-to-end proof that a real restart of the real binary actually benefits from it.

mod common;

use common::{stderr, wait_gone, Home};
use serde_json::{json, Value};
use swe_store::Index;

fn call(home: &Home, op: &str, params: Value) -> Value {
    let o = home.swe(&["call", op, &params.to_string(), "--json"]);
    assert!(o.status.success(), "{op} failed: {}", stderr(&o));
    serde_json::from_slice(&o.stdout).unwrap()
}

fn index_path(home: &Home) -> std::path::PathBuf {
    home.dir.path().join("home/index.sqlite")
}

fn index_event_count(home: &Home) -> u64 {
    Index::open(&index_path(home)).unwrap().stats().unwrap().events
}

/// One completed record: `records.collection.created` (seeding LeetCode) +
/// `records.item.created` + `records.item.completed` — 3 events. Shuts the daemon down so
/// the index file sits still while the test tampers with it before the next restart.
fn seed_and_stop(home: &Home) {
    call(
        home,
        "records.add",
        json!({"collection": "leetcode", "id": "two-sum", "fields": {"title": "Two Sum", "difficulty": "easy"}}),
    );
    call(home, "records.complete", json!({"collection": "leetcode", "id": "two-sum"}));
    assert!(home.swe(&["shutdown"]).status.success());
    wait_gone(&home.socket());
}

#[test]
fn a_corrupt_index_does_not_stop_the_daemon_starting() {
    let home = Home::new();
    seed_and_stop(&home);
    let before = index_event_count(&home);
    assert_eq!(before, 3, "collection.created + item.created + item.completed");

    std::fs::write(index_path(&home), b"not a sqlite file at all, just garbage bytes").unwrap();

    // The bug this proves fixed: this used to make the daemon refuse to start at all
    // ("cannot start daemon: internal: index: file is not a database").
    let pong = home.swe(&["ping"]);
    assert!(pong.status.success(), "daemon failed to start over a corrupt index: {}", stderr(&pong));

    assert_eq!(index_event_count(&home), before, "rebuilt from the event log to the same count");
}

#[test]
fn a_truncated_index_does_not_stop_the_daemon_starting() {
    let home = Home::new();
    seed_and_stop(&home);
    let before = index_event_count(&home);

    // A real database, cut off partway through — SQLITE_CORRUPT ("database disk image is
    // malformed"), a distinct code from garbage bytes' SQLITE_NOTADB, and both must recover.
    let bytes = std::fs::read(index_path(&home)).unwrap();
    std::fs::write(index_path(&home), &bytes[..bytes.len() / 3]).unwrap();

    let pong = home.swe(&["ping"]);
    assert!(pong.status.success(), "daemon failed to start over a truncated index: {}", stderr(&pong));
    assert_eq!(index_event_count(&home), before);
}

#[test]
fn an_empty_index_file_was_never_a_failure_case() {
    let home = Home::new();
    seed_and_stop(&home);
    let before = index_event_count(&home);

    // SQLite treats a zero-length file as a fresh, valid database — there is no error here
    // to recover from. This proves the daemon still starts and still repopulates it, not
    // that the new recovery code path fires (it doesn't need to).
    std::fs::write(index_path(&home), b"").unwrap();

    let pong = home.swe(&["ping"]);
    assert!(pong.status.success(), "{}", stderr(&pong));
    assert_eq!(index_event_count(&home), before);
}
