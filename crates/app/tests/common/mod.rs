//! Shared harness for the end-to-end tests: the real `swe` binary in a throwaway `$SWE_HOME`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use tempfile::TempDir;

pub struct Home {
    pub dir: TempDir,
}

impl Home {
    pub fn new() -> Self {
        Self { dir: TempDir::new().unwrap() }
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.path().join("d.sock")
    }

    pub fn swe(&self, args: &[&str]) -> Output {
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

pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

pub fn wait_gone(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!path.exists(), "daemon did not remove its socket after shutdown");
}
