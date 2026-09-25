//! The one binary. Today it dispatches to the daemon and the mock daemon; CLI and TUI dispatch
//! arrives with their crates. Clients hold no business logic (§2), so nothing else belongs here.

use std::process::ExitCode;

use swe_daemon::{Daemon, DaemonConfig};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: swe daemon    run the daemon in the foreground (Ctrl-C to stop)
       swe mockd     run the mock daemon for building clients (swe mockd --help)";

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();

    match std::env::args().nth(1).as_deref() {
        Some("daemon") => run_daemon().await,
        Some("mockd") => run_mockd(std::env::args().skip(2).collect()).await,
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

async fn run_daemon() -> ExitCode {
    // Modules are registered here by the app as they land; none exist yet.
    let daemon = match Daemon::start(DaemonConfig::from_env(), Vec::new()).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("swe: cannot start daemon: {e}");
            return ExitCode::FAILURE;
        }
    };
    let handle = daemon.shutdown_handle();
    tokio::spawn(async move {
        wait_for_signal().await;
        handle.shutdown();
    });
    // Also returns when a `core.shutdown` request stops the daemon.
    daemon.wait().await;
    ExitCode::SUCCESS
}

async fn run_mockd(args: Vec<String>) -> ExitCode {
    let options = match swe_mockd::Command::parse(args) {
        Ok(swe_mockd::Command::Run(options)) => options,
        Ok(swe_mockd::Command::Help) => {
            println!("{}", swe_mockd::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("swe mockd: {e}\n\n{}", swe_mockd::USAGE);
            return ExitCode::from(2);
        }
    };
    let mockd = match swe_mockd::Mockd::start(options).await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("swe mockd: {e}");
            return ExitCode::FAILURE;
        }
    };
    let handle = mockd.shutdown_handle();
    tokio::spawn(async move {
        wait_for_signal().await;
        handle.shutdown();
    });
    // Also returns when a `core.shutdown` request stops it.
    mockd.wait().await;
    ExitCode::SUCCESS
}

async fn wait_for_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
        return std::future::pending().await;
    };
    tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
}
