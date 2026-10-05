//! Workspace templates (ADR 0025): a workspace folder plus `template.toml`'s questions. The
//! answers become the created workspace's `[env]`; the scripts are copied as they are.
//!
//! Pure: a template is its files' text, answers are plain values, and the result is the files to
//! write (§12 rule 10). The module checks the result with [`crate::manifest::parse`] and writes
//! it in one transaction.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::{json, Map, Value};
use shimmer_core::{Error, Result};

use crate::manifest::RESERVED_ENV_PREFIX;

/// The file holding the questions. Never copied into a workspace.
pub const TEMPLATE_FILE: &str = "template.toml";

/// Variables other programs rely on. `[env]` is passed to everything a step starts, so a
/// question may not set these (ADR 0025 §2).
const COMMON_ENV: [&str; 12] =
    ["PATH", "HOME", "USER", "SHELL", "PWD", "TMPDIR", "LANG", "TERM", "TERMINAL", "EDITOR", "VISUAL", "BROWSER"];

/// What an answer may be (ADR 0025 §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Text,
    Command,
    Folder,
    File,
    Url,
    Urls,
    Choice,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Command => "command",
            Self::Folder => "folder",
            Self::File => "file",
            Self::Url => "url",
            Self::Urls => "urls",
            Self::Choice => "choice",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub name: String,
    pub prompt: String,
    pub kind: Kind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub help: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    template: Header,
    #[serde(default)]
    question: Vec<Question>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    label: String,
    description: String,
}

/// A checked template: its questions, and the workspace files it creates.
#[derive(Clone, Debug)]
pub struct Template {
    pub id: String,
    pub label: String,
    pub description: String,
    pub questions: Vec<Question>,
    /// Every file but `template.toml`, by path inside the workspace folder.
    pub files: Vec<(String, String)>,
}

impl Template {
    /// Read a template from its files (`template.toml`, `workspace.toml`, `steps/…`, …). Every
    /// problem names the template and the question.
    pub fn parse(id: &str, files: &[(&str, &str)]) -> Result<Self> {
        let bad = |msg: String| Error::invalid_params(format!("template '{id}': {msg}"));
        let text = files
            .iter()
            .find(|(p, _)| *p == TEMPLATE_FILE)
            .map(|(_, t)| *t)
            .ok_or_else(|| bad(format!("has no {TEMPLATE_FILE}")))?;
        let file: File = toml::from_str(text).map_err(|e| bad(format!("{TEMPLATE_FILE}: {e}")))?;

        let mut seen = BTreeSet::new();
        for q in &file.question {
            let at = |msg: &str| bad(format!("question '{}': {msg}", q.name));
            check_name(&q.name).map_err(|m| at(&m))?;
            if !seen.insert(q.name.as_str()) {
                return Err(at("is asked twice"));
            }
            if q.prompt.trim().is_empty() {
                return Err(at("prompt must not be empty"));
            }
            match (q.kind, q.choices.is_empty()) {
                (Kind::Choice, true) => return Err(at("a choice needs choices")),
                (Kind::Choice, false) if q.choices.iter().collect::<BTreeSet<_>>().len() != q.choices.len() => {
                    return Err(at("repeats a choice"));
                }
                (Kind::Choice, false) => {}
                (_, false) => return Err(at("only a choice takes choices")),
                (_, true) => {}
            }
            if let Some(default) = &q.default {
                if q.required {
                    return Err(at("a required question can't have a default"));
                }
                // `auto` is an ordinary answer the scripts work out (ADR 0025 §3).
                if default != "auto" || q.kind == Kind::Choice {
                    q.check(default).map_err(|m| at(&format!("default: {m}")))?;
                }
            }
        }

        let files: Vec<(String, String)> = files
            .iter()
            .filter(|(p, _)| *p != TEMPLATE_FILE)
            .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
            .collect();
        let workspace =
            files.iter().find(|(p, _)| p == "workspace.toml").ok_or_else(|| bad("has no workspace.toml".into()))?;
        let doc: toml::Table = toml::from_str(&workspace.1).map_err(|e| bad(format!("workspace.toml: {e}")))?;
        if doc.contains_key("env") {
            return Err(bad("workspace.toml must not have [env]: the answers become it".into()));
        }
        if workspace.1.lines().filter(|l| l.starts_with("label = ")).count() != 1 {
            return Err(bad("workspace.toml must have exactly one line starting 'label = '".into()));
        }

        Ok(Self {
            id: id.to_owned(),
            label: file.template.label,
            description: file.template.description,
            questions: file.question,
            files,
        })
    }

    /// As `workspaces.templates` shows it.
    pub fn to_wire(&self) -> Value {
        let questions: Vec<Value> = self
            .questions
            .iter()
            .map(|q| {
                let mut out =
                    json!({"name": q.name, "prompt": q.prompt, "kind": q.kind.as_str(), "required": q.required});
                if let Some(d) = &q.default {
                    out["default"] = json!(d);
                }
                if !q.choices.is_empty() {
                    out["choices"] = json!(q.choices);
                }
                if let Some(h) = &q.help {
                    out["help"] = json!(h);
                }
                out
            })
            .collect();
        json!({"id": self.id, "label": self.label, "description": self.description, "questions": questions})
    }

    /// Every question's answer, in question order: the given value, else the default, else
    /// empty. Unknown names, missing required answers and answers that don't fit their kind are
    /// `invalid_params` naming the question.
    pub fn answers(&self, values: &Map<String, Value>) -> Result<Vec<(String, String)>> {
        if let Some(name) = values.keys().find(|k| !self.questions.iter().any(|q| &q.name == *k)) {
            let names: Vec<&str> = self.questions.iter().map(|q| q.name.as_str()).collect();
            return Err(Error::invalid_params(format!(
                "template '{}' has no question '{name}' (it asks: {})",
                self.id,
                names.join(", ")
            )));
        }
        let mut out = Vec::new();
        for q in &self.questions {
            let given = match values.get(&q.name) {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.trim().to_owned()),
                Some(other) => {
                    return Err(Error::invalid_params(format!("{}: answers are text, got {other}", q.name)));
                }
            };
            let value = match given.filter(|s| !s.is_empty()) {
                Some(v) => {
                    if !(v == "auto" && q.default.as_deref() == Some("auto")) {
                        q.check(&v).map_err(|m| Error::invalid_params(format!("{} ({}): {m}", q.name, q.prompt)))?;
                    }
                    v
                }
                None if q.required => {
                    return Err(Error::invalid_params(format!("{} ({}) is required", q.name, q.prompt)));
                }
                None => q.default.clone().unwrap_or_default(),
            };
            out.push((q.name.clone(), value));
        }
        Ok(out)
    }

    /// The workspace's files: everything but `template.toml`, with `workspace.toml` given the
    /// answers as `[env]` (empty ones too, so every setting can be found and changed later) and,
    /// when given, a new label.
    pub fn workspace_files(&self, answers: &[(String, String)], label: Option<&str>) -> Vec<(String, String)> {
        self.files
            .iter()
            .map(|(path, text)| {
                if path != "workspace.toml" {
                    return (path.clone(), text.clone());
                }
                let mut out = String::new();
                for line in text.lines() {
                    match (label, line.starts_with("label = ")) {
                        (Some(label), true) => out.push_str(&format!("label = {}", quoted(label))),
                        _ => out.push_str(line),
                    }
                    out.push('\n');
                }
                if !answers.is_empty() {
                    out.push_str("\n# Your answers. Change them here; every step reads them.\n[env]\n");
                    for (name, value) in answers {
                        out.push_str(&format!("{name} = {}\n", quoted(value)));
                    }
                }
                (path.clone(), out)
            })
            .collect()
    }
}

impl Question {
    /// Does `value` (non-empty) fit this question's kind? `Err` says what it should be.
    fn check(&self, value: &str) -> std::result::Result<(), String> {
        if value.contains(['\n', '\r']) {
            return Err("must be one line".into());
        }
        let url = |v: &str| (v.starts_with("http://") || v.starts_with("https://")) && !v.contains(char::is_whitespace);
        match self.kind {
            Kind::Text | Kind::Command => Ok(()),
            Kind::Folder | Kind::File if value.starts_with('/') => Ok(()),
            Kind::Folder | Kind::File => Err(format!("must be an absolute path, got '{value}'")),
            Kind::Url if url(value) => Ok(()),
            Kind::Url => Err(format!("must start with http:// or https://, got '{value}'")),
            Kind::Urls => match value.split_whitespace().find(|v| !url(v)) {
                None => Ok(()),
                Some(v) => Err(format!("each link must start with http:// or https://, got '{v}'")),
            },
            Kind::Choice if self.choices.iter().any(|c| c == value) => Ok(()),
            Kind::Choice => Err(format!("must be one of {}, got '{value}'", self.choices.join(", "))),
        }
    }
}

/// A question name is an `[env]` name (ADR 0012 §6) that no other program relies on.
fn check_name(name: &str) -> std::result::Result<(), String> {
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err("must be a variable name ([A-Za-z_][A-Za-z0-9_]*)".into());
    }
    if name.to_ascii_uppercase().starts_with(RESERVED_ENV_PREFIX) {
        return Err(format!("names starting with {RESERVED_ENV_PREFIX} are reserved"));
    }
    if COMMON_ENV.contains(&name) {
        return Err("other programs use this variable; pick another name".into());
    }
    Ok(())
}

/// A TOML basic string.
fn quoted(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKSPACE: &str = "# Hi\n[workspace]\nlabel = \"Site\"\n\n[[step]]\nname = \"editor\"\nmode = \"detached\"\n";

    fn template(questions: &str) -> Result<Template> {
        let text = format!("[template]\nlabel = \"Site\"\ndescription = \"A site\"\n{questions}");
        Template::parse(
            "site",
            &[("template.toml", text.as_str()), ("workspace.toml", WORKSPACE), ("steps/01-editor.sh", "true\n")],
        )
    }

    const QUESTIONS: &str = r#"
[[question]]
name = "PROJECT_DIR"
prompt = "Project folder"
kind = "folder"
required = true

[[question]]
name = "CODE_EDITOR"
prompt = "Editor"
kind = "choice"
choices = ["vscode", "cursor"]
default = "vscode"

[[question]]
name = "LOCAL_URL"
prompt = "Local URL"
kind = "url"
default = "auto"

[[question]]
name = "LINKS"
prompt = "Links"
kind = "urls"
"#;

    fn values(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn answers_take_defaults_and_keep_empty_ones() {
        let t = template(QUESTIONS).unwrap();
        let got = t
            .answers(&values(json!({"PROJECT_DIR": " /home/me/site ", "LINKS": "https://a.dev https://b.dev"})))
            .unwrap();
        assert_eq!(
            got,
            [
                ("PROJECT_DIR".into(), "/home/me/site".into()),
                ("CODE_EDITOR".into(), "vscode".into()),
                ("LOCAL_URL".into(), "auto".into()),
                ("LINKS".into(), "https://a.dev https://b.dev".into())
            ]
        );
        let got = t.answers(&values(json!({"PROJECT_DIR": "/p", "LOCAL_URL": "auto", "LINKS": ""}))).unwrap();
        assert_eq!(got[3], ("LINKS".into(), String::new()), "empty stays empty");
    }

    #[test]
    fn bad_answers_name_the_question() {
        let t = template(QUESTIONS).unwrap();
        for (v, want) in [
            (json!({}), "PROJECT_DIR (Project folder) is required"),
            (json!({"PROJECT_DIR": "~/site"}), "must be an absolute path"),
            (json!({"PROJECT_DIR": "/p", "CODE_EDITOR": "emacs"}), "must be one of vscode, cursor"),
            (json!({"PROJECT_DIR": "/p", "LOCAL_URL": "localhost:3000"}), "must start with http"),
            (json!({"PROJECT_DIR": "/p", "LINKS": "https://a.dev vercel.com"}), "got 'vercel.com'"),
            (json!({"PROJECT_DIR": "/p", "NOPE": "x"}), "has no question 'NOPE'"),
            (json!({"PROJECT_DIR": 3}), "answers are text"),
            (json!({"PROJECT_DIR": "/p\n/q"}), "one line"),
        ] {
            let e = t.answers(&values(v.clone())).unwrap_err();
            assert!(e.message.contains(want), "{v}: {}", e.message);
        }
    }

    #[test]
    fn the_workspace_file_gets_env_and_the_label() {
        let t = template(QUESTIONS).unwrap();
        let answers = t.answers(&values(json!({"PROJECT_DIR": "/home/me/my \"site\""}))).unwrap();
        let files = t.workspace_files(&answers, Some("My site"));
        assert_eq!(files.len(), 2, "template.toml is not copied");
        let text = &files.iter().find(|(p, _)| p == "workspace.toml").unwrap().1;
        assert!(text.starts_with("# Hi\n[workspace]\nlabel = \"My site\"\n"), "{text}");
        assert!(text.contains("\n[env]\nPROJECT_DIR = "), "{text}");
        assert!(text.contains("CODE_EDITOR = \"vscode\"\nLOCAL_URL = \"auto\"\nLINKS = \"\"\n"), "{text}");
        let env: toml::Table = toml::from_str(text).unwrap();
        assert_eq!(env["env"]["PROJECT_DIR"].as_str(), Some("/home/me/my \"site\""), "quoted safely");
        assert!(t.workspace_files(&answers, None)[0].1.contains("label = \"Site\""));
    }

    #[test]
    fn bad_templates_are_rejected() {
        for (questions, want) in [
            ("[[question]]\nname = \"EDITOR\"\nprompt = \"E\"\nkind = \"text\"", "other programs use this variable"),
            ("[[question]]\nname = \"SHIMMER_X\"\nprompt = \"E\"\nkind = \"text\"", "reserved"),
            ("[[question]]\nname = \"1X\"\nprompt = \"E\"\nkind = \"text\"", "variable name"),
            ("[[question]]\nname = \"A\"\nprompt = \"E\"\nkind = \"choice\"", "needs choices"),
            ("[[question]]\nname = \"A\"\nprompt = \"E\"\nkind = \"text\"\nchoices = [\"a\"]", "only a choice"),
            ("[[question]]\nname = \"A\"\nprompt = \"E\"\nkind = \"choice\"\nchoices = [\"a\"]\ndefault = \"b\"", "default"),
            ("[[question]]\nname = \"A\"\nprompt = \"E\"\nkind = \"text\"\nrequired = true\ndefault = \"b\"", "can't have a default"),
            ("[[question]]\nname = \"A\"\nprompt = \"E\"\nkind = \"text\"\n[[question]]\nname = \"A\"\nprompt = \"E\"\nkind = \"text\"", "asked twice"),
            ("[[question]]\nname = \"A\"\nprompt = \"E\"\nkind = \"number\"", "unknown variant"),
            ("[[question]]\nname = \"A\"\nprompt = \"E\"\nkind = \"text\"\nask = 1", "unknown field"),
        ] {
            let e = template(questions).unwrap_err();
            assert!(e.message.contains("template 'site'") && e.message.contains(want), "{questions}: {}", e.message);
        }
        let with_env = format!("{WORKSPACE}[env]\nA = \"b\"\n");
        let e = Template::parse(
            "site",
            &[("template.toml", "[template]\nlabel = \"S\"\ndescription = \"d\""), ("workspace.toml", &with_env)],
        )
        .unwrap_err();
        assert!(e.message.contains("must not have [env]"));
    }
}
