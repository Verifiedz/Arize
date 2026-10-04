//! The built-in packs (ADR 0013 §5). Each is a real `pack.toml` in `crates/cli/packs/`, compiled
//! into the binary and checked by the same code as anyone's own pack. All are unbranded: no name
//! from another company's game, show, film or brand ever ships in the binary.

use super::pack::{parse, Pack};

/// (id, `pack.toml`), in the order `shimmer packs list` shows them.
pub const BUILTIN: &[(&str, &str)] = &[
    ("anime-tropes", include_str!("../../packs/anime-tropes.toml")),
    ("ship-it", include_str!("../../packs/ship-it.toml")),
    ("starship", include_str!("../../packs/starship.toml")),
    ("short", include_str!("../../packs/short.toml")),
];

/// The built-in pack `id`, if there is one. Every built-in passes its checks (a test below), so
/// `None` only ever means "no such built-in".
pub fn builtin(id: &str) -> Option<Pack> {
    let (_, text) = BUILTIN.iter().find(|(b, _)| *b == id)?;
    parse(&format!("built-in pack '{id}'"), text).ok()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::pack::TARGETS;
    use super::super::COMMON_TOOLS;
    use super::*;

    fn all() -> Vec<Pack> {
        BUILTIN
            .iter()
            .map(|(id, text)| parse(id, text).unwrap_or_else(|p| panic!("built-in '{id}' fails its checks: {p:#?}")))
            .collect()
    }

    #[test]
    fn every_built_in_passes_every_check_under_its_own_id() {
        for ((id, _), pack) in BUILTIN.iter().zip(all()) {
            assert_eq!(pack.id, *id);
            assert_eq!(builtin(id), Some(pack));
        }
        assert_eq!(builtin("nope"), None);
    }

    #[test]
    fn every_built_in_names_every_command_exactly_once() {
        for pack in all() {
            let mut ops: Vec<&str> = pack.aliases.values().map(|t| t.op).collect();
            ops.sort();
            let mut want: Vec<&str> = TARGETS.iter().map(|t| t.op).collect();
            want.sort();
            assert_eq!(ops, want, "{}", pack.id);
        }
    }

    #[test]
    fn no_built_in_alias_is_a_common_tool_or_shared_between_packs() {
        let mut seen = BTreeSet::new();
        for pack in all() {
            for alias in pack.aliases.keys() {
                assert!(!COMMON_TOOLS.contains(&alias.as_str()), "{}: '{alias}' is a common tool", pack.id);
                assert!(seen.insert(alias.clone()), "'{alias}' is in two built-in packs");
            }
        }
    }
}
