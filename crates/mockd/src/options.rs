//! `shimmer mockd` arguments. Both paths are required: the mock must never silently take over
//! the real daemon's socket, and it has no home directory of its own.

use std::path::PathBuf;

pub const USAGE: &str = "usage: shimmer mockd --fixtures DIR --socket PATH

  --fixtures DIR   directory of *.json fixture files (see crates/mockd/fixtures/README.md)
  --socket PATH    Unix socket to listen on; pick one other than the real daemon's
  -h, --help       show this help";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub fixtures: PathBuf,
    pub socket: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Run(Options),
    Help,
}

impl Command {
    /// `args` are what follows `shimmer mockd`.
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Self, String> {
        let (mut fixtures, mut socket) = (None, None);
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
                _ => (arg, None),
            };
            let mut value = |name: &str| {
                inline.clone().or_else(|| args.next()).filter(|v| !v.is_empty()).ok_or(format!("{name} needs a value"))
            };
            match flag.as_str() {
                "-h" | "--help" => return Ok(Self::Help),
                "--fixtures" => fixtures = Some(PathBuf::from(value("--fixtures")?)),
                "--socket" => socket = Some(PathBuf::from(value("--socket")?)),
                other => return Err(format!("unknown argument '{other}'")),
            }
        }
        match (fixtures, socket) {
            (Some(fixtures), Some(socket)) => Ok(Self::Run(Options { fixtures, socket })),
            (None, _) => Err("--fixtures is required".into()),
            (_, None) => Err("--socket is required".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        Command::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn both_paths_are_required_and_either_spelling_works() {
        let want = Command::Run(Options { fixtures: "f".into(), socket: "/tmp/s.sock".into() });
        assert_eq!(parse(&["--fixtures", "f", "--socket", "/tmp/s.sock"]).unwrap(), want);
        assert_eq!(parse(&["--socket=/tmp/s.sock", "--fixtures=f"]).unwrap(), want);
        assert!(parse(&["--fixtures", "f"]).unwrap_err().contains("--socket"));
        assert!(parse(&["--socket", "s"]).unwrap_err().contains("--fixtures"));
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn help_and_bad_input() {
        assert_eq!(parse(&["--help"]).unwrap(), Command::Help);
        assert_eq!(parse(&["-h"]).unwrap(), Command::Help);
        assert!(parse(&["--fixtures"]).unwrap_err().contains("needs a value"));
        assert!(parse(&["--fixtures="]).unwrap_err().contains("needs a value"));
        assert!(parse(&["--nope", "x"]).unwrap_err().contains("unknown argument"));
    }
}
