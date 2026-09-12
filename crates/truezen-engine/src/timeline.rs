//! Session automation.
//!
//! A preset is not a static setting -- a meditation session is a journey, e.g.
//! settle at 10 Hz, descend to 4.5 Hz over twenty minutes, hold, then come
//! back up before it ends. A `Timeline` describes that shape.

use serde::{Deserialize, Serialize};

/// How a breakpoint is approached from the one before it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    /// Constant until the breakpoint's time, then a step.
    Hold,
    Linear,
    /// Geometric interpolation. **The right default for frequency**: pitch and
    /// rate perception are logarithmic, so a linear 10 -> 4 Hz sweep spends
    /// most of its time in the bottom of the range and feels front-loaded.
    #[default]
    Exponential,
    /// Ease in and out. Good for gains, where a linear fade has audible
    /// corners at each end.
    Smoothstep,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct Breakpoint {
    pub at_s: f64,
    pub value: f64,
    #[serde(default)]
    pub curve: Curve,
}

impl Breakpoint {
    pub fn new(at_s: f64, value: f64, curve: Curve) -> Self {
        Self { at_s, value, curve }
    }
}

/// Which parameter of which layer a track drives.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerParam {
    Carrier,
    Beat,
    Gain,
    Pan,
    FilterCutoff,
    /// Isochronic gate shape. Applied at cycle boundaries, not immediately --
    /// shrinking the duty past the current position would otherwise slam the
    /// gate shut mid-pulse and click.
    Duty,
    RampMs,
    Depth,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ParamTarget {
    MasterGain,
    Layer { index: usize, param: LayerParam },
}

/// One automated parameter and its breakpoints.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub target: ParamTarget,
    pub points: Vec<Breakpoint>,
}

impl Track {
    pub fn new(target: ParamTarget, points: Vec<Breakpoint>) -> Self {
        let mut t = Self { target, points };
        t.sort();
        t
    }

    /// Breakpoints must be time-ordered for `value_at` to be correct; a
    /// hand-edited preset file may not be.
    pub fn sort(&mut self) {
        self.points
            .sort_by(|a, b| a.at_s.partial_cmp(&b.at_s).unwrap_or(std::cmp::Ordering::Equal));
    }

    pub fn end_s(&self) -> f64 {
        self.points.last().map_or(0.0, |p| p.at_s)
    }

    /// Value of this track at time `t`. Clamps to the first and last
    /// breakpoint outside the defined range.
    pub fn value_at(&self, t: f64) -> Option<f64> {
        let first = self.points.first()?;
        if t <= first.at_s {
            return Some(first.value);
        }
        let last = self.points.last()?;
        if t >= last.at_s {
            return Some(last.value);
        }

        // Linear scan: tracks hold a handful of breakpoints, so this beats a
        // binary search and stays branch-predictable.
        let idx = self.points.iter().position(|p| p.at_s > t)?;
        let a = &self.points[idx - 1];
        let b = &self.points[idx];

        let span = b.at_s - a.at_s;
        if span <= 0.0 {
            return Some(b.value);
        }
        let u = ((t - a.at_s) / span).clamp(0.0, 1.0);
        Some(interpolate(a.value, b.value, u, b.curve))
    }
}

#[inline]
fn interpolate(a: f64, b: f64, u: f64, curve: Curve) -> f64 {
    match curve {
        Curve::Hold => a,
        Curve::Linear => a + (b - a) * u,
        Curve::Smoothstep => {
            let s = u * u * (3.0 - 2.0 * u);
            a + (b - a) * s
        }
        Curve::Exponential => {
            // Geometric interpolation is undefined through zero, which a gain
            // track will hit; fall back to linear there.
            if a > 0.0 && b > 0.0 {
                a * (b / a).powf(u)
            } else {
                a + (b - a) * u
            }
        }
    }
}

/// The full automation set for a session.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Timeline {
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// Explicit session length. When absent, the last breakpoint defines it.
    #[serde(default)]
    pub duration_s: Option<f64>,
    /// Fade to silence over this long at the end. Ending a delta-wave sleep
    /// session on an abrupt cut defeats the point.
    #[serde(default)]
    pub fade_out_s: f64,
}

impl Timeline {
    pub fn duration(&self) -> f64 {
        self.duration_s.unwrap_or_else(|| {
            self.tracks
                .iter()
                .map(Track::end_s)
                .fold(0.0f64, f64::max)
        })
    }

    /// Master fade multiplier at time `t`.
    pub fn fade_gain_at(&self, t: f64) -> f64 {
        let dur = self.duration();
        if self.fade_out_s <= 0.0 || dur <= 0.0 {
            return 1.0;
        }
        let fade_start = dur - self.fade_out_s;
        if t <= fade_start {
            1.0
        } else if t >= dur {
            0.0
        } else {
            let u = (t - fade_start) / self.fade_out_s;
            // Equal-power taper; a linear fade sounds like it stalls near the end.
            ((1.0 - u) * std::f64::consts::FRAC_PI_2).sin()
        }
    }

    /// Reject a timeline that would misbehave once installed. Curves arrive
    /// from the UI and from hand-edited preset files, so this runs on every
    /// swap rather than trusting the caller.
    pub fn validate(&self, layer_count: usize) -> Result<(), String> {
        if let Some(d) = self.duration_s {
            if !d.is_finite() || d < 0.0 {
                return Err(format!("duration {d} is not a valid length"));
            }
        }
        if !self.fade_out_s.is_finite() || self.fade_out_s < 0.0 {
            return Err(format!("fade-out {} is not a valid length", self.fade_out_s));
        }
        for (i, track) in self.tracks.iter().enumerate() {
            if let ParamTarget::Layer { index, .. } = track.target {
                if index >= layer_count {
                    return Err(format!("track {i} targets layer {index}, which does not exist"));
                }
            }
            if track.points.is_empty() {
                return Err(format!("track {i} has no breakpoints"));
            }
            for p in &track.points {
                if !p.at_s.is_finite() || p.at_s < 0.0 || !p.value.is_finite() {
                    return Err(format!("track {i} has an invalid breakpoint"));
                }
            }
        }
        Ok(())
    }

    pub fn is_finished(&self, t: f64) -> bool {
        let dur = self.duration();
        dur > 0.0 && t >= dur
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(points: Vec<Breakpoint>) -> Track {
        Track::new(ParamTarget::MasterGain, points)
    }

    #[test]
    fn clamps_outside_the_defined_range() {
        let t = track(vec![
            Breakpoint::new(10.0, 2.0, Curve::Linear),
            Breakpoint::new(20.0, 8.0, Curve::Linear),
        ]);
        assert_eq!(t.value_at(0.0), Some(2.0));
        assert_eq!(t.value_at(100.0), Some(8.0));
    }

    #[test]
    fn linear_hits_the_midpoint() {
        let t = track(vec![
            Breakpoint::new(0.0, 0.0, Curve::Linear),
            Breakpoint::new(10.0, 10.0, Curve::Linear),
        ]);
        assert!((t.value_at(5.0).unwrap() - 5.0).abs() < 1e-12);
    }

    /// The reason exponential is the default: a 10 -> 2.5 Hz descent should
    /// pass through the geometric mean (5 Hz) at the halfway point, not the
    /// arithmetic mean (6.25 Hz).
    #[test]
    fn exponential_uses_the_geometric_mean() {
        let t = track(vec![
            Breakpoint::new(0.0, 10.0, Curve::Exponential),
            Breakpoint::new(10.0, 2.5, Curve::Exponential),
        ]);
        assert!((t.value_at(5.0).unwrap() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn exponential_falls_back_to_linear_through_zero() {
        let t = track(vec![
            Breakpoint::new(0.0, 0.0, Curve::Exponential),
            Breakpoint::new(10.0, 1.0, Curve::Exponential),
        ]);
        let v = t.value_at(5.0).unwrap();
        assert!(v.is_finite() && (v - 0.5).abs() < 1e-12, "got {v}");
    }

    #[test]
    fn hold_steps_at_the_breakpoint() {
        let t = track(vec![
            Breakpoint::new(0.0, 1.0, Curve::Hold),
            Breakpoint::new(10.0, 5.0, Curve::Hold),
        ]);
        assert_eq!(t.value_at(9.99), Some(1.0));
        assert_eq!(t.value_at(10.0), Some(5.0));
    }

    #[test]
    fn unsorted_breakpoints_are_repaired_on_construction() {
        let t = track(vec![
            Breakpoint::new(20.0, 8.0, Curve::Linear),
            Breakpoint::new(0.0, 0.0, Curve::Linear),
        ]);
        assert_eq!(t.value_at(0.0), Some(0.0));
        assert_eq!(t.value_at(20.0), Some(8.0));
    }

    #[test]
    fn fade_out_reaches_silence_exactly_at_the_end() {
        let tl = Timeline {
            tracks: vec![track(vec![
                Breakpoint::new(0.0, 1.0, Curve::Linear),
                Breakpoint::new(60.0, 1.0, Curve::Linear),
            ])],
            duration_s: Some(60.0),
            fade_out_s: 10.0,
        };
        assert_eq!(tl.fade_gain_at(0.0), 1.0);
        assert_eq!(tl.fade_gain_at(50.0), 1.0);
        assert!(tl.fade_gain_at(55.0) < 1.0 && tl.fade_gain_at(55.0) > 0.0);
        assert_eq!(tl.fade_gain_at(60.0), 0.0);
        assert!(tl.is_finished(60.0));
    }
}
