//! Append-only event log: `events/YYYY-MM-DD.jsonl`, one JSON `Event` per line.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use swe_core::{Event, Result};

use crate::atomic::fsync_dir;

pub struct EventScan {
    pub events: Vec<Event>,
    /// Lines that did not parse: typically one torn final line after a crash.
    pub skipped: usize,
}

fn day_file(home: &Path, event: &Event) -> PathBuf {
    home.join("events").join(format!("{}.jsonl", event.at.format("%Y-%m-%d")))
}

pub fn append(home: &Path, event: &Event) -> Result<()> {
    let path = day_file(home, event);
    let mut line = serde_json::to_vec(event).map_err(|e| swe_core::Error::internal(format!("encode event: {e}")))?;
    line.push(b'\n');

    let existed = path.exists();
    let mut f = OpenOptions::new().create(true).append(true).read(true).open(&path)?;
    // A crash mid-append can leave a line with no newline; don't glue the next event to it.
    if existed && !ends_with_newline(&mut f)? {
        line.insert(0, b'\n');
    }
    f.write_all(&line)?;
    f.sync_data()?;
    if !existed {
        fsync_dir(path.parent().unwrap_or(home))?;
    }
    Ok(())
}

fn ends_with_newline(f: &mut File) -> io::Result<bool> {
    if f.metadata()?.len() == 0 {
        return Ok(true);
    }
    f.seek(SeekFrom::End(-1))?;
    let mut b = [0u8; 1];
    f.read_exact(&mut b)?;
    Ok(b[0] == b'\n')
}

/// Whether an event with this id is already in its day's file.
pub fn contains(home: &Path, event: &Event) -> Result<bool> {
    let text = match fs::read_to_string(day_file(home, event)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let needle = event.id.to_string();
    Ok(text
        .lines()
        .filter(|l| l.contains(&needle))
        .filter_map(|l| serde_json::from_str::<Event>(l).ok())
        .any(|e| e.id == event.id))
}

pub fn read_all(home: &Path) -> Result<EventScan> {
    let mut files: Vec<PathBuf> = fs::read_dir(home.join("events"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    let mut scan = EventScan { events: Vec::new(), skipped: 0 };
    for file in files {
        for line in String::from_utf8_lossy(&fs::read(&file)?).lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Event>(line) {
                Ok(e) => scan.events.push(e),
                Err(_) => scan.skipped += 1,
            }
        }
    }
    Ok(scan)
}
