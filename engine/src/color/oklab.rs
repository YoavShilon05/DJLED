//! Oklab, and the colour spaces either side of it.
//!
//! Three distinct spaces are in play and conflating any two of them produces
//! visibly wrong output:
//!
//! - **sRGB** — gamma-encoded, what a hex code or colour picker gives you.
//! - **Linear RGB** — proportional to actual light output. This is what a WS2812
//!   PWM byte controls.
//! - **Oklab** — perceptually uniform. Interpolation belongs here.
//!
//! Interpolating in sRGB drags gradients through muddy grey; interpolating hue
//! in HSV bands badly and swings through colours nobody asked for. Oklab moves
//! between two colours along the path the eye reads as direct.
//!
//! # Why there is no gamma step on the way out
//!
//! WS2812 brightness is linear in the byte value, and Oklab's `l` is already
//! perceptual lightness. So `Oklab -> linear RGB -> byte` is complete and
//! correct: perceived brightness ends up proportional to `l`, which is exactly
//! what makes a gradient look even.
//!
//! The usual reason people bolt a gamma curve onto WS2812 output is that they
//! started from an sRGB value and drove the LED with it directly, which
//! double-counts the encoding. Doing the conversion properly removes the need.
//! [`super::render::RenderConfig::gamma`] exists only as a taste trim for
//! strips that are not quite linear, and defaults to 1.0.

/// Perceptually uniform. `l` is lightness in 0..1; `a`/`b` are the green-red and
/// blue-yellow axes, typically within about ±0.4.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Oklab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
}

/// Light-linear RGB, 0..1 per channel. Values outside that range are physically
/// unrepresentable and must be gamut-mapped before display.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinearRgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Oklab {
    pub const fn new(l: f32, a: f32, b: f32) -> Self {
        Self { l, a, b }
    }

    /// Weighted average of several colours. Used by the keyframe surface; kept
    /// here so the mixing stays in the perceptual space.
    pub fn blend(items: impl IntoIterator<Item = (f32, Oklab)>) -> Self {
        let (mut l, mut a, mut b, mut total) = (0.0, 0.0, 0.0, 0.0);
        for (w, c) in items {
            l += w * c.l;
            a += w * c.a;
            b += w * c.b;
            total += w;
        }
        if total <= 0.0 {
            return Self::default();
        }
        Self::new(l / total, a / total, b / total)
    }

    pub fn to_linear_rgb(self) -> LinearRgb {
        let l_ = self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b;
        let m_ = self.l - 0.105_561_35 * self.a - 0.063_854_17 * self.b;
        let s_ = self.l - 0.089_484_18 * self.a - 1.291_485_5 * self.b;

        let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);

        LinearRgb {
            r: 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            g: -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            b: -0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s,
        }
    }
}

impl LinearRgb {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    pub fn to_oklab(self) -> Oklab {
        let l = 0.412_221_47 * self.r + 0.536_332_54 * self.g + 0.051_445_995 * self.b;
        let m = 0.211_903_5 * self.r + 0.680_699_5 * self.g + 0.107_396_96 * self.b;
        let s = 0.088_302_46 * self.r + 0.281_718_85 * self.g + 0.629_978_7 * self.b;

        // cbrt rather than powf(1/3): it is defined for negatives, which occur
        // for out-of-gamut inputs and would otherwise produce NaN.
        let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());

        Oklab {
            l: 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
            a: 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
            b: 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
        }
    }

    /// Clamp into the representable cube.
    ///
    /// Per-channel clamping shifts hue for badly out-of-gamut colours, so it is
    /// only an acceptable substitute for real gamut mapping when excursions are
    /// tiny. They are here: see [`Self::gamut_excursion`].
    pub fn clamped(self) -> Self {
        Self {
            r: self.r.clamp(0.0, 1.0),
            g: self.g.clamp(0.0, 1.0),
            b: self.b.clamp(0.0, 1.0),
        }
    }

    /// How far outside the representable cube the worst channel falls, in linear
    /// units. Zero when fully displayable.
    ///
    /// Worth understanding why this is ever non-zero. The colour surface blends
    /// by normalised weighted average, which is a convex combination *in Oklab*
    /// — but `Oklab -> linear RGB` is cubic, and the image of a convex set under
    /// a nonlinear map need not be convex. So a blend of two perfectly
    /// displayable colours can land marginally outside the cube.
    ///
    /// Marginally is the operative word: with keyframes authored inside gamut,
    /// measured excursions are on the order of 1e-4, roughly a thirtieth of one
    /// 8-bit step. That is what justifies plain clamping instead of a real
    /// gamut-mapping pass.
    pub fn gamut_excursion(self) -> f32 {
        [self.r, self.g, self.b]
            .into_iter()
            .map(|c| if c < 0.0 { -c } else { (c - 1.0).max(0.0) })
            .fold(0.0, f32::max)
    }

    /// Within half an 8-bit step of displayable — that is, close enough that
    /// clamping cannot change the byte that reaches the LED.
    pub fn is_in_gamut(self) -> bool {
        self.gamut_excursion() <= 0.5 / 255.0
    }
}

/// Decode an sRGB hex triplet (`0xRRGGBB`) to linear light.
pub fn srgb_hex_to_linear(hex: u32) -> LinearRgb {
    let ch = |shift: u32| srgb_to_linear(((hex >> shift) & 0xFF) as f32 / 255.0);
    LinearRgb::new(ch(16), ch(8), ch(0))
}

/// sRGB electro-optical transfer function: encoded value to linear light.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Inverse of [`srgb_to_linear`]. Needed only for previewing on a monitor, never
/// on the path to the LEDs.
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn oklab_round_trips_through_linear_rgb() {
        for hex in [0xFF0000, 0x00FF00, 0x0000FF, 0xFFFFFF, 0x808080, 0x1A2B3C, 0x000000] {
            let original = srgb_hex_to_linear(hex);
            let back = original.to_oklab().to_linear_rgb();
            assert!(
                close(original.r, back.r, 1e-4)
                    && close(original.g, back.g, 1e-4)
                    && close(original.b, back.b, 1e-4),
                "{hex:06X} round-tripped {original:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn srgb_transfer_function_round_trips() {
        for i in 0..=255 {
            let c = i as f32 / 255.0;
            assert!(close(linear_to_srgb(srgb_to_linear(c)), c, 1e-5), "failed at {c}");
        }
    }

    /// White must be neutral: equal channels in, zero chroma out.
    #[test]
    fn greys_have_no_chroma() {
        for level in [0.0f32, 0.25, 0.5, 1.0] {
            let lab = LinearRgb::new(level, level, level).to_oklab();
            assert!(
                close(lab.a, 0.0, 1e-4) && close(lab.b, 0.0, 1e-4),
                "grey {level} produced chroma a={} b={}",
                lab.a, lab.b
            );
        }
    }

    /// Lightness must be monotonic in luminance, or brightness ramps invert.
    #[test]
    fn lightness_increases_with_luminance() {
        let mut previous = f32::NEG_INFINITY;
        for i in 0..=20 {
            let v = i as f32 / 20.0;
            let l = LinearRgb::new(v, v, v).to_oklab().l;
            assert!(l > previous, "lightness not monotonic at {v}");
            previous = l;
        }
    }

    /// Blending two in-gamut colours must stay in gamut — the property the
    /// surface relies on so it never needs real gamut mapping.
    #[test]
    fn blends_of_in_gamut_colours_stay_in_gamut() {
        let a = srgb_hex_to_linear(0xFF0000).to_oklab();
        let b = srgb_hex_to_linear(0x0000FF).to_oklab();
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let mixed = Oklab::blend([(1.0 - t, a), (t, b)]).to_linear_rgb();
            assert!(mixed.is_in_gamut(), "t={t} left gamut: {mixed:?}");
        }
    }
}
