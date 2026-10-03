//! The real `LaunchBackend` (ADR 0010). Lands incrementally, one `SpawnMode`/concern per
//! sub-branch of the `m3-launch-backend` umbrella; this slice adds `SpawnMode::Supervised`
//! on top of the previous one's `Detached`.
//!
//! Linux/macOS (`cfg(unix)`) only, same scoping as ADR 0010 §7 — Windows is sketched there,
//! not implemented. Not yet wired into any `Ctx` (that is the capability-scoped-wiring
//! sub-branch); this module is exercised only by its own tests until then.

// Unwired until the capability-scoped-wiring sub-branch has `Core::new` construct this
// behind the module's `"process"` capability — remove once it does.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use shimmer_core::launcher::env_names;
use shimmer_core::store::validate_path;
use shimmer_core::{Error, LaunchBackend, LaunchStep, Result, SpawnMode, Step, StepOutcome};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

/// Implements [`LaunchBackend`] for real, spawning scripts under
/// `$SHIMMER_HOME/data/workspaces/<workspace_dir>/`.
pub struct RealLaunchBackend {
    /// `$SHIMMER_HOME`. The backend is the one place allowed to hold this as a raw path
    /// (ADR 0010 §4) — nothing upstream of it ever sees one.
    home: PathBuf,
}

impl RealLaunchBackend {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }
}

#[async_trait]
impl LaunchBackend for RealLaunchBackend {
    async fn run(&self, step: &LaunchStep, cancel: &CancellationToken) -> Result<StepOutcome> {
        let script = resolve_script(&self.home, &step.workspace_dir, &step.step)?;
        match step.mode {
            SpawnMode::Detached => run_detached(&script, step).await,
            SpawnMode::Supervised { timeout } => run_supervised(&self.home, &script, step, timeout, cancel).await,
        }
    }
}

/// `Step` -> a real path under the workspace's own directory, never outside it (ADR 0010
/// §2a/§4). Only escape-safety is checked here (`validate_path`, already applied to every
/// other module's paths); the stricter `[a-z0-9][a-z0-9_-]*` charset on `name` lands in the
/// step-name-validation sub-branch (closes #14).
fn resolve_script(home: &Path, workspace_dir: &str, step: &Step) -> Result<PathBuf> {
    validate_path(workspace_dir)?;
    let rel = match step {
        Step::Launch { index, name, .. } => {
            validate_path(name)?;
            format!("{index:02}-{name}.sh")
        }
        Step::Cleanup => "cleanup.sh".to_string(),
    };
    let rel = if matches!(step, Step::Cleanup) { rel } else { format!("steps/{rel}") };
    Ok(home.join("data/workspaces").join(workspace_dir).join(rel))
}

/// ADR 0010 §3 "Detached": own process group, stdio to null (same pattern
/// `crates/cli/src/autostart.rs` already uses for the daemon autostart spawn), `run` returns
/// as soon as the process is spawned, and a second task reaps it independently so it never
/// becomes a zombie — without ever retaining a handle (§10.2: "no handle retained").
async fn run_detached(script: &Path, step: &LaunchStep) -> Result<StepOutcome> {
    if !script.exists() {
        return Err(Error::unavailable(format!("launch script not found: {}", script.display())));
    }
    let mut cmd = Command::new("sh");
    cmd.arg(script);
    inject_env(&mut cmd, step);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(|e| Error::unavailable(format!("cannot spawn {}: {e}", script.display())))?;
    // Reaping only — no store write, no event, no module state (ADR 0010 §3). Dropping the
    // JoinHandle is deliberate: holding it would be the retained handle §10.2 forbids.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(StepOutcome {
        // Placeholder — minted for real from `Ctx`'s clock in the session-id sub-branch.
        session_id: String::new(),
        exit_code: None,
        timed_out: false,
        // Detached stdio goes to null (above), so there is nothing to capture.
        log_path: String::new(),
    })
}

/// ADR 0010 §3 "Supervised": same process-group mechanism as `Detached`, but the handle is
/// retained and raced against a timeout and the caller's cancellation — both converge on the
/// same process-group kill (§9: "both paths converge on the same kill"). Unlike `Detached`,
/// whose stdio goes to null, this captures the child's stdout/stderr to `logs/` (§3 "Logs").
async fn run_supervised(
    home: &Path,
    script: &Path,
    step: &LaunchStep,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<StepOutcome> {
    if !script.exists() {
        return Err(Error::unavailable(format!("launch script not found: {}", script.display())));
    }
    let log_rel = log_path_for(step);
    let log_abs = home.join(&log_rel);
    if let Some(parent) = log_abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stdout_file = std::fs::File::create(&log_abs)?;
    let stderr_file = stdout_file.try_clone()?;

    let mut cmd = Command::new("sh");
    cmd.arg(script);
    inject_env(&mut cmd, step);
    cmd.stdin(Stdio::null()).stdout(Stdio::from(stdout_file)).stderr(Stdio::from(stderr_file));
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(|e| Error::unavailable(format!("cannot spawn {}: {e}", script.display())))?;
    let pid = child.id();

    let (exit_code, timed_out) = tokio::select! {
        status = child.wait() => {
            let status = status.map_err(|e| Error::unavailable(format!("waiting on {}: {e}", script.display())))?;
            (status.code(), false)
        }
        _ = tokio::time::sleep(timeout) => {
            kill_process_group(pid);
            let _ = child.wait().await;
            (None, true)
        }
        _ = cancel.cancelled() => {
            kill_process_group(pid);
            let _ = child.wait().await;
            (None, false)
        }
    };

    Ok(StepOutcome {
        // Placeholder — minted for real from `Ctx`'s clock in the session-id sub-branch.
        session_id: String::new(),
        exit_code,
        timed_out,
        log_path: log_rel,
    })
}

/// Relative to `$SHIMMER_HOME`, matching `docs/protocol.md`'s `workspace_dirty` detail shape
/// (ADR 0010 §3 "Logs"). Provisional naming — folds in the real session id once the
/// session-id sub-branch mints one; nothing downstream depends on this exact shape yet.
fn log_path_for(step: &LaunchStep) -> String {
    let label = match &step.step {
        Step::Launch { name, .. } => name.as_str(),
        Step::Cleanup => "cleanup",
    };
    format!("logs/{}-{}.log", step.workspace_dir, label)
}

/// Negative PID targets the whole process group (ADR 0010 §3), so a script's own children —
/// e.g. a build tool's compiler subprocesses — die with it, not just the immediate child.
/// A no-op if the process already exited and its id could not be read.
fn kill_process_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        let pgid = nix::unistd::Pid::from_raw(-(pid as i32));
        let _ = nix::sys::signal::kill(pgid, nix::sys::signal::Signal::SIGKILL);
    }
}

/// The five vars every spawned script gets (CLAUDE.md §10.1). `step.user_env` is not merged
/// in yet — that, and the override guard, land in the env-injection sub-branch.
fn inject_env(cmd: &mut Command, step: &LaunchStep) {
    cmd.env(env_names::WORKSPACE_ID, &step.workspace_id);
    cmd.env(env_names::WORKSPACE_DIR, &step.workspace_dir);
    cmd.env(env_names::PLATFORM, platform());
}

fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;

    fn write_step_script(home: &Path, workspace_dir: &str, name: &str, body: &str) -> PathBuf {
        let dir = home.join("data/workspaces").join(workspace_dir).join("steps");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("01-{name}.sh"));
        std::fs::write(&path, body).unwrap();
        path
    }

    fn step(workspace_dir: &str, name: &str) -> LaunchStep {
        LaunchStep {
            workspace_id: "deep-work".into(),
            workspace_dir: workspace_dir.into(),
            step: Step::Launch { index: 1, count: 1, name: name.into() },
            mode: SpawnMode::Detached,
            user_env: vec![],
        }
    }

    #[tokio::test]
    async fn detached_script_outlives_the_backend_that_spawned_it() {
        let home = TempDir::new().unwrap();
        let marker = home.path().join("marker");
        write_step_script(home.path(), "deep-work", "setup", &format!("sleep 0.3 && touch {}\n", marker.display()));

        {
            let backend = RealLaunchBackend::new(home.path().to_path_buf());
            let outcome = backend.run(&step("deep-work", "setup"), &CancellationToken::new()).await.unwrap();
            assert_eq!(outcome.exit_code, None);
            assert!(!outcome.timed_out);
            // `backend` (and the `Launcher`/`LaunchBackend` value in general) drops here —
            // ADR 0010 §10: the process must still be running afterwards.
        }

        assert!(!marker.exists(), "script should not have finished yet");
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(marker.exists(), "detached process did not outlive the backend that spawned it");
    }

    #[tokio::test]
    async fn injects_workspace_id_dir_and_platform() {
        let home = TempDir::new().unwrap();
        let out = home.path().join("out");
        write_step_script(
            home.path(),
            "deep-work",
            "setup",
            &format!("echo \"$SHIMMER_WORKSPACE_ID $SHIMMER_WORKSPACE_DIR $SHIMMER_PLATFORM\" > {}\n", out.display()),
        );

        let backend = RealLaunchBackend::new(home.path().to_path_buf());
        backend.run(&step("deep-work", "setup"), &CancellationToken::new()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let seen = std::fs::read_to_string(&out).unwrap();
        assert_eq!(seen.trim(), format!("deep-work deep-work {}", platform()));
    }

    #[tokio::test]
    async fn missing_script_fails_closed_without_spawning() {
        let home = TempDir::new().unwrap();
        let backend = RealLaunchBackend::new(home.path().to_path_buf());
        let e = backend.run(&step("deep-work", "nope"), &CancellationToken::new()).await.unwrap_err();
        assert_eq!(e.code, shimmer_core::ErrorCode::Unavailable);
    }

    #[test]
    fn resolve_script_stays_inside_the_workspace_dir() {
        let home = Path::new("/home/shimmer");
        let launch =
            resolve_script(home, "deep-work", &Step::Launch { index: 2, count: 3, name: "editor".into() }).unwrap();
        assert_eq!(launch, home.join("data/workspaces/deep-work/steps/02-editor.sh"));

        let cleanup = resolve_script(home, "deep-work", &Step::Cleanup).unwrap();
        assert_eq!(cleanup, home.join("data/workspaces/deep-work/cleanup.sh"));

        assert!(resolve_script(home, "../escape", &Step::Cleanup).is_err());
        assert!(resolve_script(home, "deep-work", &Step::Launch { index: 1, count: 1, name: "../x".into() }).is_err());
    }

    fn supervised_step(workspace_dir: &str, name: &str, timeout: Duration) -> LaunchStep {
        let mut s = step(workspace_dir, name);
        s.mode = SpawnMode::Supervised { timeout };
        s
    }

    #[tokio::test]
    async fn supervised_step_waits_for_exit_and_captures_stdout_verbatim() {
        let home = TempDir::new().unwrap();
        write_step_script(home.path(), "deep-work", "setup", "echo hello-from-setup\n");

        let backend = RealLaunchBackend::new(home.path().to_path_buf());
        let outcome = backend
            .run(&supervised_step("deep-work", "setup", Duration::from_secs(5)), &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(outcome.exit_code, Some(0));
        assert!(!outcome.timed_out);
        let log = std::fs::read_to_string(home.path().join(&outcome.log_path)).unwrap();
        assert_eq!(log, "hello-from-setup\n");
    }

    #[tokio::test]
    async fn timeout_kills_the_whole_process_group_not_just_the_immediate_child() {
        let home = TempDir::new().unwrap();
        let grandchild_marker = home.path().join("grandchild_marker");
        // The backgrounded subshell inherits the script's process group (no setsid of its
        // own), so a process-group kill must take it down too — proving §10's "kills the
        // grandchild too (proves process-group kill, not just the immediate child)".
        write_step_script(
            home.path(),
            "deep-work",
            "setup",
            &format!("(sleep 1 && touch {}) &\nsleep 5\n", grandchild_marker.display()),
        );

        let backend = RealLaunchBackend::new(home.path().to_path_buf());
        let outcome = backend
            .run(&supervised_step("deep-work", "setup", Duration::from_millis(200)), &CancellationToken::new())
            .await
            .unwrap();

        assert!(outcome.timed_out);
        assert_eq!(outcome.exit_code, None);
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!grandchild_marker.exists(), "grandchild survived the process-group kill");
    }

    #[tokio::test]
    async fn cancellation_kills_the_group_and_is_not_reported_as_timed_out() {
        let home = TempDir::new().unwrap();
        let grandchild_marker = home.path().join("grandchild_marker");
        write_step_script(
            home.path(),
            "deep-work",
            "setup",
            &format!("(sleep 1 && touch {}) &\nsleep 5\n", grandchild_marker.display()),
        );

        let backend = RealLaunchBackend::new(home.path().to_path_buf());
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel_clone.cancel();
        });
        let outcome =
            backend.run(&supervised_step("deep-work", "setup", Duration::from_secs(5)), &cancel).await.unwrap();

        assert!(!outcome.timed_out, "a cancelled step is not timed out (callers distinguish via cancel)");
        assert_eq!(outcome.exit_code, None);
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!grandchild_marker.exists(), "grandchild survived the process-group kill");
    }
}
