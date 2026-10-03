//! `shimmer workspaces …`: typed commands over the `workspaces.*` ops (docs/protocol.md,
//! "Workspace ops"; issue #33).
//!
//! A thin client (CLAUDE.md §2): the daemon decides everything (whether a workspace may launch,
//! what dirty means). The CLI only sends ops, shows what comes back, and asks before the two
//! actions that skip safety: `reset` and `force-relaunch`.

use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal, Write as _};

use serde_json::{json, Value};
use shimmer_core::{Error, Result};

use crate::client::Client;
use crate::render::{self, cell, table};

pub const USAGE: &str = "usage: shimmer workspaces <command>

commands:
  list                list workspaces and their state
  status NAME         show one workspace in full: steps, last session, why it is dirty
  reset NAME [--yes]  clear dirty without running cleanup (asks first)

A workspace is a folder under data/workspaces/ in your Shimmer folder, holding a
workspace.toml and its steps/ scripts (ADR 0012). Add --json to any command for raw output.";

#[derive(Clone, Debug, PartialEq)]
pub enum WorkspacesCmd {
    Help,
    List,
    Status { id: String },
    Reset { id: String, yes: bool },
}

// ---------------------------------------------------------------- parsing

/// `words` are what follows `shimmer workspaces`, with global flags already removed.
pub fn parse(words: Vec<String>) -> std::result::Result<WorkspacesCmd, String> {
    let mut words = words.into_iter();
    let Some(sub) = words.next() else { return Ok(WorkspacesCmd::Help) };
    let mut positional = Vec::new();
    let mut yes = false;
    for word in words {
        match word.as_str() {
            "--yes" | "-y" => yes = true,
            w if w.starts_with('-') && w.len() > 1 => {
                return Err(format!("unknown option '{}'", w.split('=').next().unwrap_or(w)));
            }
            _ => positional.push(word),
        }
    }
    let only_yes_for = |cmd: &str| match yes {
        true => Err(format!("'workspaces {cmd}' takes no --yes")),
        false => Ok(()),
    };
    let name = |cmd: &str, positional: Vec<String>| -> std::result::Result<String, String> {
        match <[String; 1]>::try_from(positional) {
            Ok([id]) => Ok(id),
            Err(_) => Err(format!("usage: shimmer workspaces {cmd} NAME")),
        }
    };
    match sub.as_str() {
        "help" => Ok(WorkspacesCmd::Help),
        "list" => {
            only_yes_for("list")?;
            match positional.is_empty() {
                true => Ok(WorkspacesCmd::List),
                false => Err("'workspaces list' takes no arguments".into()),
            }
        }
        "status" => {
            only_yes_for("status")?;
            Ok(WorkspacesCmd::Status { id: name("status", positional)? })
        }
        "reset" => Ok(WorkspacesCmd::Reset { id: name("reset", positional)?, yes }),
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
            let note = w.get("dirty_reason").or(w.get("error")).map(first_line).unwrap_or_default();
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
