//! Factory presets, compiled into the binary.
//!
//! Embedding rather than shipping loose files means the library can never go
//! missing or half-installed; user presets on disk extend this set.

use crate::preset::Preset;

pub const FACTORY_JSON: &[&str] = &[
    include_str!("../presets/alpha-settle.json"),
    include_str!("../presets/anxiety-downshift.json"),
    include_str!("../presets/body-relaxation.json"),
    include_str!("../presets/creative-reverie.json"),
    include_str!("../presets/deep-focus-flow.json"),
    include_str!("../presets/deep-theta.json"),
    include_str!("../presets/gamma-concentration.json"),
    include_str!("../presets/lucid-rem.json"),
    include_str!("../presets/morning-activation.json"),
    include_str!("../presets/power-nap.json"),
    include_str!("../presets/schumann-ground.json"),
    include_str!("../presets/sleep-onset.json"),
    include_str!("../presets/stack-expanded-state.json"),
    include_str!("../presets/study-reading.json"),
];

/// Parse every factory preset. Panics on a malformed one, which is correct:
/// these are compiled in, so a failure here is a build-time bug, not a
/// runtime condition to recover from. `factory_presets_are_all_valid` catches
/// it in CI first.
pub fn load_all() -> Vec<Preset> {
    FACTORY_JSON
        .iter()
        .map(|s| Preset::from_json(s).expect("factory preset failed to parse"))
        .collect()
}

pub fn by_id(id: &str) -> Option<Preset> {
    load_all().into_iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_presets_are_all_valid() {
        let presets = load_all();
        assert_eq!(presets.len(), 14);
        for p in &presets {
            p.validate().unwrap_or_else(|e| panic!("{}: {e}", p.id));
            assert!(!p.name.is_empty(), "{} has no name", p.id);
            assert!(!p.description.is_empty(), "{} has no description", p.id);
        }
    }

    #[test]
    fn preset_ids_are_unique() {
        let presets = load_all();
        let mut ids: Vec<&str> = presets.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate preset id");
    }

    /// The headphone flag drives a user-facing warning, so it must match the
    /// actual layer stack rather than whatever the JSON happened to say.
    #[test]
    fn headphone_flags_match_the_layers() {
        for mut p in load_all() {
            let stored = p.requires_headphones;
            p.refresh_headphone_flag();
            assert_eq!(stored, p.requires_headphones, "{} has a stale flag", p.id);
        }
    }
}
