//! `config.toml`: lane overrides and per-module sections. Read once at startup; a missing
//! file means defaults.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;
use swe_core::{Error, Result};

#[derive(Debug, Default)]
pub struct Config {
    /// `[lanes] fetchers = 4`
    pub lanes: HashMap<String, usize>,
    /// `[modules.records] ...`, handed to that module as its `ModuleConfig`.
    pub modules: HashMap<String, Value>,
}

impl Config {
    pub fn load(home: &Path) -> Result<Self> {
        match std::fs::read_to_string(home.join("config.toml")) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
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
        Ok(cfg)
    }
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
}
