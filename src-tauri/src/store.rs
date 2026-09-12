//! The user preset library on disk.
//!
//! Presets are plain JSON files, one per preset, in the app's config
//! directory. That makes them diffable, shareable as a single file, and
//! recoverable by hand if anything here goes wrong — no database, no index to
//! fall out of sync with the files.

use std::path::{Path, PathBuf};

use truezen_engine::factory;
use truezen_engine::preset::Preset;

/// Where a preset came from. User presets can be edited and deleted; factory
/// ones can only be copied.
#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Factory,
    User,
    /// A user preset saved over a factory id, which hides the original. Kept
    /// distinct so the UI can offer "revert to the shipped version".
    Override,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Ids are used as filenames, so they must not be able to escape the
    /// preset directory or collide with anything outside it.
    pub fn sanitise_id(raw: &str) -> String {
        let cleaned: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        // Collapse runs of separators and trim them from the ends, so
            // "My Preset!!" becomes "my-preset" rather than "my-preset--".
        let collapsed = cleaned
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        if collapsed.is_empty() {
            "preset".into()
        } else {
            collapsed.chars().take(64).collect()
        }
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", Self::sanitise_id(id)))
    }

    fn ensure_dir(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("could not create {}: {e}", self.dir.display()))
    }

    /// Every user preset on disk. A file that fails to parse is skipped rather
    /// than failing the whole listing — one bad hand-edit should not hide the
    /// rest of the library.
    pub fn user_presets(&self) -> Vec<Preset> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<Preset> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|text| Preset::from_json(&text).ok())
            .filter(|p| p.validate().is_ok())
            .collect();
        out.sort_by_key(|p| p.name.to_lowercase());
        out
    }

    /// Factory presets plus user presets, with a user preset of the same id
    /// hiding the factory one.
    pub fn all(&self) -> Vec<(Preset, Source)> {
        let user = self.user_presets();
        let factory_ids: Vec<String> = factory::load_all().iter().map(|p| p.id.clone()).collect();

        let mut out: Vec<(Preset, Source)> = factory::load_all()
            .into_iter()
            .filter(|f| !user.iter().any(|u| u.id == f.id))
            .map(|p| (p, Source::Factory))
            .collect();

        for p in user {
            let source = if factory_ids.contains(&p.id) {
                Source::Override
            } else {
                Source::User
            };
            out.push((p, source));
        }
        out
    }

    pub fn get(&self, id: &str) -> Option<(Preset, Source)> {
        self.all().into_iter().find(|(p, _)| p.id == id)
    }

    pub fn save(&self, preset: &Preset) -> Result<(), String> {
        preset.validate()?;
        self.ensure_dir()?;
        let path = self.path_for(&preset.id);
        let json = preset.to_json().map_err(|e| e.to_string())?;
        // Write to a temporary file and rename, so an interrupted save cannot
        // leave a truncated preset behind.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("could not write preset: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("could not save preset: {e}"))
    }

    /// Remove a user preset. A factory id reverts to the shipped version
    /// rather than disappearing.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let path = self.path_for(id);
        if !path.exists() {
            return Err(format!("'{id}' is not a saved preset"));
        }
        std::fs::remove_file(&path).map_err(|e| format!("could not delete preset: {e}"))
    }

    /// An id derived from `name` that no existing preset is using.
    pub fn unique_id(&self, name: &str) -> String {
        let base = Self::sanitise_id(name);
        let taken: Vec<String> = self.all().into_iter().map(|(p, _)| p.id).collect();
        if !taken.contains(&base) {
            return base;
        }
        (2..).map(|n| format!("{base}-{n}")).find(|c| !taken.contains(c)).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use truezen_engine::layer::LayerConfig;

    /// A directory of this test's own. Tests run in parallel, so a shared one
    /// would have them deleting each other's files.
    fn temp_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("truezen-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::new(dir)
    }

    fn preset(id: &str, name: &str) -> Preset {
        Preset {
            id: id.into(),
            name: name.into(),
            description: "test".into(),
            layers: vec![LayerConfig::default()],
            ..Default::default()
        }
    }

    #[test]
    fn ids_cannot_escape_the_preset_directory() {
        assert_eq!(Store::sanitise_id("../../etc/passwd"), "etc-passwd");
        assert_eq!(Store::sanitise_id("My Preset!!"), "my-preset");
        assert_eq!(Store::sanitise_id("  "), "preset");
        assert_eq!(Store::sanitise_id("a/b\\c"), "a-b-c");
        assert!(!Store::sanitise_id("....").contains('.'));
    }

    #[test]
    fn saves_and_reads_back() {
        let s = temp_store("roundtrip");
        s.save(&preset("mine", "Mine")).unwrap();
        let (got, source) = s.get("mine").unwrap();
        assert_eq!(got.name, "Mine");
        assert_eq!(source, Source::User);
        let _ = std::fs::remove_dir_all(s.dir());
    }

    /// A user preset saved over a factory id must hide the original exactly
    /// once, and be reported as an override so it can be reverted.
    #[test]
    fn a_user_preset_overrides_the_factory_one() {
        let s = temp_store("override");
        let mut p = preset("deep-theta", "My Deep Theta");
        p.description = "edited".into();
        s.save(&p).unwrap();

        let all = s.all();
        let matches: Vec<_> = all.iter().filter(|(x, _)| x.id == "deep-theta").collect();
        assert_eq!(matches.len(), 1, "factory preset was not hidden");
        assert_eq!(matches[0].0.name, "My Deep Theta");
        assert_eq!(matches[0].1, Source::Override);

        s.delete("deep-theta").unwrap();
        let (reverted, source) = s.get("deep-theta").unwrap();
        assert_eq!(source, Source::Factory);
        assert_eq!(reverted.name, "Deep Theta", "deleting did not restore the original");
        let _ = std::fs::remove_dir_all(s.dir());
    }

    #[test]
    fn unique_id_avoids_collisions() {
        let s = temp_store("unique");
        assert_eq!(s.unique_id("Deep Theta"), "deep-theta-2", "collided with a factory id");
        s.save(&preset("my-thing", "My Thing")).unwrap();
        assert_eq!(s.unique_id("My Thing"), "my-thing-2");
        let _ = std::fs::remove_dir_all(s.dir());
    }

    #[test]
    fn invalid_presets_are_refused_and_bad_files_are_skipped() {
        let s = temp_store("invalid");
        let mut bad = preset("bad", "Bad");
        bad.layers.clear();
        assert!(s.save(&bad).is_err(), "saved a preset with no layers");

        s.save(&preset("good", "Good")).unwrap();
        std::fs::write(s.dir().join("junk.json"), "{ not json").unwrap();
        let listed = s.user_presets();
        assert_eq!(listed.len(), 1, "a malformed file broke the listing");
        let _ = std::fs::remove_dir_all(s.dir());
    }

    #[test]
    fn deleting_something_that_was_never_saved_is_an_error() {
        let s = temp_store("missing");
        assert!(s.delete("nope").is_err());
        let _ = std::fs::remove_dir_all(s.dir());
    }
}
