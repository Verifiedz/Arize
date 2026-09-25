//! M0's definition of done (CLAUDE.md §14): a `swe` command reaches the daemon and back.
//! Runs the real binary as a user would, in a throwaway `$SWE_HOME`, starting from no daemon.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use tempfile::TempDir;

struct Home {
    dir: TempDir,
}

impl Home {
    fn new() -> Self {
        Self { dir: TempDir::new().unwrap() }
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("d.sock")
    }

    fn swe(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_swe"))
            .args(args)
            .env("SWE_HOME", self.dir.path().join("home"))
            .env("SWE_SOCKET", self.socket())
            .output()
            .unwrap()
    }
}

/// Never leave a daemon running behind a failed assertion.
impl Drop for Home {
    fn drop(&mut self) {
        if self.socket().exists() {
            let _ = self.swe(&["shutdown"]);
        }
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn wait_gone(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!path.exists(), "daemon did not remove its socket after shutdown");
}

#[test]
fn a_command_reaches_the_daemon_and_back() {
    let home = Home::new();
    assert!(!home.socket().exists());

    // No daemon yet: the CLI starts one and answers.
    let o = home.swe(&["ping"]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));
    assert!(stdout(&o).starts_with("pong (daemon up "), "{}", stdout(&o));
    assert!(home.socket().exists());

    // The same daemon answers the next command.
    let o = home.swe(&["manifest", "--json"]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));
    let m: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(m["protocol"], 1);
    assert!(m["lanes"].as_array().unwrap().iter().any(|l| l["id"] == "default"));

    // Daemon errors come back with their stable code and exit 1.
    let o = home.swe(&["call", "nope.nothing"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).starts_with("swe: unknown_op: "), "{}", stderr(&o));

    let o = home.swe(&["shutdown"]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "daemon stopped");
    wait_gone(&home.socket());

    // Stopping a stopped daemon does not start one.
    let o = home.swe(&["shutdown"]);
    assert!(o.status.success());
    assert!(stderr(&o).contains("not running"));
    assert!(!home.socket().exists());
}

#[test]
fn an_explicit_socket_never_auto_starts() {
    let home = Home::new();
    let elsewhere = home.dir.path().join("elsewhere.sock");
    let o = home.swe(&["ping", "--socket", elsewhere.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("unavailable: cannot connect"), "{}", stderr(&o));
    assert!(!elsewhere.exists());
    assert!(!home.socket().exists());
}

#[test]
fn bad_usage_exits_2_with_help() {
    let home = Home::new();
    let o = home.swe(&["frobnicate"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("unknown command 'frobnicate'"));
    assert!(stderr(&o).contains("usage: swe"));
    assert!(!home.socket().exists(), "bad usage must not start a daemon");
}
