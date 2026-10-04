//! Bare commands (ADR 0013 §10): a symlink per alias in the link folder (`~/.local/bin` by
//! default), pointing at the `shimmer` binary, so `ikuzo deep-work` works on its own. The binary
//! sees the name it was started under and runs that alias.
//!
//! This touches a folder that also holds the user's own programs, so it is careful:
//! - it never overwrites anything, and never makes a link that would hide a real command;
//! - it only removes a name it made (recorded in `cli.toml` `linked`) and only while that name is
//!   still a symlink, so a real file is never deleted.

use std::path::{Path, PathBuf};

use super::COMMON_TOOLS;

/// Where links go, what they point at, and the `PATH` to check names against.
pub struct Linker<'a> {
    pub dir: &'a Path,
    /// The real `shimmer` binary (symlinks resolved).
    pub exe: &'a Path,
    pub path_dirs: &'a [PathBuf],
}

/// What [`Linker::sync`] did.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// Every link Shimmer now owns in the folder, sorted: save this as `cli.toml` `linked`.
    pub linked: Vec<String>,
    /// Links made this time.
    pub added: Vec<String>,
    /// Links removed this time.
    pub removed: Vec<String>,
    /// Aliases not linked, and why. They still work as `shimmer <alias>`.
    pub skipped: Vec<(String, String)>,
    /// Things that went wrong, and why.
    pub failed: Vec<(String, String)>,
    /// Whether the folder is on `PATH`; if not, the links exist but the shell won't find them.
    pub dir_on_path: bool,
}

impl Linker<'_> {
    /// Make the links in the folder exactly `want`, minus anything unsafe. `had` are the links
    /// made before (`cli.toml` `linked`): the only names this may remove.
    pub fn sync(&self, want: &[String], had: &[String]) -> Report {
        let mut r = Report { dir_on_path: self.path_dirs.iter().any(|d| same_dir(d, self.dir)), ..Report::default() };

        for name in had.iter().filter(|n| !want.contains(n)) {
            match remove_own_link(&self.dir.join(name)) {
                Ok(true) => r.removed.push(name.clone()),
                // Already gone, or replaced by a real file: not ours any more, so let it be.
                Ok(false) => {}
                Err(e) => {
                    r.failed.push((name.clone(), format!("couldn't remove the old link: {e}")));
                    r.linked.push(name.clone()); // still ours; try again next time
                }
            }
        }

        for name in want {
            if COMMON_TOOLS.contains(&name.as_str()) {
                r.skipped.push((name.clone(), "is a common command name".into()));
                continue;
            }
            let path = self.dir.join(name);
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                let ours = had.contains(name) && meta.file_type().is_symlink();
                if !ours {
                    r.skipped.push((name.clone(), format!("already exists in {}", self.dir.display())));
                    continue;
                }
                if std::fs::read_link(&path).is_ok_and(|target| target == self.exe) {
                    r.linked.push(name.clone()); // already right
                    continue;
                }
                // Ours, but pointing at an old binary: replace it.
                if let Err(e) = std::fs::remove_file(&path) {
                    r.failed.push((name.clone(), format!("couldn't replace the old link: {e}")));
                    r.linked.push(name.clone());
                    continue;
                }
            }
            if let Some(other) = self.elsewhere_on_path(name) {
                r.skipped.push((name.clone(), format!("is already a command ({})", other.display())));
                continue;
            }
            match std::fs::create_dir_all(self.dir).and_then(|()| make_link(self.exe, &path)) {
                Ok(()) => {
                    r.added.push(name.clone());
                    r.linked.push(name.clone());
                }
                Err(e) => r.failed.push((name.clone(), format!("couldn't link: {e}"))),
            }
        }
        r.linked.sort();
        r.linked.dedup();
        r
    }

    /// Put the folder back the way it was before [`Linker::sync`] made `r`: remove the links it
    /// added and re-create the ones it removed. For when the switch can't be recorded in
    /// `cli.toml`, so that a failed switch changes nothing (#70 review). Best effort: a step that
    /// fails here leaves that name as `sync` left it, which the next `packs use` or `link` tidies.
    pub fn undo(&self, r: &Report) {
        for name in &r.added {
            let _ = remove_own_link(&self.dir.join(name));
        }
        for name in &r.removed {
            let _ = make_link(self.exe, &self.dir.join(name));
        }
    }

    /// A program called `name` in another `PATH` folder, which a link would hide or be hidden by.
    fn elsewhere_on_path(&self, name: &str) -> Option<PathBuf> {
        self.path_dirs
            .iter()
            .filter(|d| !same_dir(d, self.dir))
            .map(|d| d.join(name))
            .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file()))
    }
}

/// Remove `path` only if it is a symlink. `Ok(false)` if there's nothing there or it isn't a
/// link (a real file is never deleted).
fn remove_own_link(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::remove_file(path).map(|()| true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(unix)]
fn make_link(exe: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(exe, link)
}

#[cfg(not(unix))]
fn make_link(_exe: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "bare commands aren't supported on this platform yet"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    struct Setup {
        _tmp: tempfile::TempDir,
        dir: PathBuf,
        exe: PathBuf,
        other: PathBuf,
    }

    /// A link folder, a fake shimmer binary, and another PATH folder holding a program `taken`.
    fn setup() -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let (dir, other) = (root.join("bin"), root.join("other"));
        std::fs::create_dir_all(&other).unwrap();
        let exe = root.join("shimmer");
        std::fs::write(&exe, "binary").unwrap();
        std::fs::write(other.join("taken"), "someone else's program").unwrap();
        Setup { _tmp: tmp, dir, exe, other }
    }

    fn names(s: &[&str]) -> Vec<String> {
        s.iter().map(|n| n.to_string()).collect()
    }

    fn sync(s: &Setup, want: &[&str], had: &[&str]) -> Report {
        let path_dirs = [s.dir.clone(), s.other.clone()];
        Linker { dir: &s.dir, exe: &s.exe, path_dirs: &path_dirs }.sync(&names(want), &names(had))
    }

    fn link_target(s: &Setup, name: &str) -> PathBuf {
        std::fs::read_link(s.dir.join(name)).unwrap()
    }

    #[test]
    fn links_point_at_the_binary_and_the_folder_is_created() {
        let s = setup();
        let r = sync(&s, &["ikuzo", "nani"], &[]);
        assert_eq!((r.linked.clone(), r.added.clone()), (names(&["ikuzo", "nani"]), names(&["ikuzo", "nani"])));
        assert!(r.dir_on_path && r.skipped.is_empty() && r.failed.is_empty(), "{r:?}");
        assert_eq!(link_target(&s, "ikuzo"), s.exe);
    }

    #[test]
    fn switching_removes_only_the_old_links_and_keeps_shared_ones() {
        let s = setup();
        sync(&s, &["a", "b"], &[]);
        let r = sync(&s, &["b", "c"], &["a", "b"]);
        assert_eq!((r.removed, r.added, r.linked), (names(&["a"]), names(&["c"]), names(&["b", "c"])));
        assert!(!s.dir.join("a").exists());
    }

    #[test]
    fn nothing_is_ever_overwritten_or_hidden() {
        let s = setup();
        std::fs::create_dir_all(&s.dir).unwrap();
        std::fs::write(s.dir.join("mine"), "user's own script").unwrap();
        let r = sync(&s, &["mine", "taken", "git", "ok"], &[]);
        assert_eq!(r.linked, names(&["ok"]));
        let why: Vec<(&str, &str)> = r.skipped.iter().map(|(n, w)| (n.as_str(), w.as_str())).collect();
        assert!(why[0].0 == "mine" && why[0].1.starts_with("already exists in"), "{why:?}");
        assert!(why[1].0 == "taken" && why[1].1.starts_with("is already a command"), "{why:?}");
        assert_eq!(why[2], ("git", "is a common command name"));
        assert_eq!(std::fs::read_to_string(s.dir.join("mine")).unwrap(), "user's own script");
    }

    #[test]
    fn only_its_own_symlinks_are_ever_removed() {
        let s = setup();
        sync(&s, &["a", "b"], &[]);
        // The user replaced `a` with a real file, and made a symlink `theirs` themselves.
        std::fs::remove_file(s.dir.join("a")).unwrap();
        std::fs::write(s.dir.join("a"), "real file now").unwrap();
        std::os::unix::fs::symlink(&s.exe, s.dir.join("theirs")).unwrap();

        let r = sync(&s, &[], &["a", "b"]);
        assert_eq!((r.removed, r.linked), (names(&["b"]), vec![]));
        assert_eq!(std::fs::read_to_string(s.dir.join("a")).unwrap(), "real file now");
        assert!(s.dir.join("theirs").exists(), "never recorded as ours, never touched");
    }

    #[test]
    fn a_link_to_an_old_binary_is_replaced_and_a_right_one_is_kept() {
        let s = setup();
        std::fs::create_dir_all(&s.dir).unwrap();
        std::os::unix::fs::symlink("/old/place/shimmer", s.dir.join("up")).unwrap();
        sync(&s, &["down"], &[]);
        let r = sync(&s, &["up", "down"], &["up", "down"]);
        assert_eq!((r.added, r.linked), (names(&["up"]), names(&["down", "up"])));
        assert_eq!(link_target(&s, "up"), s.exe);
    }

    #[test]
    fn undo_puts_the_folder_back_exactly() {
        let s = setup();
        sync(&s, &["a", "b"], &[]);
        let r = sync(&s, &["b", "c"], &["a", "b"]);
        assert_eq!((r.removed.clone(), r.added.clone()), (names(&["a"]), names(&["c"])));
        let path_dirs = [s.dir.clone(), s.other.clone()];
        Linker { dir: &s.dir, exe: &s.exe, path_dirs: &path_dirs }.undo(&r);
        let mut now: Vec<String> =
            std::fs::read_dir(&s.dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        now.sort();
        assert_eq!(now, names(&["a", "b"]), "a is back, c is gone, b untouched");
        assert_eq!(link_target(&s, "a"), s.exe);
    }

    #[test]
    fn a_folder_off_path_is_reported() {
        let s = setup();
        let path_dirs = [s.other.clone()];
        let r = Linker { dir: &s.dir, exe: &s.exe, path_dirs: &path_dirs }.sync(&names(&["x"]), &[]);
        assert!(!r.dir_on_path);
        assert_eq!(r.linked, names(&["x"]), "still linked; the user just needs PATH");
    }

    #[test]
    fn an_unwritable_folder_fails_cleanly_and_records_nothing() {
        let s = setup();
        std::fs::write(&s.dir, "a file where the folder should be").unwrap();
        let r = sync(&s, &["x"], &[]);
        assert!(r.linked.is_empty() && r.failed.len() == 1, "{r:?}");
    }
}
