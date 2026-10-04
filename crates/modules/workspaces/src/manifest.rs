//! `workspace.toml` and the `steps/` folder, checked exactly as ADR 0012 §6 says.
//!
//! Pure: [`parse`] takes the folder name, the file's text and the names of the files in the
//! workspace folder, and does no I/O (§12 rule 10). The module reads them through `ctx.store`.
//! Every problem is `invalid_params`, naming the workspace, the file, and the step or field.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::Deserialize;
use shimmer_core::{Error, Result, SpawnMode, Step};

/// File numbers are two digits (ADR 0012 §6).
pub const MAX_STEPS: usize = 99;
/// For workspace ids and step names (ADR 0012 §6).
pub const MAX_NAME_LEN: usize = 64;
/// Catches typos like `300000` (ADR 0012 §6).
pub const MAX_TIMEOUT_S: i64 = 3600;
/// The prefix of every variable Shimmer injects; reserved as a whole (ADR 0012 §6).
pub const RESERVED_ENV_PREFIX: &str = "SHIMMER_";

/// A checked `workspace.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    /// The folder name (ADR 0012 §3).
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    /// In launch order: `steps[0]` is step 1.
    pub steps: Vec<StepSpec>,
    /// `Some` exactly when a cleanup script exists.
    pub cleanup_timeout: Option<Duration>,
    /// `[env]`, in name order. Never contains a `SHIMMER_` name.
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepSpec {
    pub name: String,
    pub description: Option<String>,
    pub mode: SpawnMode,
}

/// `.sh` on Linux and macOS, `.ps1` on Windows (ADR 0012 §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptExt {
    Sh,
    Ps1,
}

impl ScriptExt {
    pub fn this_platform() -> Self {
        if cfg!(windows) {
            Self::Ps1
        } else {
            Self::Sh
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sh => "sh",
            Self::Ps1 => "ps1",
        }
    }
}

impl Workspace {
    /// What the launcher needs for each step, in order (ADR 0010 §2a).
    pub fn launch_steps(&self) -> impl Iterator<Item = (Step, SpawnMode)> + '_ {
        let count = self.steps.len() as u32;
        self.steps
            .iter()
            .enumerate()
            .map(move |(i, s)| (Step::Launch { index: i as u32 + 1, count, name: s.name.clone() }, s.mode))
    }

    /// Checked when the workspace is activated, not when it is loaded: a workspace only needs
    /// scripts for the platforms it is used on (ADR 0012 §4). `files` as for [`parse`].
    pub fn check_scripts(&self, files: &[String], ext: ScriptExt) -> Result<()> {
        let have: BTreeSet<&str> = files.iter().map(String::as_str).collect();
        for (i, step) in self.steps.iter().enumerate() {
            let path = format!("steps/{}", script_name(i + 1, &step.name, ext));
            if !have.contains(path.as_str()) {
                return Err(self.error(&format!("step {} (\"{}\"): missing {path}", i + 1, step.name)));
            }
        }
        if self.cleanup_timeout.is_some() {
            self.check_cleanup_script(files, ext)?;
        }
        Ok(())
    }

    /// The cleanup script for this platform exists. Used by `check_scripts` and by the
    /// `cleanup` op, so the rule lives in one place.
    pub fn check_cleanup_script(&self, files: &[String], ext: ScriptExt) -> Result<()> {
        let script = format!("cleanup.{}", ext.as_str());
        match files.contains(&script) {
            true => Ok(()),
            false => Err(self.error(&format!("missing {script} for this platform"))),
        }
    }

    fn error(&self, msg: &str) -> Error {
        Error::invalid_params(format!("{}: {msg}", self.id))
    }
}

/// `steps/<NN>-<name>.<ext>`, e.g. `02-editor.sh`.
pub fn script_name(index: usize, name: &str, ext: ScriptExt) -> String {
    format!("{index:02}-{name}.{}", ext.as_str())
}

/// Check one workspace. `id` is its folder name, `text` its `workspace.toml`, and `files` every
/// file in the folder as a path relative to it (`workspace.toml`, `cleanup.sh`,
/// `steps/01-setup.sh`, …).
pub fn parse(id: &str, text: &str, files: &[String]) -> Result<Workspace> {
    let at = |msg: String| Error::invalid_params(format!("{id}/workspace.toml: {msg}"));

    if !valid_name(id) {
        return Err(Error::invalid_params(format!(
            "workspace folder '{id}': the name must match [a-z0-9][a-z0-9_-]* and be at most {MAX_NAME_LEN} characters"
        )));
    }

    let file: File = toml::from_str(text).map_err(|e| at(e.to_string()))?;

    let label = file.workspace.label.trim();
    if label.is_empty() {
        return Err(at("[workspace] label must not be empty".into()));
    }

    if file.step.is_empty() {
        return Err(at("there must be at least one [[step]]".into()));
    }
    if file.step.len() > MAX_STEPS {
        return Err(at(format!("there can be at most {MAX_STEPS} steps, found {}", file.step.len())));
    }

    let mut steps = Vec::with_capacity(file.step.len());
    let mut seen = BTreeSet::new();
    for (i, raw) in file.step.into_iter().enumerate() {
        let n = i + 1;
        let step_at = |msg: String| at(format!("step {n} (\"{}\"): {msg}", raw.name));
        if !valid_name(&raw.name) {
            return Err(step_at(format!(
                "name must match [a-z0-9][a-z0-9_-]* and be at most {MAX_NAME_LEN} characters"
            )));
        }
        if !seen.insert(raw.name.clone()) {
            return Err(step_at("this name is already used by an earlier step".into()));
        }
        let mode = match (raw.mode.as_str(), raw.timeout_s) {
            ("supervised", Some(t)) => SpawnMode::Supervised { timeout: timeout(t).map_err(step_at)? },
            ("supervised", None) => return Err(step_at("mode = \"supervised\" requires timeout_s".into())),
            ("detached", None) => SpawnMode::Detached,
            ("detached", Some(_)) => {
                return Err(step_at("timeout_s is only allowed when mode = \"supervised\"".into()))
            }
            (other, _) => return Err(step_at(format!("mode must be \"supervised\" or \"detached\", not \"{other}\""))),
        };
        steps.push(StepSpec { name: raw.name.clone(), description: raw.description, mode });
    }

    let has_cleanup_script = files.iter().any(|f| f == "cleanup.sh" || f == "cleanup.ps1");
    let cleanup_timeout = match (file.cleanup, has_cleanup_script) {
        (Some(c), true) => Some(timeout(c.timeout_s).map_err(|m| at(format!("[cleanup] {m}")))?),
        (None, false) => None,
        (Some(_), false) => {
            return Err(at("[cleanup] is set but there is no cleanup.sh or cleanup.ps1".into()));
        }
        (None, true) => {
            return Err(at("cleanup.sh or cleanup.ps1 exists, so [cleanup] timeout_s is required".into()));
        }
    };

    let env = check_env(file.env.unwrap_or_default()).map_err(at)?;

    check_steps_folder(id, &steps, files)?;

    Ok(Workspace {
        id: id.to_owned(),
        label: label.to_owned(),
        description: file.workspace.description,
        steps,
        cleanup_timeout,
        env,
    })
}

// ---------------------------------------------------------------- the file as written

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    workspace: Header,
    #[serde(default)]
    step: Vec<RawStep>,
    cleanup: Option<RawCleanup>,
    /// Read loosely, then checked by hand, so a number gets "write it as a string" rather
    /// than serde's generic type error.
    env: Option<toml::Table>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    label: String,
    description: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    name: String,
    mode: String,
    /// Signed, so `-5` gets the range message instead of a type error.
    timeout_s: Option<i64>,
    description: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCleanup {
    timeout_s: i64,
}

// ---------------------------------------------------------------- checks

/// `[a-z0-9][a-z0-9_-]*`, at most 64 characters: workspace ids and step names. The charset is
/// the one record ids use (ADR 0008); ADR 0010 §2a requires it before a name is used in a path.
pub(crate) fn valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    s.len() <= MAX_NAME_LEN
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn timeout(seconds: i64) -> std::result::Result<Duration, String> {
    if (1..=MAX_TIMEOUT_S).contains(&seconds) {
        Ok(Duration::from_secs(seconds as u64))
    } else {
        Err(format!("timeout_s must be a whole number from 1 to {MAX_TIMEOUT_S}, got {seconds}"))
    }
}

fn check_env(table: toml::Table) -> std::result::Result<Vec<(String, String)>, String> {
    let mut env = BTreeMap::new();
    for (name, value) in table {
        let mut chars = name.chars();
        let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            return Err(format!("[env] '{name}' is not a valid variable name ([A-Za-z_][A-Za-z0-9_]*)"));
        }
        // Case-insensitive: Windows treats variable names that way.
        if name.to_ascii_uppercase().starts_with(RESERVED_ENV_PREFIX) {
            return Err(format!(
                "[env] '{name}': names starting with {RESERVED_ENV_PREFIX} are reserved for the variables Shimmer sets"
            ));
        }
        match value {
            toml::Value::String(s) => {
                env.insert(name, s);
            }
            other => {
                return Err(format!("[env] {name} must be a string: write {name} = \"{other}\""));
            }
        }
    }
    Ok(env.into_iter().collect())
}

/// Every script directly in `steps/` must belong to a listed step at exactly its position
/// (ADR 0012 §6). Only `.sh` and `.ps1` files count; hidden files, other files and anything
/// in a subfolder are ignored. A listed step missing its script is not an error here, only on
/// activate ([`Workspace::check_scripts`]).
fn check_steps_folder(id: &str, steps: &[StepSpec], files: &[String]) -> Result<()> {
    let at = |msg: String| Error::invalid_params(format!("{id}/steps: {msg}"));
    let position: BTreeMap<&str, usize> = steps.iter().enumerate().map(|(i, s)| (s.name.as_str(), i + 1)).collect();

    for file in files {
        let Some(name) = file.strip_prefix("steps/") else { continue };
        if name.contains('/') || name.starts_with('.') {
            continue;
        }
        let Some((stem, ext)) = name.rsplit_once('.') else { continue };
        let ext = match ext {
            "sh" => ScriptExt::Sh,
            "ps1" => ScriptExt::Ps1,
            _ => continue,
        };
        let Some((number, step)) = stem
            .split_once('-')
            .filter(|(n, s)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && !s.is_empty())
        else {
            return Err(at(format!("'{name}': scripts in steps/ must be named <NN>-<name>.{}", ext.as_str())));
        };
        let Some(&expected) = position.get(step) else {
            return Err(at(format!("'{name}' is not listed in workspace.toml")));
        };
        let expected_name = script_name(expected, step, ext);
        if number.len() != 2 {
            return Err(at(format!("'{name}': use two digits: '{expected_name}'")));
        }
        if number.parse::<usize>().ok() != Some(expected) {
            return Err(at(format!("'{name}': '{step}' is step {expected}, so expected '{expected_name}'")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use shimmer_core::ErrorCode;

    use super::*;

    /// ADR 0012 §1's example, with all three scripts and cleanup.
    const DEEP_WORK: &str = r#"
[workspace]
label = "Deep Work"
description = "Shimmer development on Hyprland workspace 3"

[[step]]
name = "setup"
description = "Pull the latest code and start the database"
mode = "supervised"
timeout_s = 60

[[step]]
name = "editor"
mode = "detached"

[[step]]
name = "terminal"
mode = "detached"

[cleanup]
timeout_s = 30

[env]
PROJECT_DIR = "/home/me/code/shimmer"
"#;

    fn files(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn deep_work_files() -> Vec<String> {
        files(&["workspace.toml", "steps/01-setup.sh", "steps/02-editor.sh", "steps/03-terminal.sh", "cleanup.sh"])
    }

    /// One step, no cleanup, plus `extra` appended to the file.
    fn minimal(extra: &str) -> String {
        format!("[workspace]\nlabel = \"Deep Work\"\n\n[[step]]\nname = \"setup\"\nmode = \"detached\"\n{extra}")
    }

    fn minimal_files() -> Vec<String> {
        files(&["workspace.toml", "steps/01-setup.sh"])
    }

    /// Parse expecting failure; returns the message after checking the code.
    fn err(id: &str, text: &str, files: &[String]) -> String {
        let e = parse(id, text, files).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidParams, "{}", e.message);
        e.message
    }

    fn assert_has(message: &str, part: &str) {
        assert!(message.contains(part), "expected {part:?} in:\n{message}");
    }

    // ------------------------------------------------------------ the happy path

    #[test]
    fn the_adr_example_parses() {
        let w = parse("deep-work", DEEP_WORK, &deep_work_files()).unwrap();
        assert_eq!(w.id, "deep-work");
        assert_eq!(w.label, "Deep Work");
        assert_eq!(w.description.as_deref(), Some("Shimmer development on Hyprland workspace 3"));
        assert_eq!(w.steps.len(), 3);
        assert_eq!(w.steps[0].mode, SpawnMode::Supervised { timeout: Duration::from_secs(60) });
        assert_eq!(w.steps[0].description.as_deref(), Some("Pull the latest code and start the database"));
        assert_eq!(w.steps[1].mode, SpawnMode::Detached);
        assert_eq!(w.cleanup_timeout, Some(Duration::from_secs(30)));
        assert_eq!(w.env, [("PROJECT_DIR".to_owned(), "/home/me/code/shimmer".to_owned())]);
    }

    #[test]
    fn launch_steps_are_numbered_from_one_with_the_count() {
        let w = parse("deep-work", DEEP_WORK, &deep_work_files()).unwrap();
        let steps: Vec<_> = w.launch_steps().collect();
        assert_eq!(steps[1].0, Step::Launch { index: 2, count: 3, name: "editor".into() });
        assert_eq!(steps[1].1, SpawnMode::Detached);
        assert_eq!(steps.len(), 3);
    }

    #[test]
    fn description_and_env_are_optional() {
        let w = parse("deep-work", &minimal(""), &minimal_files()).unwrap();
        assert_eq!(w.description, None);
        assert!(w.env.is_empty());
        assert_eq!(w.cleanup_timeout, None);
    }

    #[test]
    fn the_label_is_trimmed() {
        let text = minimal("").replace("\"Deep Work\"", "\"  Deep Work  \"");
        assert_eq!(parse("deep-work", &text, &minimal_files()).unwrap().label, "Deep Work");
    }

    // ------------------------------------------------------------ the file

    #[test]
    fn invalid_toml_names_the_file() {
        let m = err("deep-work", "[workspace\nlabel = 1", &minimal_files());
        assert_has(&m, "deep-work/workspace.toml: ");
    }

    #[test]
    fn unknown_keys_are_rejected_everywhere() {
        for extra in [
            "tiemout_s = 30\n",             // inside [[step]]
            "\n[workspace.extra]\nx = 1\n", // nested under [workspace]
            "\n[cleanup]\ntimeout_s = 5\nretries = 2\n",
            "\n[colors]\nx = 1\n", // unknown table
        ] {
            let m = err("deep-work", &minimal(extra), &files(&["workspace.toml", "steps/01-setup.sh", "cleanup.sh"]));
            assert!(m.contains("unknown field") || m.contains("duplicate"), "{extra:?} -> {m}");
        }
    }

    #[test]
    fn wrong_types_are_rejected_not_converted() {
        let text = minimal("").replace("mode = \"detached\"", "mode = \"supervised\"\ntimeout_s = \"30\"");
        let m = err("deep-work", &text, &minimal_files());
        assert_has(&m, "invalid type");
    }

    // ------------------------------------------------------------ [workspace]

    #[test]
    fn label_is_required_and_not_blank() {
        let missing = "[workspace]\n\n[[step]]\nname = \"setup\"\nmode = \"detached\"\n";
        assert_has(&err("deep-work", missing, &minimal_files()), "missing field `label`");
        let blank = minimal("").replace("\"Deep Work\"", "\"   \"");
        assert_has(&err("deep-work", &blank, &minimal_files()), "label must not be empty");
    }

    #[test]
    fn the_folder_name_must_be_a_valid_id() {
        let too_long = "a".repeat(MAX_NAME_LEN + 1);
        for id in ["Deep Work", "deep work", "-deep", "deep.work", "", too_long.as_str()] {
            assert_has(&err(id, &minimal(""), &minimal_files()), "the name must match");
        }
        let longest = "a".repeat(MAX_NAME_LEN);
        for id in ["deep-work", "2026-plan", "deep_work", longest.as_str()] {
            parse(id, &minimal(""), &minimal_files()).unwrap();
        }
    }

    // ------------------------------------------------------------ [[step]]

    #[test]
    fn there_must_be_one_to_ninety_nine_steps() {
        let none = "[workspace]\nlabel = \"Deep Work\"\n";
        assert_has(&err("deep-work", none, &files(&["workspace.toml"])), "at least one [[step]]");

        let mut many = String::from("[workspace]\nlabel = \"x\"\n");
        for i in 1..=MAX_STEPS + 1 {
            many.push_str(&format!("[[step]]\nname = \"s{i}\"\nmode = \"detached\"\n"));
        }
        assert_has(&err("deep-work", &many, &files(&["workspace.toml"])), "at most 99 steps, found 100");
    }

    #[test]
    fn step_names_are_checked_and_unique() {
        for bad in ["Setup", "set up", "-setup", "set.up"] {
            let text = minimal("").replace("name = \"setup\"", &format!("name = \"{bad}\""));
            let m = err("deep-work", &text, &files(&["workspace.toml"]));
            assert_has(&m, &format!("step 1 (\"{bad}\"): name must match"));
        }
        let dup = minimal("\n[[step]]\nname = \"setup\"\nmode = \"detached\"\n");
        assert_has(&err("deep-work", &dup, &minimal_files()), "step 2 (\"setup\"): this name is already used");
    }

    #[test]
    fn mode_is_required_and_exact() {
        let missing = "[workspace]\nlabel = \"x\"\n[[step]]\nname = \"setup\"\n";
        assert_has(&err("deep-work", missing, &minimal_files()), "missing field `mode`");
        for bad in ["Detached", "background", ""] {
            let text = minimal("").replace("\"detached\"", &format!("\"{bad}\""));
            let m = err("deep-work", &text, &minimal_files());
            assert_has(&m, "mode must be \"supervised\" or \"detached\"");
        }
    }

    #[test]
    fn timeout_s_goes_with_supervised_only_and_is_in_range() {
        let supervised = |t: &str| minimal("").replace("mode = \"detached\"", &format!("mode = \"supervised\"{t}"));
        assert_has(&err("deep-work", &supervised(""), &minimal_files()), "requires timeout_s");
        for bad in ["0", "-5", "3601", "300000"] {
            let m = err("deep-work", &supervised(&format!("\ntimeout_s = {bad}")), &minimal_files());
            assert_has(&m, "timeout_s must be a whole number from 1 to 3600");
        }
        for ok in ["1", "3600"] {
            parse("deep-work", &supervised(&format!("\ntimeout_s = {ok}")), &minimal_files()).unwrap();
        }
        let detached = minimal("timeout_s = 30\n");
        assert_has(&err("deep-work", &detached, &minimal_files()), "only allowed when mode = \"supervised\"");
    }

    #[test]
    fn errors_name_the_step_by_number_and_name() {
        let text = DEEP_WORK
            .replace("name = \"editor\"\nmode = \"detached\"", "name = \"editor\"\nmode = \"detached\"\ntimeout_s = 5");
        let m = err("deep-work", &text, &deep_work_files());
        assert_eq!(
            m,
            "deep-work/workspace.toml: step 2 (\"editor\"): timeout_s is only allowed when mode = \"supervised\""
        );
    }

    // ------------------------------------------------------------ steps/ against the list

    #[test]
    fn non_scripts_hidden_files_and_subfolders_are_ignored() {
        let mut f = minimal_files();
        f.extend(files(&[
            "steps/.DS_Store",
            "steps/README.md",
            "steps/notes.txt",
            "steps/old/09-x.sh",
            "steps/.01-x.sh",
        ]));
        parse("deep-work", &minimal(""), &f).unwrap();
    }

    #[test]
    fn both_platform_scripts_for_a_step_are_fine() {
        let f = files(&["workspace.toml", "steps/01-setup.sh", "steps/01-setup.ps1"]);
        parse("deep-work", &minimal(""), &f).unwrap();
    }

    #[test]
    fn each_kind_of_file_mismatch_has_its_own_message() {
        let cases = [
            ("steps/2-editor.sh", "'2-editor.sh': use two digits: '02-editor.sh'"),
            ("steps/05-editor.sh", "'05-editor.sh': 'editor' is step 2, so expected '02-editor.sh'"),
            ("steps/04-music.sh", "'04-music.sh' is not listed in workspace.toml"),
            ("steps/setup.sh", "'setup.sh': scripts in steps/ must be named <NN>-<name>.sh"),
            ("steps/ab-setup.ps1", "'ab-setup.ps1': scripts in steps/ must be named <NN>-<name>.ps1"),
        ];
        for (extra, expected) in cases {
            let mut f = deep_work_files();
            f.push(extra.into());
            let m = err("deep-work", DEEP_WORK, &f);
            assert_eq!(m, format!("deep-work/steps: {expected}"), "{extra}");
        }
    }

    #[test]
    fn a_missing_script_is_not_a_load_error_only_an_activate_error() {
        // Loads: a workspace only needs scripts for the platforms it's used on (ADR 0012 §4).
        let w = parse("deep-work", &minimal(""), &files(&["workspace.toml"])).unwrap();
        let e = w.check_scripts(&files(&["workspace.toml"]), ScriptExt::Sh).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidParams);
        assert_eq!(e.message, "deep-work: step 1 (\"setup\"): missing steps/01-setup.sh");

        // Present for Windows only: fine there, missing on Linux/macOS.
        let ps1 = files(&["workspace.toml", "steps/01-setup.ps1"]);
        let w = parse("deep-work", &minimal(""), &ps1).unwrap();
        w.check_scripts(&ps1, ScriptExt::Ps1).unwrap();
        assert!(w.check_scripts(&ps1, ScriptExt::Sh).is_err());
    }

    #[test]
    fn check_scripts_also_wants_this_platforms_cleanup_script() {
        let f = files(&["workspace.toml", "steps/01-setup.sh", "steps/01-setup.ps1", "cleanup.ps1"]);
        let w = parse("deep-work", &minimal("\n[cleanup]\ntimeout_s = 5\n"), &f).unwrap();
        w.check_scripts(&f, ScriptExt::Ps1).unwrap();
        assert_eq!(
            w.check_scripts(&f, ScriptExt::Sh).unwrap_err().message,
            "deep-work: missing cleanup.sh for this platform"
        );
    }

    // ------------------------------------------------------------ [cleanup]

    #[test]
    fn cleanup_is_required_exactly_when_a_cleanup_script_exists() {
        let with_script = files(&["workspace.toml", "steps/01-setup.sh", "cleanup.sh"]);
        assert_has(&err("deep-work", &minimal(""), &with_script), "so [cleanup] timeout_s is required");
        let ps1_script = files(&["workspace.toml", "steps/01-setup.sh", "cleanup.ps1"]);
        assert_has(&err("deep-work", &minimal(""), &ps1_script), "so [cleanup] timeout_s is required");

        let table = minimal("\n[cleanup]\ntimeout_s = 30\n");
        assert_has(&err("deep-work", &table, &minimal_files()), "[cleanup] is set but there is no cleanup.sh");
        let w = parse("deep-work", &table, &with_script).unwrap();
        assert_eq!(w.cleanup_timeout, Some(Duration::from_secs(30)));
    }

    #[test]
    fn cleanup_timeout_follows_the_same_rules() {
        let with_script = files(&["workspace.toml", "steps/01-setup.sh", "cleanup.sh"]);
        let m = err("deep-work", &minimal("\n[cleanup]\ntimeout_s = 0\n"), &with_script);
        assert_has(&m, "[cleanup] timeout_s must be a whole number from 1 to 3600");
        assert_has(&err("deep-work", &minimal("\n[cleanup]\n"), &with_script), "missing field `timeout_s`");
    }

    // ------------------------------------------------------------ [env]

    #[test]
    fn env_names_must_be_valid() {
        for bad in ["\"1PATH\"", "\"MY-VAR\"", "\"my var\""] {
            let m = err("deep-work", &minimal(&format!("\n[env]\n{bad} = \"x\"\n")), &minimal_files());
            assert_has(&m, "is not a valid variable name");
        }
        let w = parse("deep-work", &minimal("\n[env]\n_X = \"1\"\nPATH = \"/bin\"\n"), &minimal_files()).unwrap();
        assert_eq!(w.env, [("PATH".to_owned(), "/bin".to_owned()), ("_X".to_owned(), "1".to_owned())]);
    }

    #[test]
    fn env_values_must_be_strings() {
        let m = err("deep-work", &minimal("\n[env]\nPORT = 8080\n"), &minimal_files());
        assert_has(&m, "[env] PORT must be a string: write PORT = \"8080\"");
    }

    #[test]
    fn the_whole_shimmer_prefix_is_reserved() {
        // Today's variables, a future one, and a lowercase spelling (Windows ignores case).
        for name in ["SHIMMER_SOCKET", "SHIMMER_HOME", "SHIMMER_SOMETHING_NEW", "shimmer_socket"] {
            let m = err("deep-work", &minimal(&format!("\n[env]\n{name} = \"x\"\n")), &minimal_files());
            assert_has(&m, "names starting with SHIMMER_ are reserved");
        }
        // Merely containing it is fine.
        parse("deep-work", &minimal("\n[env]\nMY_SHIMMER_DIR = \"x\"\n"), &minimal_files()).unwrap();
    }
}
