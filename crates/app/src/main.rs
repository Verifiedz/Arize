//! The one binary. `swe daemon` and `swe mockd` run servers; every other command belongs to the
//! CLI. TUI dispatch arrives with its crate. Clients hold no business logic (§2), so nothing
//! else belongs here.

use std::process::ExitCode;
use std::sync::Arc;

use swe_core::Module;
use swe_daemon::{Daemon, DaemonConfig};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();

    match std::env::args().nth(1).as_deref() {
        Some("daemon") => run_daemon().await,
        Some("mockd") => run_mockd(std::env::args().skip(2).collect()).await,
        _ => swe_cli::run(std::env::args().skip(1).collect()).await,
    }
}

async fn run_daemon() -> ExitCode {
    // Every module the daemon runs. A new module is one more line here.
    let modules: Vec<Arc<dyn Module>> = vec![Arc::new(swe_records::Records::default())];
    let daemon = match Daemon::start(DaemonConfig::from_env(), modules).await {
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
