//! `shimmer-cli`: the command-line client. A thin client (CLAUDE.md §2): it parses a command,
//! sends requests over the socket, and renders the answers. No business logic lives here.
//!
//! Depends on `core` and `proto` only (§3 rule 2). Command packs and aliases (§15.1) land
//! here later; they rewrite the command word and never reach the wire.

mod args;
mod autostart;
mod client;
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
    let args = match Args::parse(args) {
        Ok(a) => a,
        Err(e) => {
            let usage = if e.contains("records") {
                records::USAGE
            } else if e.contains("workspaces") {
                workspaces::USAGE
            } else {
                USAGE
            };
            eprintln!("shimmer: {e}\n\n{usage}");
            return ExitCode::from(2);
        }
    };
    match execute(&args).await {
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

async fn execute(args: &Args) -> Result<Option<String>> {
    match &args.command {
        Command::Help => return Ok(Some(USAGE.into())),
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
