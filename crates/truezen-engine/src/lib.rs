//! Real-time-safe brainwave entrainment synthesis.
//!
//! The crate deliberately has no audio-device, filesystem or UI dependency:
//! [`engine::Engine::process`] is the whole output interface, which is what
//! lets one implementation serve live playback, offline rendering and tests.

pub mod engine;
pub mod factory;
pub mod layer;
pub mod mixer;
pub mod noise;
pub mod osc;
pub mod preset;
pub mod smooth;
pub mod timeline;

pub use engine::{Command, Engine, Meters, SessionState, Transport};
pub use layer::{Layer, LayerConfig, LayerKind};
pub use preset::{Goal, Preset};
pub use timeline::{Breakpoint, Curve, LayerParam, ParamTarget, Timeline, Track};
