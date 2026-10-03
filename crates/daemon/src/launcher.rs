//! The real `LaunchBackend` (ADR 0010). Lands incrementally, one `SpawnMode`/concern per
//! sub-branch of the `m3-launch-backend` umbrella; this slice is `SpawnMode::Detached` only.
//!
//! Linux/macOS (`cfg(unix)`) only, same scoping as ADR 0010 §7 — Windows is sketched there,
//! not implemented. Not yet wired into any `Ctx` (that is the capability-scoped-wiring
//! sub-branch); this module is exercised only by its own tests until then.

// Unwired until the capability-scoped-wiring sub-branch has `Core::new` construct this
// behind the module's `"process"` capability — remove once it does.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Stdio;

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
            SpawnMode::Supervised { .. } => {
                // Lands in the next sub-branch of this umbrella (ADR 0010 §3 "Supervised").
                let _ = cancel;
                Err(Error::internal("supervised spawn mode not implemented yet"))
            }
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

    #[tokio::test]
    async fn supervised_mode_is_not_implemented_yet() {
        let home = TempDir::new().unwrap();
        let backend = RealLaunchBackend::new(home.path().to_path_buf());
        let mut s = step("deep-work", "setup");
        s.mode = SpawnMode::Supervised { timeout: Duration::from_secs(1) };
        let e = backend.run(&s, &CancellationToken::new()).await.unwrap_err();
        assert_eq!(e.code, shimmer_core::ErrorCode::Internal);
    }
}
