//! Turning response data into text. Pure functions over JSON: nothing here talks to the
//! daemon, and nothing here computes anything the daemon did not send (§2).

use std::fmt::Write as _;

use serde_json::Value;
use shimmer_core::{Error, Execution};
use shimmer_proto::{ManifestData, PingData};

pub fn ping(data: &Value) -> String {
    match serde_json::from_value::<PingData>(data.clone()) {
        Ok(p) => format!("pong (daemon up {})", uptime(p.uptime_s)),
        Err(_) => json(data),
    }
}

pub fn manifest(data: &Value) -> String {
    let Ok(m) = serde_json::from_value::<ManifestData>(data.clone()) else { return json(data) };
    let mut out = format!("protocol v{}\n\nlanes\n", m.protocol);
    let width = m.lanes.iter().map(|l| l.id.as_str().len()).max().unwrap_or(0);
    for l in &m.lanes {
        let _ = writeln!(out, "  {:width$}  max {}", l.id.as_str(), l.max_concurrent);
    }
    out.push_str("\nmodules\n");
    if m.modules.is_empty() {
        out.push_str("  (none registered)\n");
    }
    for module in &m.modules {
        let _ = writeln!(out, "  {} {}", module.id, module.version);
        let rows: Vec<(String, &str)> =
            module.commands.iter().map(|c| (execution(&c.execution), c.summary.as_str())).collect();
        let op_w = module.commands.iter().map(|c| c.op.len()).max().unwrap_or(0);
        let ex_w = rows.iter().map(|(e, _)| e.len()).max().unwrap_or(0);
        for (c, (ex, summary)) in module.commands.iter().zip(&rows) {
            let _ = writeln!(out, "    {:op_w$}  {ex:ex_w$}  {summary}", c.op);
        }
    }
    out.truncate(out.trim_end().len());
    out
}

pub fn shutdown() -> String {
    "daemon stopped".into()
}

/// Anything without a dedicated renderer, and everything under `--json`.
pub fn json(data: &Value) -> String {
    serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
}

/// `code: message`, plus the structured detail when there is one. Clients branch on the
/// code; the message is for people.
pub fn error(e: &Error) -> String {
    let mut out = format!("{}: {}", e.code, e.message);
    if let Some(detail) = e.detail.as_ref().filter(|d| !d.is_null()) {
        let _ = write!(out, "\n{}", json(detail));
    }
    out
}

fn execution(e: &Execution) -> String {
    match e {
        Execution::Inline => "inline".into(),
        Execution::Queued { lane } => format!("queued:{lane}"),
    }
}

fn uptime(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    match (h, m) {
        (0, 0) => format!("{s}s"),
        (0, _) => format!("{m}m {s}s"),
        _ => format!("{h}h {m}m {s}s"),
    }
}

// ---------------------------------------------------------------- tables, shared by records and workspaces

pub(crate) const MAX_CELL: usize = 40;

/// One value as table text: strings bare, `null` as `-`, long values cut with `…`.
pub(crate) fn cell(v: &Value) -> String {
    let s = match v {
        Value::Null => "-".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > MAX_CELL {
        s.chars().take(MAX_CELL - 1).chain(['…']).collect()
    } else {
        s
    }
}

pub(crate) fn table(header: &[String], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, c) in widths.iter_mut().zip(row) {
            *w = (*w).max(c.chars().count());
        }
    }
    let line = |cells: &[String]| {
        let mut s = String::new();
        for (i, (c, w)) in cells.iter().zip(&widths).enumerate() {
            if i + 1 == cells.len() {
                s.push_str(c);
            } else {
                let pad = w - c.chars().count();
                let _ = write!(s, "{c}{}  ", " ".repeat(pad));
            }
        }
        s
    };
    let mut out = line(header);
    for row in rows {
        out.push('\n');
        out.push_str(&line(row));
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn ping_shows_uptime() {
        assert_eq!(ping(&json!({"pong": true, "uptime_s": 4})), "pong (daemon up 4s)");
        assert_eq!(ping(&json!({"pong": true, "uptime_s": 3725})), "pong (daemon up 1h 2m 5s)");
        // An unexpected shape is shown, not hidden.
        assert_eq!(ping(&json!({"surprise": 1})), "{\n  \"surprise\": 1\n}");
    }

    #[test]
    fn manifest_lists_lanes_modules_and_ops() {
        let data = json!({
            "protocol": 1,
            "lanes": [{"id": "default", "max_concurrent": 4}, {"id": "index", "max_concurrent": 1}],
            "modules": [{
                "id": "records", "version": "0.1.0", "namespace": "records", "topics": [],
                "commands": [
                    {"op": "records.list", "summary": "List records", "params_schema": {}, "execution": "inline"},
                    {"op": "records.reindex", "summary": "Rebuild the index", "params_schema": {},
                     "execution": {"queued": {"lane": "index"}}}
                ]
            }]
        });
        let want = "\
protocol v1

lanes
  default  max 4
  index    max 1

modules
  records 0.1.0
    records.list     inline        List records
    records.reindex  queued:index  Rebuild the index";
        assert_eq!(manifest(&data), want);
    }

    #[test]
    fn empty_manifest_says_so() {
        let data = json!({"protocol": 1, "lanes": [{"id": "default", "max_concurrent": 4}], "modules": []});
        assert!(manifest(&data).ends_with("modules\n  (none registered)"));
    }

    #[test]
    fn errors_show_code_message_and_detail() {
        assert_eq!(error(&Error::not_found("no collection 'x'")), "not_found: no collection 'x'");
        let e = Error::conflict("stale").with_detail(json!({"queue_version": 41}));
        assert_eq!(error(&e), "conflict: stale\n{\n  \"queue_version\": 41\n}");
    }
}
