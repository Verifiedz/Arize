//! The derived SQLite index. Never a source of truth (§1.4): every row here came from the
//! event log, and deleting the file loses nothing.

use std::path::Path;

use rusqlite::{params, Connection, ErrorCode};
use swe_core::{Error, Event, Result};

use crate::Store;

fn db_err(e: rusqlite::Error) -> Error {
    Error::internal(format!("index: {e}"))
}

/// Whether an `open_conn` failure means the file is corrupt or not a database at all, as
/// opposed to locked, busy, or a permissions problem. Only this case is safe to recover from
/// by deleting and rebuilding — deleting the wrong thing (a file another process merely has
/// locked, or one we can't read for permission reasons) is worse than refusing to start.
fn is_corrupt_or_not_a_database(e: &rusqlite::Error) -> bool {
    matches!(e.sqlite_error_code(), Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase))
}

fn open_conn(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS events (
             id     TEXT PRIMARY KEY,
             at     TEXT NOT NULL,
             source TEXT NOT NULL,
             topic  TEXT NOT NULL,
             json   TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS events_topic ON events(topic, id);
         CREATE INDEX IF NOT EXISTS events_at ON events(at);",
    )?;
    Ok(conn)
}

#[derive(Debug, PartialEq, Eq)]
pub struct IndexStats {
    pub events: u64,
}

pub struct Index {
    conn: Connection,
}

impl Index {
    /// Never a source of truth (§1.4), so a file that won't open as a valid database is
    /// safe to discard and rebuild from the event log — but only when that is actually why
    /// it failed. A locked or busy file, or a permissions error, is left untouched and
    /// propagated: the caller (`crates/daemon/src/indexer.rs`) is exercised end to end by a
    /// daemon-level test for exactly this recovery, not just this function in isolation.
    pub fn open(path: &Path) -> Result<Self> {
        match open_conn(path) {
            Ok(conn) => Ok(Self { conn }),
            Err(e) if is_corrupt_or_not_a_database(&e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "index.sqlite is corrupt; deleting it (and any -wal/-shm) and rebuilding from the event log"
                );
                for suffix in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
                }
                Ok(Self { conn: open_conn(path).map_err(db_err)? })
            }
            Err(e) => Err(db_err(e)),
        }
    }

    /// Project one event. Idempotent: the same id twice is one row.
    pub fn apply(&mut self, event: &Event) -> Result<()> {
        self.apply_all(std::slice::from_ref(event))
    }

    /// One SQLite transaction: all of `events` land or none do.
    pub fn apply_all(&mut self, events: &[Event]) -> Result<()> {
        let tx = self.conn.transaction().map_err(db_err)?;
        insert(&tx, events)?;
        tx.commit().map_err(db_err)
    }

    /// Bring the index up to date with the log without discarding anything.
    pub fn catch_up(&mut self, store: &Store) -> Result<()> {
        let scan = store.read_events()?;
        self.apply_all(&scan.events)
    }

    /// Throw the projection away and rebuild it from the log, atomically: a crash midway
    /// leaves the previous consistent index.
    pub fn rebuild(&mut self, store: &Store) -> Result<()> {
        let scan = store.read_events()?;
        let tx = self.conn.transaction().map_err(db_err)?;
        tx.execute("DELETE FROM events", []).map_err(db_err)?;
        insert(&tx, &scan.events)?;
        tx.commit().map_err(db_err)
    }

    pub fn stats(&self) -> Result<IndexStats> {
        let events = self.conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0)).map_err(db_err)?;
        Ok(IndexStats { events: events as u64 })
    }

    /// Events matching a subscription-style pattern (`a.b.c`, `a.*`, `*`), oldest first.
    pub fn events_matching(&self, pattern: &str, limit: usize) -> Result<Vec<Event>> {
        let (sql, arg) = match pattern.strip_suffix(".*") {
            _ if pattern == "*" => ("SELECT json FROM events ORDER BY id LIMIT ?2", String::new()),
            Some(prefix) => (
                "SELECT json FROM events WHERE substr(topic, 1, length(?1)) = ?1 ORDER BY id LIMIT ?2",
                format!("{prefix}."),
            ),
            None => ("SELECT json FROM events WHERE topic = ?1 ORDER BY id LIMIT ?2", pattern.to_owned()),
        };
        let mut stmt = self.conn.prepare(sql).map_err(db_err)?;
        let rows = stmt.query_map(params![arg, limit as i64], |r| r.get::<_, String>(0)).map_err(db_err)?;
        rows.map(|r| {
            let json = r.map_err(db_err)?;
            serde_json::from_str(&json).map_err(|e| Error::internal(format!("index row: {e}")))
        })
        .collect()
    }
}

fn insert(tx: &rusqlite::Transaction<'_>, events: &[Event]) -> Result<()> {
    let mut stmt = tx
        .prepare_cached("INSERT OR IGNORE INTO events (id, at, source, topic, json) VALUES (?1, ?2, ?3, ?4, ?5)")
        .map_err(db_err)?;
    for e in events {
        let json = serde_json::to_string(e).map_err(|e| Error::internal(format!("encode event: {e}")))?;
        stmt.execute(params![e.id.to_string(), e.at.to_rfc3339(), e.source.as_str(), e.topic, json]).map_err(db_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use swe_core::Clock;
    use tempfile::TempDir;

    use super::*;

    fn ev(topic: &str, n: u32) -> Event {
        Event::new("records".into(), topic, json!({ "n": n }), Clock::system().now())
    }

    /// CLAUDE.md §7: delete the index, reindex, compare query results.
    #[test]
    fn deleting_the_index_loses_nothing() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        let mut idx = Index::open(&store.index_path()).unwrap();
        for i in 0..20 {
            let e = ev(if i % 2 == 0 { "records.item.created" } else { "records.item.completed" }, i);
            store.append_event(&e).unwrap();
            idx.apply(&e).unwrap();
        }
        let before: Vec<_> =
            ["records.*", "records.item.completed", "*"].iter().map(|p| idx.events_matching(p, 100).unwrap()).collect();
        assert_eq!(before[0].len(), 20);
        assert_eq!(before[1].len(), 10);
        drop(idx);

        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", store.index_path().display()));
        }
        let mut idx = Index::open(&store.index_path()).unwrap();
        assert_eq!(idx.stats().unwrap().events, 0);
        idx.rebuild(&store).unwrap();
        let after: Vec<_> =
            ["records.*", "records.item.completed", "*"].iter().map(|p| idx.events_matching(p, 100).unwrap()).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn apply_is_idempotent_and_catch_up_fills_gaps() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        let mut idx = Index::open(&store.index_path()).unwrap();
        let (a, b) = (ev("records.item.created", 1), ev("records.item.created", 2));
        store.append_event(&a).unwrap();
        store.append_event(&b).unwrap();
        idx.apply(&a).unwrap();
        idx.apply(&a).unwrap();
        assert_eq!(idx.stats().unwrap().events, 1);
        idx.catch_up(&store).unwrap();
        assert_eq!(idx.stats().unwrap().events, 2);
    }

    /// Only a corrupt-or-not-a-database failure is safe to delete and rebuild from. A
    /// locked, busy, or permissions problem must be left completely alone: deleting the
    /// wrong thing is worse than refusing to start. Constructed directly against synthetic
    /// `rusqlite::Error` values (`ffi::Error::new` is public) rather than real lock
    /// contention, so this is exact and has no flakiness.
    #[test]
    fn only_corruption_is_treated_as_recoverable() {
        let code = |raw: std::ffi::c_int| rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(raw), None);
        assert!(is_corrupt_or_not_a_database(&code(rusqlite::ffi::SQLITE_CORRUPT)));
        assert!(is_corrupt_or_not_a_database(&code(rusqlite::ffi::SQLITE_NOTADB)));
        for raw in [
            rusqlite::ffi::SQLITE_BUSY,
            rusqlite::ffi::SQLITE_LOCKED,
            rusqlite::ffi::SQLITE_PERM,
            rusqlite::ffi::SQLITE_CANTOPEN,
            rusqlite::ffi::SQLITE_IOERR,
        ] {
            assert!(!is_corrupt_or_not_a_database(&code(raw)), "{raw} must not be treated as recoverable");
        }
    }

    #[test]
    fn open_recovers_from_garbage_bytes() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        std::fs::write(store.index_path(), b"not a sqlite file at all, just garbage bytes").unwrap();

        let mut idx = Index::open(&store.index_path()).unwrap();
        assert_eq!(idx.stats().unwrap().events, 0);
        idx.apply(&ev("records.item.created", 1)).unwrap();
        assert_eq!(idx.stats().unwrap().events, 1);
    }

    #[test]
    fn open_recovers_from_a_truncated_file() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        {
            // A real database, then cut off partway through — SQLITE_CORRUPT ("database
            // disk image is malformed"), distinct from garbage bytes' SQLITE_NOTADB.
            let mut idx = Index::open(&store.index_path()).unwrap();
            for i in 0..50 {
                idx.apply(&ev("records.item.created", i)).unwrap();
            }
        }
        let len = std::fs::metadata(store.index_path()).unwrap().len();
        let bytes = std::fs::read(store.index_path()).unwrap();
        std::fs::write(store.index_path(), &bytes[..(len / 3) as usize]).unwrap();

        let idx = Index::open(&store.index_path()).unwrap();
        assert_eq!(idx.stats().unwrap().events, 0, "truncated data is gone, not silently half-read");
    }

    /// Not a failure case at all — SQLite treats a zero-length file as a fresh database, so
    /// this proves the recovery path is never even triggered, not that it recovers.
    #[test]
    fn open_accepts_an_empty_file_without_any_recovery() {
        let home = TempDir::new().unwrap();
        let store = Store::open(home.path()).unwrap();
        std::fs::write(store.index_path(), b"").unwrap();

        let idx = Index::open(&store.index_path()).unwrap();
        assert_eq!(idx.stats().unwrap().events, 0);
    }
}
