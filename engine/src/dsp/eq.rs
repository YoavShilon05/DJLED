//! Parametric EQ over band magnitudes.
//!
//! No audio is filtered here. The analyser has already reduced the signal to a
//! magnitude per band, so the EQ is applied as a gain curve sampled at each
//! band's centre frequency — which is what a filter's magnitude response *is*.
//!
//! The shape is therefore the entire specification, and it is the RBJ cookbook
//! biquad rather than an invented bell, so the curve the editor draws and the
//! gain applied here come from the same closed form. `ui/src/config/eq.ts` is
//! the other half of that pair; the two are asserted against the same
//! properties on both sides.
//!
//! # Where this sits in the chain
//!
//! After AGC, before the range map. Both ends matter:
//!
//! - **After AGC**, because the AGC pushes every band toward a reference level.
//!   An EQ applied before it would be treated as drift and quietly undone over
//!   the AGC time constant.
//! - **Before the range map**, because that is what keeps a boost from lifting
//!   silence. A silent band sits far below the dB floor; adding 12 dB leaves it
//!   far below the dB floor. Applied to the normalised level instead, where
//!   zero *is* the floor, the same boost would make an idle strip glow.

use serde::{Deserialize, Serialize};

pub const Q_MIN: f32 = 0.1;
pub const Q_MAX: f32 = 12.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EqType {
    /// Peaking bell.
    #[default]
    Peak,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
}

impl EqType {
    /// The pass filters cut rather than gain, so their `gain` field is ignored.
    pub fn has_gain(self) -> bool {
        matches!(self, EqType::Peak | EqType::LowShelf | EqType::HighShelf)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EqBand {
    #[serde(rename = "type")]
    pub kind: EqType,
    pub hz: f32,
    /// dB. Ignored when [`EqType::has_gain`] is false.
    pub gain: f32,
    pub q: f32,
}

impl Default for EqBand {
    fn default() -> Self {
        Self { kind: EqType::Peak, hz: 1000.0, gain: 0.0, q: 1.0 }
    }
}

/// Unnormalised biquad coefficients. `a0` is kept rather than divided out
/// because the magnitude is a ratio of two polynomials and needs it.
#[derive(Clone, Copy, Debug)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a0: f32,
    a1: f32,
    a2: f32,
}

/// RBJ Audio EQ Cookbook.
fn coefficients(band: &EqBand, sample_rate: f32) -> Coefficients {
    let nyquist = sample_rate / 2.0;
    let f0 = band.hz.clamp(1.0, nyquist * 0.999);
    let q = band.q.clamp(Q_MIN, Q_MAX);
    let gain = if band.kind.has_gain() { band.gain } else { 0.0 };

    let w0 = std::f32::consts::TAU * f0 / sample_rate;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    let a = 10f32.powf(gain / 40.0);
    let sqrt_a = a.sqrt();

    match band.kind {
        EqType::Peak => Coefficients {
            b0: 1.0 + alpha * a,
            b1: -2.0 * cos,
            b2: 1.0 - alpha * a,
            a0: 1.0 + alpha / a,
            a1: -2.0 * cos,
            a2: 1.0 - alpha / a,
        },
        EqType::LowShelf => Coefficients {
            b0: a * (a + 1.0 - (a - 1.0) * cos + 2.0 * sqrt_a * alpha),
            b1: 2.0 * a * (a - 1.0 - (a + 1.0) * cos),
            b2: a * (a + 1.0 - (a - 1.0) * cos - 2.0 * sqrt_a * alpha),
            a0: a + 1.0 + (a - 1.0) * cos + 2.0 * sqrt_a * alpha,
            a1: -2.0 * (a - 1.0 + (a + 1.0) * cos),
            a2: a + 1.0 + (a - 1.0) * cos - 2.0 * sqrt_a * alpha,
        },
        EqType::HighShelf => Coefficients {
            b0: a * (a + 1.0 + (a - 1.0) * cos + 2.0 * sqrt_a * alpha),
            b1: -2.0 * a * (a - 1.0 + (a + 1.0) * cos),
            b2: a * (a + 1.0 + (a - 1.0) * cos - 2.0 * sqrt_a * alpha),
            a0: a + 1.0 - (a - 1.0) * cos + 2.0 * sqrt_a * alpha,
            a1: 2.0 * (a - 1.0 - (a + 1.0) * cos),
            a2: a + 1.0 - (a - 1.0) * cos - 2.0 * sqrt_a * alpha,
        },
        EqType::LowPass => Coefficients {
            b0: (1.0 - cos) / 2.0,
            b1: 1.0 - cos,
            b2: (1.0 - cos) / 2.0,
            a0: 1.0 + alpha,
            a1: -2.0 * cos,
            a2: 1.0 - alpha,
        },
        EqType::HighPass => Coefficients {
            b0: (1.0 + cos) / 2.0,
            b1: -(1.0 + cos),
            b2: (1.0 + cos) / 2.0,
            a0: 1.0 + alpha,
            a1: -2.0 * cos,
            a2: 1.0 - alpha,
        },
    }
}

/// `20·log10 |H(e^jw)|`, floored rather than allowed to reach negative infinity.
fn magnitude_db(c: &Coefficients, hz: f32, sample_rate: f32) -> f32 {
    let w = std::f32::consts::TAU * hz.min(sample_rate / 2.0) / sample_rate;
    let (sin1, cos1) = w.sin_cos();
    let (sin2, cos2) = (2.0 * w).sin_cos();

    let num = (c.b0 + c.b1 * cos1 + c.b2 * cos2).hypot(-(c.b1 * sin1 + c.b2 * sin2));
    let den = (c.a0 + c.a1 * cos1 + c.a2 * cos2).hypot(-(c.a1 * sin1 + c.a2 * sin2));
    if den == 0.0 {
        return 0.0;
    }
    (20.0 * (num / den).max(1e-6).log10()).max(-90.0)
}

/// One band's contribution on its own, for tests and for the editor's solo view.
pub fn band_gain_db(band: &EqBand, hz: f32, sample_rate: f32) -> f32 {
    magnitude_db(&coefficients(band, sample_rate), hz, sample_rate)
}

/// The combined response of a cascade.
#[derive(Clone, Debug, Default)]
pub struct EqCurve {
    sections: Vec<Coefficients>,
    sample_rate: f32,
}

impl EqCurve {
    pub fn new(bands: &[EqBand], sample_rate: f32) -> Self {
        Self {
            sections: bands.iter().map(|b| coefficients(b, sample_rate)).collect(),
            sample_rate,
        }
    }

    pub fn is_flat(&self) -> bool {
        self.sections.is_empty()
    }

    /// Total gain in dB. Cascaded filters multiply in magnitude, so they add here.
    pub fn gain_db(&self, hz: f32) -> f32 {
        self.sections
            .iter()
            .map(|c| magnitude_db(c, hz, self.sample_rate))
            .sum()
    }

    /// Pre-sampled at each band centre, so the per-frame cost is a slice read.
    pub fn table(&self, centers: &[f32]) -> Vec<f32> {
        if self.is_flat() {
            return vec![0.0; centers.len()];
        }
        centers.iter().map(|&f| self.gain_db(f)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn band(kind: EqType, hz: f32, gain: f32, q: f32) -> EqBand {
        EqBand { kind, hz, gain, q }
    }

    /// Closed-form properties, not values captured from a previous run: a
    /// transcription slip in the coefficients fails these rather than being
    /// baked in as the new expectation.
    #[test]
    fn bell_reaches_its_full_gain_at_centre() {
        // |H(w0)| = A^2 for the peaking filter, i.e. exactly the requested dB.
        for gain in [-18.0, -6.0, 3.0, 12.0] {
            let g = band_gain_db(&band(EqType::Peak, 1000.0, gain, 1.4), 1000.0, SR);
            assert!((g - gain).abs() < 1e-3, "asked {gain} dB, got {g}");
        }
    }

    #[test]
    fn bell_leaves_distant_bands_alone() {
        let b = band(EqType::Peak, 1000.0, 12.0, 2.0);
        assert!(band_gain_db(&b, 40.0, SR).abs() < 0.05);
        assert!(band_gain_db(&b, 18_000.0, SR).abs() < 0.2);
    }

    #[test]
    fn lower_q_widens_the_bell() {
        let at = |q| band_gain_db(&band(EqType::Peak, 1000.0, 12.0, q), 500.0, SR);
        assert!(at(0.3) > at(1.0));
        assert!(at(1.0) > at(6.0));
    }

    #[test]
    fn shelves_lift_one_side_only() {
        let low = band(EqType::LowShelf, 500.0, 9.0, 0.707);
        assert!((band_gain_db(&low, 25.0, SR) - 9.0).abs() < 0.1);
        assert!(band_gain_db(&low, 15_000.0, SR).abs() < 0.1);

        let high = band(EqType::HighShelf, 2000.0, -9.0, 0.707);
        assert!((band_gain_db(&high, 15_000.0, SR) + 9.0).abs() < 0.1);
        assert!(band_gain_db(&high, 25.0, SR).abs() < 0.1);
    }

    #[test]
    fn pass_filters_are_three_db_down_at_cutoff() {
        // |H(w0)| = Q for both pass types, so Q = 1/sqrt(2) is the -3.01 dB point.
        let q = std::f32::consts::FRAC_1_SQRT_2;
        for kind in [EqType::LowPass, EqType::HighPass] {
            let g = band_gain_db(&band(kind, 1000.0, 0.0, q), 1000.0, SR);
            assert!((g + 3.0103).abs() < 1e-2, "{kind:?} was {g} dB at cutoff");
        }
    }

    #[test]
    fn pass_filters_cut_the_far_side() {
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let low = band(EqType::LowPass, 1000.0, 0.0, q);
        assert!(band_gain_db(&low, 100.0, SR).abs() < 0.05);
        assert!(band_gain_db(&low, 8000.0, SR) < -30.0);

        let high = band(EqType::HighPass, 1000.0, 0.0, q);
        assert!(band_gain_db(&high, 8000.0, SR).abs() < 0.05);
        assert!(band_gain_db(&high, 100.0, SR) < -30.0);
    }

    #[test]
    fn cascaded_bands_add_in_db() {
        let a = band(EqType::Peak, 200.0, 6.0, 1.0);
        let b = band(EqType::Peak, 5000.0, -4.0, 1.0);
        let curve = EqCurve::new(&[a.clone(), b.clone()], SR);
        for hz in [50.0, 200.0, 1000.0, 5000.0, 16_000.0] {
            let expected = band_gain_db(&a, hz, SR) + band_gain_db(&b, hz, SR);
            assert!((curve.gain_db(hz) - expected).abs() < 1e-4);
        }
    }

    #[test]
    fn empty_cascade_is_flat() {
        let curve = EqCurve::new(&[], SR);
        assert!(curve.is_flat());
        assert_eq!(curve.gain_db(1000.0), 0.0);
        assert_eq!(curve.table(&[100.0, 1000.0]), vec![0.0, 0.0]);
    }

    /// The editor's axis reaches 20 kHz, which is above Nyquist at 32 kHz. A
    /// band parked up there must not produce NaN and poison every level.
    #[test]
    fn survives_a_band_above_nyquist() {
        for kind in [EqType::Peak, EqType::LowPass, EqType::HighShelf] {
            let g = band_gain_db(&band(kind, 20_000.0, 12.0, 4.0), 20_000.0, 32_000.0);
            assert!(g.is_finite(), "{kind:?} produced {g}");
        }
    }

    #[test]
    fn q_outside_the_usable_range_is_clamped_not_exploded() {
        for q in [0.0, -1.0, 1e9] {
            let g = band_gain_db(&band(EqType::Peak, 1000.0, 6.0, q), 1000.0, SR);
            assert!(g.is_finite(), "Q {q} produced {g}");
        }
    }

    #[test]
    fn parses_the_editor_wire_format() {
        let json = r#"{"id":"eq-1","type":"highShelf","hz":2000,"gain":-6,"q":0.7}"#;
        let band: EqBand = serde_json::from_str(json).unwrap();
        assert_eq!(band.kind, EqType::HighShelf);
        assert_eq!(band.hz, 2000.0);
        assert_eq!(band.gain, -6.0);
    }
}
