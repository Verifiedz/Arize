//! The derived SQLite index. Never a source of truth (§1.4): every row here came from the
//! event log, and deleting the file loses nothing.

use std::path::Path;

use rusqlite::{params, Connection};
use swe_core::{Error, Event, Result};

use crate::Store;

fn db_err(e: rusqlite::Error) -> Error {
    Error::internal(format!("index: {e}"))
}

#[derive(Debug, PartialEq, Eq)]
pub struct IndexStats {
    pub events: u64,
}

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(db_err)?;
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
        )
        .map_err(db_err)?;
        Ok(Self { conn })
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
}
