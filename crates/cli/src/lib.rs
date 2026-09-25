//! `swe-cli`: the command-line client. A thin client (CLAUDE.md §2): it parses a command,
//! sends one request over the socket, and renders the answer. No business logic lives here.
//!
//! Depends on `core` and `proto` only (§3 rule 2). Command packs and aliases (§15.1) land
//! here later; they rewrite the command word and never reach the wire.

mod args;
mod autostart;
mod client;
mod render;

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{json, Value};
use swe_core::{Error, Result};
use swe_proto::ops;

pub use args::{Args, Command, USAGE};
pub use client::{Client, ConnectError};

/// Run `swe <args>`: `args` are what follows the binary name. Exit codes: 0 success,
/// 1 the daemon (or reaching it) failed, 2 bad usage.
pub async fn run(args: Vec<String>) -> ExitCode {
    let args = match Args::parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("swe: {e}\n\n{USAGE}");
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
            eprintln!("swe: {}", render::error(&e));
            ExitCode::FAILURE
        }
    }
}

async fn execute(args: &Args) -> Result<Option<String>> {
    let (op, params) = match &args.command {
        Command::Help => return Ok(Some(USAGE.into())),
        Command::Ping => (ops::CORE_PING, json!({})),
        Command::Manifest => (ops::CORE_MANIFEST, json!({})),
        Command::Shutdown => (ops::CORE_SHUTDOWN, json!({})),
        Command::Call { op, params } => (op.as_str(), params.clone()),
    };

    let socket = args.socket.clone().unwrap_or_else(swe_proto::paths::socket_path);
    // Starting a daemon only to stop it again helps nobody.
    let autostart = args.socket.is_none() && args.command != Command::Shutdown;
    let mut client = match Client::connect(&socket).await {
        Ok(c) => c,
        Err(e) if e.nobody_listening() && args.command == Command::Shutdown => {
            eprintln!("swe: the daemon is not running");
            return Ok(None);
        }
        Err(e) if e.nobody_listening() && autostart => autostart::start_and_connect(&daemon_exe()?, &socket).await?,
        Err(e) => return Err(e.into_error(&socket)),
    };

    let data = client.call(op, params).await?;
    Ok(Some(output(args, &data)))
}

fn output(args: &Args, data: &Value) -> String {
    if args.json {
        return render::json(data);
    }
    match args.command {
        Command::Ping => render::ping(data),
        Command::Manifest => render::manifest(data),
        Command::Shutdown => render::shutdown(),
        Command::Call { .. } | Command::Help => render::json(data),
    }
}

/// The daemon is this same binary, run as `swe daemon`.
fn daemon_exe() -> Result<PathBuf> {
    std::env::current_exe()
        .map_err(|e| Error::unavailable(format!("cannot find the swe binary to start the daemon: {e}")))
}
