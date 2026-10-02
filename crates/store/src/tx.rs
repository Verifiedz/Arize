//! Multi-file transactions (§7.1).
//!
//! 1. Stage every write under `.staging/<txid>/`, plus a `manifest.json` describing the
//!    transaction and the events to log; fsync all of it.
//! 2. Write the `COMMITTED` marker. This is the commit point.
//! 3. Apply: put files into place, delete, append events not already logged, remove staging.
//!
//! Step 3 is idempotent and is exactly what recovery runs, so a crash anywhere after step 2
//! rolls forward and a crash before it rolls back (staging deleted).

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use shimmer_core::{Error, Event, Result, TxPlan, Write};

use crate::atomic::{fsync_dir, write_atomic};
use crate::log;

const COMMITTED: &str = "COMMITTED";
const MANIFEST: &str = "manifest.json";

#[derive(Serialize, Deserialize)]
struct Staged {
    namespace: String,
    ops: Vec<StagedOp>,
    events: Vec<Event>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum StagedOp {
    /// Namespace-relative destination, and the staged copy under `files/`.
    Put {
        path: String,
        file: String,
    },
    Delete {
        path: String,
    },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Recovery {
    pub rolled_forward: usize,
    pub discarded: usize,
}

pub fn commit(home: &Path, ns_dir: &Path, namespace: &str, plan: TxPlan) -> Result<()> {
    // Fast path: a lone file write needs no staging (§7.1, first bullet).
    if plan.events.is_empty() {
        if let [Write::Put { path, bytes }] = plan.writes.as_slice() {
            return write_atomic(&ns_dir.join(path), bytes).map_err(Into::into);
        }
    }
    let dir = stage(home, namespace, &plan)?;
    apply(home, &dir)
}

fn staging_root(home: &Path) -> PathBuf {
    home.join(".staging")
}

/// Steps 1 and 2. Returns the staging directory, committed.
fn stage(home: &Path, namespace: &str, plan: &TxPlan) -> Result<PathBuf> {
    let dir = staging_root(home).join(plan.id.to_string());
    fs::create_dir(&dir).map_err(|e| Error::conflict(format!("cannot create staging dir: {e}")))?;
    fs::create_dir(dir.join("files"))?;

    let mut ops = Vec::with_capacity(plan.writes.len());
    for (i, w) in plan.writes.iter().enumerate() {
        match w {
            Write::Put { path, bytes } => {
                let file = format!("files/{i}");
                let mut f = File::create(dir.join(&file))?;
                f.write_all(bytes)?;
                f.sync_all()?;
                ops.push(StagedOp::Put { path: path.clone(), file });
            }
            Write::Delete { path } => ops.push(StagedOp::Delete { path: path.clone() }),
        }
    }
    let manifest = Staged { namespace: namespace.to_owned(), ops, events: plan.events.clone() };
    let json = serde_json::to_vec(&manifest).map_err(|e| Error::internal(format!("encode tx: {e}")))?;
    write_atomic(&dir.join(MANIFEST), &json)?;
    fsync_dir(&dir.join("files"))?;
    fsync_dir(&dir)?;

    File::create(dir.join(COMMITTED))?.sync_all()?;
    fsync_dir(&dir)?;
    fsync_dir(&staging_root(home))?;
    Ok(dir)
}

/// Step 3. Safe to run any number of times on a committed staging directory.
fn apply(home: &Path, dir: &Path) -> Result<()> {
    let staged: Staged = serde_json::from_slice(&fs::read(dir.join(MANIFEST))?)
        .map_err(|e| Error::internal(format!("corrupt staged transaction {}: {e}", dir.display())))?;
    let ns_dir = home.join("data").join(&staged.namespace);

    for op in &staged.ops {
        match op {
            StagedOp::Put { path, file } => {
                // Copy then rename inside the destination directory, so this also works
                // if staging and data ever sit on different filesystems.
                write_atomic(&ns_dir.join(path), &fs::read(dir.join(file))?)?;
            }
            StagedOp::Delete { path } => {
                let dest = ns_dir.join(path);
                match fs::remove_file(&dest) {
                    Ok(()) => {
                        if let Some(parent) = dest.parent() {
                            fsync_dir(parent)?;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    for event in &staged.events {
        if !log::contains(home, event)? {
            log::append(home, event)?;
        }
    }
    fs::remove_dir_all(dir)?;
    fsync_dir(&staging_root(home))?;
    Ok(())
}

pub fn recover(home: &Path) -> Result<Recovery> {
    let mut r = Recovery::default();
    for entry in fs::read_dir(staging_root(home))? {
        let dir = entry?.path();
        if !dir.is_dir() {
            continue;
        }
        if dir.join(COMMITTED).exists() {
            apply(home, &dir)?;
            r.rolled_forward += 1;
        } else {
            fs::remove_dir_all(&dir)?;
            r.discarded += 1;
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use shimmer_core::{Clock, StoreBackend};
    use tempfile::TempDir;
    use ulid::Ulid;

    use super::*;
    use crate::Store;

    fn event(topic: &str) -> Event {
        Event::new("records".into(), topic, json!({}), Clock::system().now())
    }

    fn plan(events: Vec<Event>) -> TxPlan {
        TxPlan {
            id: Ulid::new(),
            writes: vec![
                Write::Put { path: "a.toml".into(), bytes: b"a".to_vec() },
                Write::Put { path: "sub/b.toml".into(), bytes: b"b".to_vec() },
            ],
            events,
        }
    }

    fn logged(store: &Store) -> Vec<Event> {
        store.read_events().unwrap().events
    }

    #[test]
    fn commit_applies_files_and_logs_events() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        let ev = event("records.item.created");
        store.commit("records", plan(vec![ev.clone()])).unwrap();

        assert_eq!(store.read("records", "sub/b.toml").unwrap().unwrap(), b"b");
        assert_eq!(store.list("records", "").unwrap(), ["a.toml", "sub/b.toml"]);
        assert_eq!(store.list("records", "sub").unwrap(), ["sub/b.toml"]);
        assert_eq!(logged(&store), [ev]);
        assert_eq!(fs::read_dir(home.path().join(".staging")).unwrap().count(), 0);
    }

    #[test]
    fn crash_before_commit_marker_rolls_back() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        let dir = stage(home.path(), "records", &plan(vec![event("records.item.created")])).unwrap();
        fs::remove_file(dir.join(COMMITTED)).unwrap(); // crash landed just before the marker

        let r = store.recover().unwrap();
        assert_eq!(r, Recovery { rolled_forward: 0, discarded: 1 });
        assert!(store.read("records", "a.toml").unwrap().is_none());
        assert!(logged(&store).is_empty());
    }

    #[test]
    fn crash_after_commit_marker_rolls_forward_exactly_once() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        let ev = event("records.item.created");
        stage(home.path(), "records", &plan(vec![ev.clone()])).unwrap(); // marker written, nothing applied

        assert_eq!(store.recover().unwrap().rolled_forward, 1);
        assert_eq!(store.read("records", "a.toml").unwrap().unwrap(), b"a");
        assert_eq!(logged(&store), [ev]);
        assert_eq!(store.recover().unwrap(), Recovery::default(), "recovery is idempotent");
    }

    #[test]
    fn crash_mid_apply_does_not_duplicate_the_event() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        let ev = event("records.item.created");
        let dir = stage(home.path(), "records", &plan(vec![ev.clone()])).unwrap();

        // Simulate dying after files and the event were applied but before staging was removed.
        let ns = home.path().join("data/records");
        write_atomic(&ns.join("a.toml"), b"a").unwrap();
        log::append(home.path(), &ev).unwrap();
        assert!(dir.exists());

        store.recover().unwrap();
        assert_eq!(logged(&store), [ev], "event id already in the log is not appended again");
        assert_eq!(store.read("records", "sub/b.toml").unwrap().unwrap(), b"b");
    }

    #[test]
    fn delete_and_single_write_fast_path() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        let put = |bytes: &[u8]| TxPlan {
            id: Ulid::new(),
            writes: vec![Write::Put { path: "x".into(), bytes: bytes.to_vec() }],
            events: vec![],
        };
        store.commit("records", put(b"1")).unwrap();
        store.commit("records", put(b"2")).unwrap();
        assert_eq!(store.read("records", "x").unwrap().unwrap(), b"2");
        assert_eq!(fs::read_dir(home.path().join(".staging")).unwrap().count(), 0);

        let del = TxPlan { id: Ulid::new(), writes: vec![Write::Delete { path: "x".into() }], events: vec![] };
        store.commit("records", del).unwrap();
        assert!(store.read("records", "x").unwrap().is_none());
    }

    #[test]
    fn namespaces_are_isolated_and_validated() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        assert!(store.read("../etc", "passwd").is_err());
        assert!(store.read("records", "../calendar/x").is_err());
    }

    #[test]
    fn open_ships_gitignore_and_survives_torn_log_line() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        let gi = fs::read_to_string(home.path().join(".gitignore")).unwrap();
        for entry in ["index.sqlite*", "cache/", "logs/", ".staging/"] {
            assert!(gi.contains(entry), "{entry}");
        }

        let e1 = event("records.item.created");
        store.append_event(&e1).unwrap();
        let path = home.path().join("events").join(format!("{}.jsonl", e1.at.format("%Y-%m-%d")));
        fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{\"id\":\"torn").unwrap();
        let e2 = event("records.item.updated");
        store.append_event(&e2).unwrap();

        let scan = store.read_events().unwrap();
        assert_eq!(scan.events, [e1, e2], "event after a torn line is not lost");
        assert_eq!(scan.skipped, 1);
    }
}
