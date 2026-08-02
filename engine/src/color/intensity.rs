//! Level to brightness: the threshold, the clamp, and the curve between them.
//!
//! # Which dB is this?
//!
//! The editor's display axis, not dBFS. By the time a level exists it has been
//! through floor subtraction, tilt, AGC and ballistics, so there is no dBFS
//! left to speak of — the number the user drags a threshold to is a position on
//! the axis they are looking at, and this maps it back the same way.
//!
//! The EQ deliberately does *not* use this scale: it is a signal-side control
//! and works in the analyser's real dB, upstream of normalisation. See
//! [`crate::dsp::eq`].

use serde::{Deserialize, Serialize};

/// Bottom of the editor's dB axis.
pub const DISPLAY_DB_MIN: f32 = -80.0;
/// Top of the editor's dB axis.
pub const DISPLAY_DB_MAX: f32 = 0.0;

pub fn level_to_db(level: f32) -> f32 {
    DISPLAY_DB_MIN + level * (DISPLAY_DB_MAX - DISPLAY_DB_MIN)
}

pub fn db_to_level(db: f32) -> f32 {
    (db - DISPLAY_DB_MIN) / (DISPLAY_DB_MAX - DISPLAY_DB_MIN)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CurveKind {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// The only kind with user-editable control points.
    Bezier,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// Every kind is a cubic Bézier with endpoints pinned at (0,0) and (1,1); the
/// presets are fixed control points, which is why only `Bezier` shows handles.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IntensityCurve {
    #[serde(rename = "type")]
    pub kind: CurveKind,
    pub p1: Point,
    pub p2: Point,
}

impl Default for IntensityCurve {
    fn default() -> Self {
        Self {
            kind: CurveKind::Linear,
            p1: Point { x: 0.25, y: 0.1 },
            p2: Point { x: 0.25, y: 1.0 },
        }
    }
}

impl IntensityCurve {
    fn control_points(&self) -> (Point, Point) {
        let p = |x, y| Point { x, y };
        match self.kind {
            CurveKind::Linear => (p(1.0 / 3.0, 1.0 / 3.0), p(2.0 / 3.0, 2.0 / 3.0)),
            CurveKind::EaseIn => (p(0.42, 0.0), p(1.0, 1.0)),
            CurveKind::EaseOut => (p(0.0, 0.0), p(0.58, 1.0)),
            CurveKind::EaseInOut => (p(0.42, 0.0), p(0.58, 1.0)),
            CurveKind::Bezier => (self.p1, self.p2),
        }
    }

    /// Brightness for a position within the threshold→clamp window.
    ///
    /// A Bézier is parametric, so x is inverted before y can be read. Bisection
    /// rather than Newton: the derivative vanishes at the ends of `EaseIn` and
    /// `EaseOut`, where Newton stalls, and 24 halvings is exact well below what
    /// eight bits can show.
    pub fn evaluate(&self, x: f32) -> f32 {
        // NaN handled explicitly rather than by inverting the comparison,
        // which reads as a typo.
        if x.is_nan() || x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        let (p1, p2) = self.control_points();

        let (mut lo, mut hi, mut t) = (0.0f32, 1.0f32, x);
        for _ in 0..24 {
            if cubic(p1.x, p2.x, t) < x {
                lo = t;
            } else {
                hi = t;
            }
            t = 0.5 * (lo + hi);
        }
        cubic(p1.y, p2.y, t).clamp(0.0, 1.0)
    }
}

/// Cubic Bézier with the endpoints pinned at 0 and 1, so their terms collapse.
fn cubic(a: f32, b: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
}

/// The dB window a band is lit across.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IntensityConfig {
    /// Below this the band is dark.
    pub threshold: f32,
    /// At and above this the band is at full brightness.
    pub clamp: f32,
    pub curve: IntensityCurve,
}

impl Default for IntensityConfig {
    fn default() -> Self {
        Self { threshold: -62.0, clamp: -6.0, curve: IntensityCurve::default() }
    }
}

impl IntensityConfig {
    /// Full brightness for anything above silence — the intensity stage doing
    /// nothing. Silence still goes dark, which is a property of the threshold
    /// rather than something to opt out of.
    pub fn pass_through() -> Self {
        Self {
            threshold: DISPLAY_DB_MIN,
            clamp: DISPLAY_DB_MIN,
            curve: IntensityCurve::default(),
        }
    }

    /// Brightness, 0..1, for a normalised band level.
    pub fn brightness(&self, level: f32) -> f32 {
        if !level.is_finite() {
            return 0.0;
        }
        let db = level_to_db(level.clamp(0.0, 1.0));
        if db <= self.threshold {
            return 0.0;
        }
        let window = self.clamp - self.threshold;
        if window <= 0.0 {
            // Degenerate but reachable while a handle is being dragged; a
            // vanishing window means "everything above the threshold is full".
            return 1.0;
        }
        self.curve.evaluate((db - self.threshold) / window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_pin_both_endpoints() {
        for kind in [
            CurveKind::Linear,
            CurveKind::EaseIn,
            CurveKind::EaseOut,
            CurveKind::EaseInOut,
            CurveKind::Bezier,
        ] {
            let c = IntensityCurve { kind, ..Default::default() };
            assert_eq!(c.evaluate(0.0), 0.0, "{kind:?} did not start at zero");
            assert_eq!(c.evaluate(1.0), 1.0, "{kind:?} did not reach one");
        }
    }

    #[test]
    fn linear_is_actually_linear() {
        let c = IntensityCurve { kind: CurveKind::Linear, ..Default::default() };
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            assert!((c.evaluate(x) - x).abs() < 1e-3, "linear bent at {x}");
        }
    }

    #[test]
    fn curves_are_monotonic() {
        for kind in [CurveKind::Linear, CurveKind::EaseIn, CurveKind::EaseOut, CurveKind::EaseInOut] {
            let c = IntensityCurve { kind, ..Default::default() };
            let mut previous = 0.0;
            for i in 0..=200 {
                let y = c.evaluate(i as f32 / 200.0);
                assert!(y + 1e-4 >= previous, "{kind:?} dipped at {i}");
                previous = y;
            }
        }
    }

    /// The defining difference between the two presets: ease-in starts slowly,
    /// ease-out starts fast. Asserted at the midpoint where it is unambiguous.
    #[test]
    fn ease_in_and_out_bend_opposite_ways() {
        let ease_in = IntensityCurve { kind: CurveKind::EaseIn, ..Default::default() };
        let ease_out = IntensityCurve { kind: CurveKind::EaseOut, ..Default::default() };
        assert!(ease_in.evaluate(0.5) < 0.5);
        assert!(ease_out.evaluate(0.5) > 0.5);
    }

    #[test]
    fn threshold_cuts_and_clamp_saturates() {
        let cfg = IntensityConfig { threshold: -60.0, clamp: -20.0, curve: Default::default() };
        assert_eq!(cfg.brightness(db_to_level(-70.0)), 0.0);
        assert_eq!(cfg.brightness(db_to_level(-60.0)), 0.0);
        assert!(cfg.brightness(db_to_level(-40.0)) > 0.4);
        assert_eq!(cfg.brightness(db_to_level(-20.0)), 1.0);
        assert_eq!(cfg.brightness(db_to_level(-5.0)), 1.0);
    }

    /// Silence must stay dark whatever the window is, or the strip glows at idle.
    #[test]
    fn silence_is_always_dark() {
        for threshold in [-79.0, -60.0, -20.0] {
            let cfg = IntensityConfig { threshold, clamp: -1.0, curve: Default::default() };
            assert_eq!(cfg.brightness(0.0), 0.0, "threshold {threshold} lit silence");
        }
    }

    #[test]
    fn survives_hostile_levels() {
        let cfg = IntensityConfig::default();
        for level in [-1.0, 2.0, f32::NAN, f32::INFINITY] {
            let b = cfg.brightness(level);
            assert!((0.0..=1.0).contains(&b), "level {level} produced {b}");
        }
    }

    #[test]
    fn inverted_window_does_not_panic() {
        let cfg = IntensityConfig { threshold: -10.0, clamp: -60.0, curve: Default::default() };
        let b = cfg.brightness(0.9);
        assert!((0.0..=1.0).contains(&b));
    }

    #[test]
    fn db_and_level_round_trip() {
        for db in [-80.0, -62.0, -30.0, 0.0] {
            assert!((level_to_db(db_to_level(db)) - db).abs() < 1e-3);
        }
    }

    #[test]
    fn parses_the_editor_wire_format() {
        let json = r#"{"type":"bezier","p1":{"x":0.2,"y":0.9},"p2":{"x":0.8,"y":0.1}}"#;
        let curve: IntensityCurve = serde_json::from_str(json).unwrap();
        assert_eq!(curve.kind, CurveKind::Bezier);
        assert_eq!(curve.p1.y, 0.9);
    }
}
