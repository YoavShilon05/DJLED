//! The 2D keyframe colour surface.
//!
//! Colour is authored as a field over the unit square: **x** is position along
//! the strip (bass on the left, treble on the right) and **y** is that band's
//! intensity. Keyframes are placed anywhere in that square, and any point
//! between them is interpolated.
//!
//! # Choice of interpolator
//!
//! Scattered 2D interpolation has three obvious candidates, and the constraints
//! here pick one cleanly:
//!
//! - **Radial basis functions** are smooth but need a K×K solve, go
//!   ill-conditioned when two keyframes land on top of each other (which a user
//!   dragging points *will* do), and overshoot outside the convex hull —
//!   producing out-of-gamut colours that then need real gamut mapping.
//! - **Inverse-distance weighting** never overshoots but has the classic
//!   bullseye artefact: a flat plateau at every control point with abrupt
//!   transitions between them. It looks bad as a colour field.
//! - **Normalised Gaussian weighting** (what this uses) is smooth everywhere,
//!   needs no matrix solve, cannot be made singular by coincident keyframes, and
//!   is a convex combination — so the result stays inside the hull of the
//!   authored colours in Oklab, and lands at worst a rounding error outside
//!   displayable RGB (see [`super::oklab::LinearRgb::gamut_excursion`]).
//!
//! The tradeoff is that it approximates rather than strictly interpolates: the
//! colour exactly at a keyframe is mostly, not purely, that keyframe's colour.
//! For a colour field that reads as an improvement, since it guarantees no
//! discontinuities anywhere.
//!
//! Evaluation is O(keyframes) — a handful of `exp` calls — so there is no lookup
//! table. Rendering a full 256×128 preview costs well under a millisecond, and
//! skipping the bake removes any chance of the cache going stale after an edit.
//!
//! # Opacity
//!
//! Keyframes carry opacity as well as colour, so the field is four channels
//! rather than three. The two are not interpolated the same way: opacity uses
//! the plain Gaussian weights, colour uses those weights scaled by opacity. See
//! [`ColorSurface::sample`] for why, and [`super::oklab`] for what opacity means
//! once it reaches the strip.

use serde::{Deserialize, Serialize};

use super::oklab::{srgb_hex_to_linear, srgba_hex_to_linear, LinearRgb, Oklab};

/// One authored control point. `color` is an sRGB hex string so saved presets
/// stay readable and hand-editable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Keyframe {
    /// Position along the strip, 0 = lowest band, 1 = highest.
    pub x: f32,
    /// Band intensity, 0 = silent, 1 = full.
    pub y: f32,
    /// sRGB hex, either `"#ff2000"` or `"#ff2000cc"` with an opacity byte.
    ///
    /// Opacity lives in the colour rather than in a field of its own, so a
    /// preset stays one readable value per keyframe and matches what CSS,
    /// every colour picker and the editor all already call a colour. Six digits
    /// means fully opaque, which is what keeps presets written before opacity
    /// existed loading unchanged.
    pub color: String,
}

impl Keyframe {
    pub fn new(x: f32, y: f32, color: &str) -> Self {
        Self { x, y, color: color.to_string() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SurfaceConfig {
    pub keyframes: Vec<Keyframe>,
    /// Blend radius. Smaller gives each keyframe a tighter region and crisper
    /// transitions; larger blurs them together. Around a quarter of the typical
    /// spacing between keyframes reads well.
    pub sigma: f32,
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self {
            // A classic EQ palette: warm at the bottom, green through the mids,
            // cool at the top, with the y=0 row near black so quiet bands go
            // dark rather than merely dim.
            keyframes: vec![
                Keyframe::new(0.0, 0.0, "#1a0000"),
                Keyframe::new(0.0, 1.0, "#ff2000"),
                Keyframe::new(0.5, 0.0, "#001a08"),
                Keyframe::new(0.5, 1.0, "#40ff60"),
                Keyframe::new(1.0, 0.0, "#000820"),
                Keyframe::new(1.0, 1.0, "#40c0ff"),
            ],
            sigma: 0.25,
        }
    }
}

struct Compiled {
    x: f32,
    y: f32,
    color: Oklab,
}

pub struct ColorSurface {
    points: Vec<Compiled>,
    /// Precomputed `1 / (2σ²)`.
    falloff: f32,
}

impl ColorSurface {
    pub fn new(cfg: &SurfaceConfig) -> Result<Self, String> {
        if cfg.keyframes.is_empty() {
            return Err("colour surface needs at least one keyframe".into());
        }

        let points = cfg
            .keyframes
            .iter()
            .map(|k| Ok(Compiled { x: k.x, y: k.y, color: parse_color(&k.color)?.to_oklab() }))
            .collect::<Result<Vec<_>, String>>()?;

        let sigma = cfg.sigma.max(1e-3);
        Ok(Self { points, falloff: 1.0 / (2.0 * sigma * sigma) })
    }

    /// Colour and opacity at a point in the unit square. Inputs outside 0..1 are
    /// clamped.
    ///
    /// Opacity interpolates on the same Gaussian weights as colour, so it is
    /// smooth for the same reasons — a keyframe faded out leaves a soft hole in
    /// the field rather than a hard-edged one.
    ///
    /// Colour, though, is weighted by `w · α` rather than `w`. See
    /// [`Oklab::blend`], which this is the inlined form of: the accumulation is
    /// spelled out here because it also has to track the nearest keyframe, and
    /// this runs once per strip position per frame.
    pub fn sample(&self, x: f32, y: f32) -> Oklab {
        let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));

        // `weight` normalises opacity; `cover` normalises colour.
        let (mut weight, mut cover) = (0.0f32, 0.0f32);
        let (mut l, mut a, mut b) = (0.0f32, 0.0f32, 0.0f32);
        let mut nearest = (f32::MAX, 0usize);

        for (i, p) in self.points.iter().enumerate() {
            let (dx, dy) = (x - p.x, y - p.y);
            let d2 = dx * dx + dy * dy;
            if d2 < nearest.0 {
                nearest = (d2, i);
            }

            let w = (-d2 * self.falloff).exp();
            let wa = w * p.color.alpha;
            weight += w;
            cover += wa;
            l += wa * p.color.l;
            a += wa * p.color.a;
            b += wa * p.color.b;
        }

        // With a small sigma and a point far from every keyframe, all weights
        // can underflow to zero. Falling back to the nearest keyframe keeps the
        // surface defined everywhere instead of returning black.
        if weight <= f32::MIN_POSITIVE {
            return self.points[nearest.1].color;
        }

        let alpha = cover / weight;

        // Every keyframe within reach is fully transparent, so there is no
        // colour to average — only the absence of one.
        if cover <= f32::MIN_POSITIVE {
            return Oklab::with_alpha(0.0, 0.0, 0.0, alpha);
        }

        Oklab::with_alpha(l / cover, a / cover, b / cover, alpha)
    }
}

/// Parse `#rrggbb` or `#rrggbbaa`, with or without the hash.
///
/// Six digits is fully opaque. That is what lets a preset saved before opacity
/// existed load with exactly the appearance it had.
fn parse_color(s: &str) -> Result<LinearRgb, String> {
    let t = s.trim().trim_start_matches('#');
    let value =
        |t: &str| u32::from_str_radix(t, 16).map_err(|_| format!("colour '{s}' is not valid hex"));
    match t.len() {
        6 => Ok(srgb_hex_to_linear(value(t)?)),
        8 => Ok(srgba_hex_to_linear(value(t)?)),
        _ => Err(format!(
            "colour '{s}' must be 6 hex digits, or 8 with an opacity byte, optionally prefixed with '#'"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::oklab::srgb_hex_to_linear;

    fn default_surface() -> ColorSurface {
        ColorSurface::new(&SurfaceConfig::default()).unwrap()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn parses_hex_with_and_without_hash() {
        let expected = srgb_hex_to_linear(0xFF2000);
        for s in ["#ff2000", "ff2000"] {
            let got = parse_color(s).unwrap();
            assert!(close(got.r, expected.r) && close(got.alpha, 1.0), "{s} gave {got:?}");
        }
        assert!(parse_color("#ff20").is_err());
        assert!(parse_color("#gggggg").is_err());
    }

    /// Eight digits carry opacity, six mean opaque. The second half is the
    /// compatibility guarantee: presets predate the channel and must not shift.
    #[test]
    fn parses_the_opacity_byte() {
        let opaque = parse_color("#ff2000").unwrap();
        let full = parse_color("#ff2000ff").unwrap();
        assert!(close(full.r, opaque.r) && close(full.alpha, 1.0));

        assert!(close(parse_color("#ff200000").unwrap().alpha, 0.0));
        assert!(close(parse_color("ff200080").unwrap().alpha, 128.0 / 255.0));

        // The colour half must not depend on the opacity half.
        let faded = parse_color("#ff200040").unwrap();
        assert!(close(faded.r, opaque.r) && close(faded.g, opaque.g) && close(faded.b, opaque.b));

        assert!(parse_color("#ff2000f").is_err());
        assert!(parse_color("#ff2000gg").is_err());
    }

    /// Sampling at a keyframe should land close to that keyframe's colour. Not
    /// exactly — this interpolator approximates — but close enough that the
    /// authored palette is clearly what shows up.
    #[test]
    fn sampling_at_a_keyframe_resembles_it() {
        let cfg = SurfaceConfig::default();
        let surface = default_surface();

        for k in &cfg.keyframes {
            let expected = parse_color(&k.color).unwrap().to_oklab();
            let got = surface.sample(k.x, k.y);
            let dist = ((got.l - expected.l).powi(2)
                + (got.a - expected.a).powi(2)
                + (got.b - expected.b).powi(2))
            .sqrt();
            assert!(dist < 0.12, "at ({}, {}) got {got:?}, wanted {expected:?}", k.x, k.y);
        }
    }

    /// Quiet bands must be dark, or the whole strip glows at idle.
    #[test]
    fn zero_intensity_is_dark_everywhere() {
        let surface = default_surface();
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            let l = surface.sample(x, 0.0).l;
            assert!(l < 0.25, "x={x} at zero intensity had lightness {l}");
        }
    }

    /// Brightness must rise with intensity at every position on the strip.
    #[test]
    fn lightness_increases_with_intensity() {
        let surface = default_surface();
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            let mut previous = f32::NEG_INFINITY;
            for j in 0..=10 {
                let l = surface.sample(x, j as f32 / 10.0).l;
                assert!(l >= previous - 1e-4, "x={x} lightness dipped at y={}", j as f32 / 10.0);
                previous = l;
            }
        }
    }

    /// Every sample must be displayable to within far less than one 8-bit step,
    /// which is what lets the renderer clamp instead of gamut-mapping.
    ///
    /// Note the invariant is "negligibly outside", not "exactly inside". The
    /// blend is convex in Oklab, but the cubic conversion to RGB does not
    /// preserve convexity, so tiny excursions are expected. This pins how tiny.
    #[test]
    fn every_sample_is_within_a_rounding_error_of_gamut() {
        let surface = default_surface();
        let mut worst = 0.0f32;
        for i in 0..=64 {
            for j in 0..=64 {
                let (x, y) = (i as f32 / 64.0, j as f32 / 64.0);
                let excursion = surface.sample(x, y).to_linear_rgb().gamut_excursion();
                worst = worst.max(excursion);
            }
        }
        assert!(
            worst <= 0.5 / 255.0,
            "worst excursion {worst} exceeds half an 8-bit step ({})",
            0.5 / 255.0
        );
    }

    /// The field must be continuous — a visible seam on the wall would come from
    /// a discontinuity here.
    #[test]
    fn surface_is_continuous() {
        let surface = default_surface();
        let step = 1.0 / 256.0;
        for i in 0..256 {
            let x = i as f32 / 256.0;
            for j in 0..256 {
                let y = j as f32 / 256.0;
                let a = surface.sample(x, y);
                let b = surface.sample(x + step, y);
                assert!(
                    (a.l - b.l).abs() < 0.05 && (a.a - b.a).abs() < 0.05 && (a.b - b.b).abs() < 0.05,
                    "jump between ({x}, {y}) and ({}, {y})",
                    x + step
                );
            }
        }
    }

    #[test]
    fn rejects_empty_and_malformed_configs() {
        assert!(ColorSurface::new(&SurfaceConfig { keyframes: vec![], sigma: 0.25 }).is_err());
        assert!(ColorSurface::new(&SurfaceConfig {
            keyframes: vec![Keyframe::new(0.0, 0.0, "nope")],
            sigma: 0.25,
        })
        .is_err());
    }

    /// Coincident keyframes are something a user dragging points will produce.
    /// This interpolator must simply average them, not blow up.
    #[test]
    fn coincident_keyframes_are_harmless() {
        let cfg = SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.5, 0.5, "#ff0000"),
                Keyframe::new(0.5, 0.5, "#0000ff"),
            ],
            sigma: 0.25,
        };
        let surface = ColorSurface::new(&cfg).unwrap();
        let c = surface.sample(0.5, 0.5);
        assert!(c.l.is_finite() && c.a.is_finite() && c.b.is_finite(), "produced {c:?}");
    }

    /// The shipped palette predates opacity and must still be fully opaque, or
    /// adding the channel would have quietly changed what everyone's strip does.
    #[test]
    fn the_default_palette_is_opaque_everywhere() {
        let surface = default_surface();
        for i in 0..=16 {
            for j in 0..=16 {
                let alpha = surface.sample(i as f32 / 16.0, j as f32 / 16.0).alpha;
                assert!(close(alpha, 1.0), "({i}, {j}) sampled at opacity {alpha}");
            }
        }
    }

    /// Opacity has to be a field like colour is: smooth between keyframes, and
    /// landing near what was authored at each of them.
    #[test]
    fn opacity_interpolates_across_the_field() {
        let cfg = SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.0, 0.5, "#ff2000ff"),
                Keyframe::new(1.0, 0.5, "#ff200000"),
            ],
            sigma: 0.25,
        };
        let surface = ColorSurface::new(&cfg).unwrap();

        assert!(surface.sample(0.0, 0.5).alpha > 0.9, "the opaque end faded");
        assert!(surface.sample(1.0, 0.5).alpha < 0.1, "the transparent end did not");

        let mut previous = f32::INFINITY;
        for i in 0..=20 {
            let alpha = surface.sample(i as f32 / 20.0, 0.5).alpha;
            assert!((0.0..=1.0).contains(&alpha), "opacity left 0..1 at x={i}: {alpha}");
            assert!(alpha <= previous + 1e-4, "opacity rose again at x={i}");
            previous = alpha;
        }
    }

    /// The bug alpha-weighted blending exists to prevent: an invisible keyframe
    /// must not tint the colours around it. A user fading a green keyframe out
    /// expects the green to leave, not to linger as a wash over its neighbours.
    #[test]
    fn a_transparent_keyframe_lends_no_colour() {
        let with_ghost = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.0, 0.5, "#ff2000"),
                Keyframe::new(0.4, 0.5, "#00ff0000"),
            ],
            sigma: 0.25,
        })
        .unwrap();
        let without = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![Keyframe::new(0.0, 0.5, "#ff2000")],
            sigma: 0.25,
        })
        .unwrap();

        for i in 0..=10 {
            let x = i as f32 / 10.0;
            let (ghost, plain) = (with_ghost.sample(x, 0.5), without.sample(x, 0.5));
            assert!(
                close(ghost.l, plain.l) && close(ghost.a, plain.a) && close(ghost.b, plain.b),
                "the invisible keyframe changed the colour at x={x}: {ghost:?} vs {plain:?}"
            );
        }

        // It must still make the field *fade*, though — it is transparent, not
        // absent, and pulling opacity down is exactly what it is there to do.
        assert!(with_ghost.sample(0.4, 0.5).alpha < 0.6, "the field did not fade toward it");
    }

    /// A field with nothing visible in it must stay finite rather than dividing
    /// by an opacity of zero.
    #[test]
    fn a_fully_transparent_field_is_harmless() {
        let surface = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.0, 0.0, "#ff200000"),
                Keyframe::new(1.0, 1.0, "#40c0ff00"),
            ],
            sigma: 0.25,
        })
        .unwrap();

        for i in 0..=8 {
            let c = surface.sample(i as f32 / 8.0, 0.5);
            assert!(c.l.is_finite() && c.a.is_finite() && c.b.is_finite(), "produced {c:?}");
            assert!(close(c.alpha, 0.0), "opacity should be zero, got {}", c.alpha);
        }
    }
}
