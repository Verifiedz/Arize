//! Command packs (CLAUDE.md §15.1, ADR 0013): themed aliases that stand for a whole command, so
//! `ikuzo deep-work` runs `shimmer workspaces activate deep-work`. Everything here lives in the
//! client: an alias is rewritten to canonical command words before anything is parsed or sent,
//! and the daemon never sees one (§12 rule 16).
//!
//! - [`pack`]: a pack's `pack.toml`, and every check a pack must pass (ADR 0013 §1, §6, §7).
//! - [`builtin`]: the five built-in packs (§5).
//! - [`active`]: which pack is active, and the alias rewrite (§3, §4, §8, §9).
//! - [`settings`]: the CLI's own `cli.toml`, which records the active pack (§3).
//! - [`link`]: the links that let an alias run on its own, `ikuzo deep-work` (§10).
//! - [`cmd`]: `shimmer packs list|show|use|link|unlink|check` (§11).

pub mod active;
pub mod builtin;
pub mod cmd;
pub mod link;
pub mod pack;
pub mod settings;

/// Commands people commonly have, or will soon install: shell keywords and builtins, the usual
/// Unix tools, and popular developer tools. A bare alias with one of these names would hide the
/// real command, so `shimmer packs link` never makes one, and no built-in pack uses one (ADR 0013
/// §10). Checked as well as, not instead of, what is actually on `PATH`.
#[rustfmt::skip]
pub const COMMON_TOOLS: &[&str] = &[
    // shell keywords and builtins
    "alias", "bg", "case", "cd", "command", "do", "done", "echo", "elif", "else", "esac", "eval", "exec",
    "exit", "export", "false", "fg", "fi", "for", "function", "history", "if", "jobs", "kill", "pwd", "read",
    "return", "select", "set", "source", "test", "then", "time", "true", "type", "ulimit", "umask", "unalias",
    "unset", "until", "wait", "while",
    // everyday Unix tools
    "awk", "cat", "chmod", "chown", "clear", "cp", "curl", "cut", "date", "df", "diff", "du", "env", "file",
    "find", "free", "grep", "gzip", "halt", "head", "htop", "less", "ln", "ls", "make", "man", "mkdir",
    "more", "mv", "nano", "open", "patch", "ps", "reboot", "rm", "rmdir", "rsync", "scp", "sed", "sleep",
    "sort", "ssh", "su", "sudo", "tail", "tar", "tee", "timeout", "top", "touch", "tr", "tree", "uniq",
    "unzip", "vi", "w", "watch", "wc", "wget", "which", "who", "whoami", "xargs", "yes", "zip",
    // developer tools
    "apt", "aws", "bat", "brew", "bun", "cargo", "cc", "clang", "cmake", "code", "deno", "dnf", "docker",
    "doas", "emacs", "exa", "eza", "fd", "flatpak", "fzf", "g++", "gcc", "gcloud", "gem", "gh", "git", "go",
    "gradle", "helm", "java", "javac", "journalctl", "jq", "kubectl", "mvn", "node", "npm", "npx", "nvim",
    "pacman", "pip", "pip3", "pipx", "pnpm", "podman", "python", "python3", "rg", "rls", "ruby", "rustc",
    "rustup", "snap", "systemctl", "terraform", "tmux", "screen", "vim", "yarn", "yay", "yq", "z", "zoxide",
];
