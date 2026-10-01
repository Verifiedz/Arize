//! The one binary. `shimmer daemon` and `shimmer mockd` run servers; every other command belongs to the
//! CLI. TUI dispatch arrives with its crate. Clients hold no business logic (§2), so nothing
//! else belongs here.

use std::fs::{self, OpenOptions};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use shimmer_core::Module;
use shimmer_daemon::{Daemon, DaemonConfig};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("daemon") => {
            // Autostart (crates/cli/src/autostart.rs) nulls the child's stderr, so this is the
            // only trace of a cold-start crash unless it also lands in a file (ADR 0007).
            let _guard = init_daemon_logging(&shimmer_proto::paths::shimmer_home());
            run_daemon().await
        }
        Some("mockd") => {
            init_stderr_logging();
            run_mockd(std::env::args().skip(2).collect()).await
        }
        _ => {
            init_stderr_logging();
            shimmer_cli::run(std::env::args().skip(1).collect()).await
        }
    }
}

fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

fn init_stderr_logging() {
    tracing_subscriber::fmt().with_env_filter(env_filter()).with_writer(std::io::stderr).init();
}

/// Logs to stderr (visible when run in the foreground) and, best-effort, to
/// `$SHIMMER_HOME/logs/daemon.log` (visible when autostarted, whose stderr is discarded). The
/// returned guard must stay alive for the process lifetime or buffered lines are dropped.
fn init_daemon_logging(home: &Path) -> Option<WorkerGuard> {
    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_filter(env_filter());
    let logs_dir = home.join("logs");
    let file = fs::create_dir_all(&logs_dir)
        .and_then(|_| OpenOptions::new().create(true).append(true).open(logs_dir.join("daemon.log")));
    match file {
        Ok(file) => {
            let (writer, guard) = tracing_appender::non_blocking(file);
            let file_layer =
                tracing_subscriber::fmt::layer().with_ansi(false).with_writer(writer).with_filter(env_filter());
            tracing_subscriber::registry().with(stderr_layer).with(file_layer).init();
            Some(guard)
        }
        Err(e) => {
            tracing_subscriber::registry().with(stderr_layer).init();
            eprintln!(
                "shimmer: cannot open {} ({e}); daemon logs will only go to stderr",
                logs_dir.join("daemon.log").display()
            );
            None
        }
    }
}

async fn run_daemon() -> ExitCode {
    // Every module the daemon runs. A new module is one more line here.
    let modules: Vec<Arc<dyn Module>> = vec![Arc::new(shimmer_records::Records::default())];
    let daemon = match Daemon::start(DaemonConfig::from_env(), modules).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("shimmer: cannot start daemon: {e}");
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
    let options = match shimmer_mockd::Command::parse(args) {
        Ok(shimmer_mockd::Command::Run(options)) => options,
        Ok(shimmer_mockd::Command::Help) => {
            println!("{}", shimmer_mockd::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("shimmer mockd: {e}\n\n{}", shimmer_mockd::USAGE);
            return ExitCode::from(2);
        }
    };
    let mockd = match shimmer_mockd::Mockd::start(options).await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("shimmer mockd: {e}");
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
