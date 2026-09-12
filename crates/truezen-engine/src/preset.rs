//! The on-disk preset format.
//!
//! Presets are plain JSON: human-readable, diffable, and shareable as a single
//! file with no server involved. Every field has a `serde` default so a preset
//! saved by an older build still loads.

use serde::{Deserialize, Serialize};

use crate::layer::LayerConfig;
use crate::timeline::Timeline;

/// Bump only for changes that older builds cannot interpret. Additive fields
/// do not need it -- that is what the defaults are for.
pub const SCHEMA_VERSION: u32 = 1;

fn schema_version() -> u32 {
    SCHEMA_VERSION
}
fn default_master_gain() -> f64 {
    0.7
}

/// Broad intent, used to group the library in the UI.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Goal {
    Focus,
    #[default]
    Meditation,
    Sleep,
    Relaxation,
    Exploration,
    Energy,
}

impl Goal {
    pub fn label(&self) -> &'static str {
        match self {
            Goal::Focus => "Focus",
            Goal::Meditation => "Meditation",
            Goal::Sleep => "Sleep",
            Goal::Relaxation => "Relaxation",
            Goal::Exploration => "Exploration",
            Goal::Energy => "Energy",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub goal: Goal,
    /// Plain-language description of what the preset is for. Deliberately
    /// framed as an intention, never as a clinical claim -- the evidence for
    /// entrainment is real but modest, and the copy should not overstate it.
    #[serde(default)]
    pub description: String,
    /// True when the preset relies on binaural layers, which do not work on
    /// speakers. The UI surfaces this before playback starts.
    #[serde(default)]
    pub requires_headphones: bool,
    #[serde(default = "default_master_gain")]
    pub master_gain: f64,
    #[serde(default)]
    pub layers: Vec<LayerConfig>,
    #[serde(default)]
    pub timeline: Option<Timeline>,
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            id: String::new(),
            name: "Untitled".into(),
            goal: Goal::default(),
            description: String::new(),
            requires_headphones: false,
            master_gain: default_master_gain(),
            layers: Vec::new(),
            timeline: None,
        }
    }
}

impl Preset {
    /// Total length in seconds; 0 means it runs until stopped.
    pub fn duration_s(&self) -> f64 {
        self.timeline.as_ref().map_or(0.0, Timeline::duration)
    }

    /// Recompute `requires_headphones` from the layer stack, so the flag can
    /// never drift out of sync with the actual content.
    pub fn refresh_headphone_flag(&mut self) {
        self.requires_headphones = self
            .layers
            .iter()
            .any(|l| l.enabled && l.kind == crate::layer::LayerKind::Binaural);
    }

    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        let mut p: Preset = serde_json::from_str(s)?;
        if let Some(tl) = p.timeline.as_mut() {
            for track in tl.tracks.iter_mut() {
                track.sort();
            }
        }
        Ok(p)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Reject values that would produce silence, a blast of noise, or NaN.
    /// Presets can be hand-edited or imported from elsewhere, so this runs on
    /// every load rather than trusting the file.
    pub fn validate(&self) -> Result<(), String> {
        if self.layers.is_empty() {
            return Err("preset has no layers".into());
        }
        if !self.master_gain.is_finite() || !(0.0..=1.0).contains(&self.master_gain) {
            return Err(format!("master_gain {} out of range 0..1", self.master_gain));
        }
        for (i, l) in self.layers.iter().enumerate() {
            let where_ = format!("layer {i} ({})", l.name);
            if !l.carrier_hz.is_finite() || !(1.0..=20_000.0).contains(&l.carrier_hz) {
                return Err(format!("{where_}: carrier {} out of range", l.carrier_hz));
            }
            if !l.beat_hz.is_finite() || !(0.0..=200.0).contains(&l.beat_hz) {
                return Err(format!("{where_}: beat {} out of range", l.beat_hz));
            }
            if !l.gain.is_finite() || !(0.0..=1.0).contains(&l.gain) {
                return Err(format!("{where_}: gain {} out of range", l.gain));
            }
            if !l.pan.is_finite() || !(-1.0..=1.0).contains(&l.pan) {
                return Err(format!("{where_}: pan {} out of range", l.pan));
            }
        }
        if let Some(tl) = &self.timeline {
            for (i, track) in tl.tracks.iter().enumerate() {
                if let crate::timeline::ParamTarget::Layer { index, .. } = track.target {
                    if index >= self.layers.len() {
                        return Err(format!("track {i} targets missing layer {index}"));
                    }
                }
                for p in &track.points {
                    if !p.at_s.is_finite() || p.at_s < 0.0 || !p.value.is_finite() {
                        return Err(format!("track {i} has an invalid breakpoint"));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{LayerConfig, LayerKind};

    fn sample() -> Preset {
        Preset {
            id: "test".into(),
            name: "Test".into(),
            layers: vec![LayerConfig {
                kind: LayerKind::Binaural,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn round_trips_through_json() {
        let p = sample();
        let back = Preset::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(back.id, p.id);
        assert_eq!(back.layers.len(), 1);
        assert_eq!(back.layers[0].kind, LayerKind::Binaural);
    }

    /// The forward-compatibility guarantee: a minimal file must still load.
    #[test]
    fn loads_a_preset_missing_every_optional_field() {
        let json = r#"{ "id": "min", "name": "Minimal", "layers": [{}] }"#;
        let p = Preset::from_json(json).unwrap();
        assert_eq!(p.schema_version, SCHEMA_VERSION);
        assert_eq!(p.layers.len(), 1);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn headphone_flag_follows_the_layers() {
        let mut p = sample();
        p.refresh_headphone_flag();
        assert!(p.requires_headphones);
        p.layers[0].kind = LayerKind::Isochronic;
        p.refresh_headphone_flag();
        assert!(!p.requires_headphones);
    }

    #[test]
    fn validate_rejects_nonsense() {
        let mut p = sample();
        p.layers[0].carrier_hz = f64::NAN;
        assert!(p.validate().is_err());

        let mut p = sample();
        p.master_gain = 40.0;
        assert!(p.validate().is_err());

        let mut p = sample();
        p.layers.clear();
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_rejects_a_track_pointing_at_a_missing_layer() {
        use crate::timeline::{Breakpoint, Curve, LayerParam, ParamTarget, Timeline, Track};
        let mut p = sample();
        p.timeline = Some(Timeline {
            tracks: vec![Track::new(
                ParamTarget::Layer {
                    index: 9,
                    param: LayerParam::Beat,
                },
                vec![Breakpoint::new(0.0, 10.0, Curve::Linear)],
            )],
            ..Default::default()
        });
        assert!(p.validate().is_err());
    }
}
