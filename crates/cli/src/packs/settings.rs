//! The CLI's own settings file, `cli.toml` (ADR 0013 §3): which pack is active, where bare
//! commands are linked, and which links Shimmer made. It lives outside `$SHIMMER_HOME`, which
//! belongs to the daemon (CLAUDE.md §1.6, §7), and only the CLI reads or writes it.
//!
//! A broken `cli.toml` must never break a command, so problems here are reported and the caller
//! carries on with no pack (ADR 0013 §8). An unknown key is only a warning, so a file written by
//! a newer Shimmer still loads in an older one.

use std::ffi::OsString;
use std::path::PathBuf;

use toml::{Table, Value};

use super::pack::{name_problem, NO_PACK};

/// `cli.toml` larger than this is rejected unread.
pub const MAX_CLI_TOML: usize = 16 * 1024;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Settings {
    /// The active pack's id. `None`: no pack.
    pub pack: Option<String>,
    /// Where bare commands are linked. `None`: the default, `~/.local/bin`.
    pub link_dir: Option<PathBuf>,
    /// The links Shimmer made: the only names it will ever remove from `link_dir`.
    pub linked: Vec<String>,
}

/// Settings read from `cli.toml`, plus warnings to show for keys this version doesn't know.
#[derive(Debug, Default, PartialEq)]
pub struct Loaded {
    pub settings: Settings,
    pub warnings: Vec<String>,
}

/// `$XDG_CONFIG_HOME/shimmer/cli.toml`, else `~/.config/shimmer/cli.toml`, on Linux and macOS.
/// `None` when neither variable is set.
pub fn path() -> Option<PathBuf> {
    path_from(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"))
}

fn path_from(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|p| !p.is_empty()) {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(home.filter(|p| !p.is_empty())?).join(".config"),
    };
    Some(base.join("shimmer/cli.toml"))
}

/// Check `cli.toml`'s text. `file` names it in every problem and warning.
pub fn parse(file: &str, text: &str) -> Result<Loaded, Vec<String>> {
    if text.len() > MAX_CLI_TOML {
        return Err(vec![format!("{file}: larger than {} KiB", MAX_CLI_TOML / 1024)]);
    }
    let table: Table = toml::from_str(text).map_err(|e| vec![format!("{file}: not valid TOML: {}", e.message())])?;

    let mut loaded = Loaded::default();
    let mut problems = Vec::new();
    for (key, value) in &table {
        match (key.as_str(), value) {
            ("pack", Value::String(id)) if id == NO_PACK => {}
            ("pack", Value::String(id)) => match name_problem(id) {
                Some(why) => problems.push(format!("{file}: pack '{id}' {why}")),
                None => loaded.settings.pack = Some(id.clone()),
            },
            ("link_dir", Value::String(dir)) => {
                let dir = PathBuf::from(dir);
                if dir.is_absolute() {
                    loaded.settings.link_dir = Some(dir);
                } else {
                    problems.push(format!("{file}: link_dir '{}' must be an absolute path", dir.display()));
                }
            }
            ("linked", Value::Array(names)) => {
                for name in names {
                    // These names are deleted from link_dir later, so each must be one plain
                    // file name: the alias charset rules out '/', '..' and anything odd.
                    match name.as_str() {
                        Some(n) if name_problem(n).is_none() => loaded.settings.linked.push(n.to_string()),
                        _ => problems.push(format!("{file}: linked entry {name} isn't a valid alias name")),
                    }
                }
            }
            ("pack" | "link_dir", _) => problems.push(format!("{file}: {key} must be a string")),
            ("linked", _) => problems.push(format!("{file}: linked must be a list of names")),
            _ => loaded.warnings.push(format!("{file}: unknown key '{key}' ignored")),
        }
    }
    if problems.is_empty() {
        Ok(loaded)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Loaded {
        parse("cli.toml", text).unwrap()
    }

    fn problems(text: &str) -> Vec<String> {
        parse("cli.toml", text).unwrap_err()
    }

    #[test]
    fn a_full_file_parses() {
        let loaded = ok("pack = \"anime-tropes\"\nlink_dir = \"/home/me/bin\"\nlinked = [\"ikuzo\", \"nani\"]\n");
        assert_eq!(
            loaded.settings,
            Settings {
                pack: Some("anime-tropes".into()),
                link_dir: Some("/home/me/bin".into()),
                linked: vec!["ikuzo".into(), "nani".into()],
            }
        );
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn an_empty_file_or_none_means_no_pack() {
        assert_eq!(ok("").settings, Settings::default());
        assert_eq!(ok("pack = \"none\"").settings.pack, None);
    }

    #[test]
    fn unknown_keys_warn_but_still_load() {
        let loaded = ok("pack = \"short\"\ntheme = \"dark\"\n");
        assert_eq!(loaded.settings.pack.as_deref(), Some("short"));
        assert_eq!(loaded.warnings, ["cli.toml: unknown key 'theme' ignored"]);
    }

    #[test]
    fn wrong_values_are_problems() {
        let mut p = problems("pack = 3\nlink_dir = \"bin\"\nlinked = \"ikuzo\"\n");
        p.sort();
        assert_eq!(
            p,
            [
                "cli.toml: link_dir 'bin' must be an absolute path",
                "cli.toml: linked must be a list of names",
                "cli.toml: pack must be a string",
            ]
        );
        assert!(problems("pack = \"Not Valid\"")[0].contains("pack 'Not Valid' must start"));
        assert!(problems("[pack\n")[0].starts_with("cli.toml: not valid TOML"));
        assert!(problems(&"#".repeat(MAX_CLI_TOML + 1))[0].contains("larger than 16 KiB"));
    }

    #[test]
    fn linked_names_can_never_reach_outside_the_link_folder() {
        for bad in ["\"../evil\"", "\"/etc/passwd\"", "\"a/b\"", "\"\"", "3"] {
            let p = problems(&format!("linked = [{bad}]"));
            assert!(p[0].contains("isn't a valid alias name"), "{bad}: {p:?}");
        }
    }

    #[test]
    fn the_file_lives_in_the_config_folder() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(path_from(os("/x"), os("/home/me")), Some("/x/shimmer/cli.toml".into()));
        assert_eq!(path_from(None, os("/home/me")), Some("/home/me/.config/shimmer/cli.toml".into()));
        assert_eq!(path_from(os(""), os("/home/me")), Some("/home/me/.config/shimmer/cli.toml".into()));
        assert_eq!(path_from(None, None), None);
    }
}
