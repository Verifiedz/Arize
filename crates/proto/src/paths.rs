//! Where things live on disk (CLAUDE.md §7, protocol.md "Transport"). Pure path arithmetic
//! from the environment; touches no files.

use std::env;
use std::path::PathBuf;

/// `$SHIMMER_HOME`, else the platform data dir.
pub fn shimmer_home() -> PathBuf {
    if let Some(p) = env::var_os("SHIMMER_HOME").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "macos") {
        return home.join("Library/Application Support/shimmer");
    }
    match env::var_os("XDG_DATA_HOME").filter(|p| !p.is_empty()) {
        Some(x) => PathBuf::from(x).join("shimmer"),
        None => home.join(".local/share/shimmer"),
    }
}

/// `$SHIMMER_SOCKET` (what workspace scripts see) wins. Otherwise
/// `$XDG_RUNTIME_DIR/shimmer/daemon.sock` on Linux, falling back to `$SHIMMER_HOME/daemon.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = env::var_os("SHIMMER_SOCKET").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    if cfg!(target_os = "linux") {
        if let Some(rt) = env::var_os("XDG_RUNTIME_DIR").filter(|p| !p.is_empty()) {
            return PathBuf::from(rt).join("shimmer/daemon.sock");
        }
    }
    shimmer_home().join("daemon.sock")
}
