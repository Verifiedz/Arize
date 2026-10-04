//! `shimmer-cli`: the command-line client. A thin client (CLAUDE.md §2): it parses a command,
//! sends requests over the socket, and renders the answers. No business logic lives here.
//!
//! Depends on `core` and `proto` only (§3 rule 2). Command packs (§15.1, ADR 0013) live in
//! [`packs`]: aliases are rewritten to canonical command words here and never reach the wire.

mod args;
mod autostart;
mod client;
pub mod packs;
mod records;
mod render;
mod workspaces;

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{json, Value};
use shimmer_core::{Error, Result};
use shimmer_proto::ops;

pub use args::{Args, Command, USAGE};
pub use client::{Client, ConnectError};
pub use records::RecordsCmd;
pub use workspaces::WorkspacesCmd;

/// Run `shimmer <args>`: `args` are what follows the binary name. Exit codes: 0 success,
/// 1 the daemon (or reaching it) failed, 2 bad usage.
pub async fn run(args: Vec<String>) -> ExitCode {
    // Command packs (ADR 0013): find the active pack, then turn an alias into the command it
    // stands for before anything else sees the line. A pack problem is only ever a warning.
    let front = match packs::active::take_front_flags(args) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("shimmer: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let active = packs::active::load(front.pack.as_deref());
    for warning in &active.warnings {
        eprintln!("shimmer: {warning}");
    }
    let help = match (&active.pack, front.canonical) {
        (Some(pack), false) => format!("{USAGE}\n\n{}", packs::active::help_section(pack)),
        _ => USAGE.to_string(),
    };
    let args = packs::active::rewrite(front.args, active.pack.as_ref());

    let usage = usage_for(&args);
    let args = match Args::parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("shimmer: {e}\n\n{usage}");
            return ExitCode::from(2);
        }
    };
    match execute(&args, help).await {
        Ok(Some(text)) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("shimmer: {}", render::error(&e));
            ExitCode::FAILURE
        }
    }
}

/// The help to show after a usage error: chosen by the command word the user typed, not by
/// searching the error message, so `shimmer workspaces activate x --bogus` gets the workspaces
/// help even though its error ("unknown option '--bogus'") never says "workspaces".
fn usage_for(args: &[String]) -> &'static str {
    let mut words = args.iter().map(String::as_str);
    while let Some(word) = words.next() {
        match word {
            // `--socket PATH` takes the next word; skip it too.
            "--socket" => {
                words.next();
            }
            w if w.starts_with('-') => {}
            "records" => return records::USAGE,
            "workspaces" => return workspaces::USAGE,
            _ => return USAGE,
        }
    }
    USAGE
}

/// `help` is the top-level help: [`USAGE`], plus the active pack's aliases unless `--canonical`.
async fn execute(args: &Args, help: String) -> Result<Option<String>> {
    match &args.command {
        Command::Help => return Ok(Some(help)),
        Command::Records(RecordsCmd::Help) => return Ok(Some(records::USAGE.into())),
        Command::Workspaces(WorkspacesCmd::Help) => return Ok(Some(workspaces::USAGE.into())),
        _ => {}
    }
    let Some(mut client) = connect(args).await? else { return Ok(None) };

    if let Command::Records(cmd) = &args.command {
        return records::run(&mut client, cmd, args.json).await.map(Some);
    }
    if let Command::Workspaces(cmd) = &args.command {
        return workspaces::run(&mut client, cmd, args.json, &mut workspaces::Terminal).await.map(Some);
    }
    let (op, params) = match &args.command {
        Command::Ping => (ops::CORE_PING, json!({})),
        Command::Manifest => (ops::CORE_MANIFEST, json!({})),
        Command::Shutdown => (ops::CORE_SHUTDOWN, json!({})),
        Command::Call { op, params } => (op.as_str(), params.clone()),
        Command::Help | Command::Records(_) | Command::Workspaces(_) => unreachable!("handled above"),
    };
    let data = client.call(op, params).await?;
    Ok(Some(output(args, &data)))
}

/// `None` when there is nothing to do: `shimmer shutdown` with no daemon running.
async fn connect(args: &Args) -> Result<Option<Client>> {
    let socket = args.socket.clone().unwrap_or_else(shimmer_proto::paths::socket_path);
    let shutdown = args.command == Command::Shutdown;
    match Client::connect(&socket).await {
        Ok(c) => Ok(Some(c)),
        // Starting a daemon only to stop it again helps nobody.
        Err(e) if e.nobody_listening() && shutdown => {
            eprintln!("shimmer: the daemon is not running");
            Ok(None)
        }
        Err(e) if e.nobody_listening() && args.socket.is_none() => {
            autostart::start_and_connect(&daemon_exe()?, &socket).await.map(Some)
        }
        Err(e) => Err(e.into_error(&socket)),
    }
}

fn output(args: &Args, data: &Value) -> String {
    if args.json {
        return render::json(data);
    }
    match args.command {
        Command::Ping => render::ping(data),
        Command::Manifest => render::manifest(data),
        Command::Shutdown => render::shutdown(),
        Command::Call { .. } | Command::Help | Command::Records(_) | Command::Workspaces(_) => render::json(data),
    }
}

/// The daemon is this same binary, run as `shimmer daemon`.
fn daemon_exe() -> Result<PathBuf> {
    std::env::current_exe()
        .map_err(|e| Error::unavailable(format!("cannot find the shimmer binary to start the daemon: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(args: &[&str]) -> &'static str {
        usage_for(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn usage_follows_the_command_word() {
        assert_eq!(usage(&["workspaces", "activate", "x", "--bogus"]), workspaces::USAGE);
        assert_eq!(usage(&["--json", "--socket", "/tmp/s", "workspaces", "status"]), workspaces::USAGE);
        assert_eq!(usage(&["records", "list", "--limit", "x"]), records::USAGE);
        assert_eq!(usage(&["call"]), USAGE);
        assert_eq!(usage(&["--bogus"]), USAGE);
        assert_eq!(usage(&[]), USAGE);
    }
}
