//! `config.toml`: lane overrides, per-module sections, and general settings. Read once at
//! startup; a missing file means defaults.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;
use swe_core::{Error, LocalTimezone, Result};

#[derive(Debug, Default)]
pub struct Config {
    /// `[lanes] fetchers = 4`
    pub lanes: HashMap<String, usize>,
    /// `[modules.records] ...`, handed to that module as its `ModuleConfig`.
    pub modules: HashMap<String, Value>,
    /// `[general] local_timezone = "America/New_York"` (ADR 0009). Defaults to UTC.
    pub local_timezone: LocalTimezone,
}

impl Config {
    /// An explicit `[general] local_timezone` in `config.toml` wins. Otherwise this
    /// best-effort detects the system zone and writes it back to `config.toml` — so it is
    /// resolved once, inspectable, and hand-editable afterward, not silently re-detected on
    /// every start — falling back to UTC with a logged warning if detection fails.
    pub fn load(home: &Path) -> Result<Self> {
        let path = home.join("config.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let mut cfg = Self::parse(&text)?;
        let explicit = toml::from_str::<toml::Table>(&text)
            .ok()
            .and_then(|root| root.get("general")?.get("local_timezone").cloned())
            .is_some();
        if !explicit {
            let (tz, name) = detect_system_timezone();
            cfg.local_timezone = tz;
            if let Err(e) = persist_detected_timezone(&path, &text, &name) {
                tracing::warn!(error = %e, path = %path.display(), "could not save detected local_timezone to config.toml");
            }
        }
        Ok(cfg)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let root: toml::Table = toml::from_str(text).map_err(|e| Error::invalid_params(format!("config.toml: {e}")))?;
        let mut cfg = Self::default();
        if let Some(lanes) = root.get("lanes") {
            let table =
                lanes.as_table().ok_or_else(|| Error::invalid_params("config.toml: [lanes] must be a table"))?;
            for (name, v) in table {
                let n = v.as_integer().filter(|n| *n >= 1).ok_or_else(|| {
                    Error::invalid_params(format!("config.toml: lanes.{name} must be an integer >= 1"))
                })?;
                cfg.lanes.insert(name.clone(), n as usize);
            }
        }
        if let Some(modules) = root.get("modules").and_then(|m| m.as_table()) {
            for (name, section) in modules {
                let json = serde_json::to_value(section)
                    .map_err(|e| Error::invalid_params(format!("config.toml: modules.{name}: {e}")))?;
                cfg.modules.insert(name.clone(), json);
            }
        }
        if let Some(name) = root.get("general").and_then(|g| g.get("local_timezone")).and_then(|v| v.as_str()) {
            cfg.local_timezone = LocalTimezone::parse(name).map_err(|_| {
                Error::invalid_params(format!("config.toml: general.local_timezone: unknown IANA timezone '{name}'"))
            })?;
        }
        Ok(cfg)
    }
}

/// `TZ`, else the `/etc/localtime` symlink target (Linux/macOS), else UTC.
fn detect_system_timezone() -> (LocalTimezone, String) {
    let candidate = std::env::var("TZ").ok().or_else(|| {
        std::fs::read_link("/etc/localtime")
            .ok()
            .and_then(|target| target.to_str()?.split("zoneinfo/").nth(1).map(str::to_string))
    });
    match candidate.and_then(|name| LocalTimezone::parse(&name).ok().map(|tz| (tz, name))) {
        Some(found) => found,
        None => {
            tracing::warn!(
                "could not detect the system timezone; local_timezone defaults to UTC \
                 (set [general] local_timezone in config.toml to override)"
            );
            (LocalTimezone::UTC, "UTC".to_string())
        }
    }
}

/// Adds `local_timezone = "<name>"` under an existing `[general]` table, or appends a new
/// one, leaving the rest of the file untouched (no reformatting, no comment loss).
fn persist_detected_timezone(path: &Path, existing: &str, name: &str) -> Result<()> {
    let addition = format!("local_timezone = \"{name}\"\n");
    let new_text = if let Some(idx) = existing.find("[general]") {
        let insert_at = existing[idx..].find('\n').map(|n| idx + n + 1).unwrap_or(existing.len());
        let mut s = existing.to_string();
        s.insert_str(insert_at, &addition);
        s
    } else if existing.trim().is_empty() {
        format!("[general]\n{addition}")
    } else {
        format!("{existing}\n[general]\n{addition}")
    };
    swe_store::write_atomic(path, new_text.as_bytes()).map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lanes_and_module_sections() {
        let cfg = Config::parse("[lanes]\nfetchers = 2\n[modules.records]\npage_size = 50\n").unwrap();
        assert_eq!(cfg.lanes["fetchers"], 2);
        assert_eq!(cfg.modules["records"]["page_size"], 50);
    }

    #[test]
    fn rejects_zero_concurrency() {
        assert!(Config::parse("[lanes]\nfetchers = 0\n").is_err());
    }

    #[test]
    fn parses_an_explicit_local_timezone() {
        let cfg = Config::parse("[general]\nlocal_timezone = \"America/New_York\"\n").unwrap();
        assert_eq!(cfg.local_timezone.name(), "America/New_York");
    }

    #[test]
    fn defaults_local_timezone_to_utc() {
        assert_eq!(Config::parse("").unwrap().local_timezone, LocalTimezone::UTC);
    }

    #[test]
    fn rejects_an_unknown_local_timezone() {
        let e = Config::parse("[general]\nlocal_timezone = \"Not/AZone\"\n").unwrap_err();
        assert!(e.message.contains("Not/AZone"));
    }

    #[test]
    fn persist_appends_a_general_table_when_absent() {
        let text = "[lanes]\nfetchers = 2\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        persist_detected_timezone(&path, text, "America/New_York").unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        let cfg = Config::parse(&written).unwrap();
        assert_eq!(cfg.lanes["fetchers"], 2, "existing content must survive");
        assert_eq!(cfg.local_timezone.name(), "America/New_York");
    }

    #[test]
    fn persist_inserts_into_an_existing_general_table() {
        let text = "[general]\nsome_other_key = 1\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        persist_detected_timezone(&path, text, "UTC").unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("some_other_key = 1"));
        assert_eq!(Config::parse(&written).unwrap().local_timezone, LocalTimezone::UTC);
    }
}
