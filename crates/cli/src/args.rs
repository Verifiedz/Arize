//! `swe <command>` arguments. Hand-rolled like `swe mockd`'s: a handful of commands does not
//! need a parser crate, and command packs (§15.1) will rewrite the command word before this runs.

use std::path::PathBuf;

use serde_json::Value;

pub const USAGE: &str = "usage: swe [--socket PATH] [--json] <command>

commands:
  ping                  check the daemon is up
  manifest              list registered modules, their ops and lanes
  call OP [PARAMS]      send any op; PARAMS is a JSON object (default {})
  shutdown              stop the daemon
  daemon                run the daemon in the foreground (Ctrl-C to stop)
  mockd                 run the mock daemon for building clients (swe mockd --help)

options:
  --socket PATH         talk to this socket instead of the default; never auto-starts
  --json                print the daemon's raw JSON instead of formatted output
  -h, --help            show this help

The daemon is started automatically when it is not running.";

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Help,
    Ping,
    Manifest,
    Shutdown,
    Call { op: String, params: Value },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    /// `None` = the default socket, which is also the only case where the daemon auto-starts.
    pub socket: Option<PathBuf>,
    pub json: bool,
    pub command: Command,
}

impl Args {
    /// `args` are what follows `swe`. Flags may appear anywhere.
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Self, String> {
        let (mut socket, mut json, mut help) = (None, false, false);
        let mut words = Vec::new();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
                _ => (arg, None),
            };
            match flag.as_str() {
                "-h" | "--help" => help = true,
                "--json" => json = true,
                "--socket" => {
                    let v = inline.or_else(|| args.next()).filter(|v| !v.is_empty());
                    socket = Some(PathBuf::from(v.ok_or("--socket needs a value")?));
                }
                f if f.starts_with('-') && f.len() > 1 => return Err(format!("unknown option '{f}'")),
                _ => words.push(flag),
            }
        }
        let command = if help { Command::Help } else { command(words)? };
        Ok(Self { socket, json, command })
    }
}

fn command(words: Vec<String>) -> Result<Command, String> {
    let mut words = words.into_iter();
    let Some(word) = words.next() else { return Ok(Command::Help) };
    let rest: Vec<String> = words.collect();
    let no_args = |c: Command| if rest.is_empty() { Ok(c) } else { Err(format!("'{word}' takes no arguments")) };
    match word.as_str() {
        "help" => no_args(Command::Help),
        "ping" => no_args(Command::Ping),
        "manifest" => no_args(Command::Manifest),
        "shutdown" => no_args(Command::Shutdown),
        "call" => call(rest),
        other => Err(format!("unknown command '{other}'")),
    }
}

fn call(rest: Vec<String>) -> Result<Command, String> {
    let mut rest = rest.into_iter();
    let op = rest.next().ok_or("call needs an op, e.g. 'swe call core.ping'")?;
    let params = match rest.next() {
        None => Value::Object(Default::default()),
        Some(text) => {
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("PARAMS is not valid JSON: {e}"))?;
            if !v.is_object() {
                return Err("PARAMS must be a JSON object, e.g. '{\"id\":\"two-sum\"}'".into());
            }
            v
        }
    };
    if rest.next().is_some() {
        return Err("call takes an op and at most one PARAMS argument; quote the JSON".into());
    }
    Ok(Command::Call { op, params })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn commands_and_flags_anywhere() {
        let a = parse(&["ping"]).unwrap();
        assert_eq!(a, Args { socket: None, json: false, command: Command::Ping });

        let a = parse(&["--json", "manifest", "--socket", "/tmp/s.sock"]).unwrap();
        assert_eq!(a, Args { socket: Some("/tmp/s.sock".into()), json: true, command: Command::Manifest });
        assert_eq!(parse(&["manifest", "--socket=/tmp/s.sock"]).unwrap().socket, Some("/tmp/s.sock".into()));
    }

    #[test]
    fn help_wins_and_is_the_default() {
        assert_eq!(parse(&[]).unwrap().command, Command::Help);
        assert_eq!(parse(&["ping", "--help"]).unwrap().command, Command::Help);
        assert_eq!(parse(&["-h"]).unwrap().command, Command::Help);
        assert_eq!(parse(&["help"]).unwrap().command, Command::Help);
    }

    #[test]
    fn call_takes_an_op_and_optional_object() {
        assert_eq!(
            parse(&["call", "core.ping"]).unwrap().command,
            Command::Call { op: "core.ping".into(), params: json!({}) }
        );
        assert_eq!(
            parse(&["call", "records.list", r#"{"collection":"leetcode"}"#]).unwrap().command,
            Command::Call { op: "records.list".into(), params: json!({"collection": "leetcode"}) }
        );
        assert!(parse(&["call"]).unwrap_err().contains("needs an op"));
        assert!(parse(&["call", "x.y", "{nope"]).unwrap_err().contains("not valid JSON"));
        assert!(parse(&["call", "x.y", "[1]"]).unwrap_err().contains("JSON object"));
        assert!(parse(&["call", "x.y", "{}", "extra"]).unwrap_err().contains("at most one"));
    }

    #[test]
    fn bad_input() {
        assert!(parse(&["nope"]).unwrap_err().contains("unknown command"));
        assert!(parse(&["ping", "extra"]).unwrap_err().contains("takes no arguments"));
        assert!(parse(&["--frobnicate"]).unwrap_err().contains("unknown option"));
        assert!(parse(&["ping", "--socket"]).unwrap_err().contains("needs a value"));
        assert!(parse(&["ping", "--socket="]).unwrap_err().contains("needs a value"));
    }
}
