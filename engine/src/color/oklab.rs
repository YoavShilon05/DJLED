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
//!
//! # Opacity
//!
//! Every colour here carries an `alpha` alongside its three colour channels.
//! Today there is exactly one layer and it is composited onto an unlit strip, so
//! alpha reads as a straight dimming — `#ff0000` at half opacity and `#800000`
//! reach the LEDs as the same bytes. That equivalence is a coincidence of the
//! backdrop being black, and it ends the moment layers stack: over a blue layer,
//! the first shows purple and the second still shows dark red.
//!
//! Two consequences of that, both load-bearing:
//!
//! - Colour is stored **un-premultiplied**. `l`/`a`/`b` mean the same thing at
//!   any opacity, so fading a keyframe out never drags its hue toward black and
//!   fading it back in returns the colour that was authored.
//! - Alpha is **linear coverage**, not a light level, so it gets no sRGB
//!   transfer function on the way in or out. `#ff000080` is half-covered red,
//!   not "red at 50% encoded brightness".
//!
//! Blending is the one place the two representations meet: a weighted average of
//! un-premultiplied colours has to weight by `w · α`, or a transparent keyframe
//! would tint its neighbours with a colour nobody can see. See [`Oklab::blend`].

/// Perceptually uniform. `l` is lightness in 0..1; `a`/`b` are the green-red and
/// blue-yellow axes, typically within about ±0.4.
///
/// `alpha` is opacity in 0..1, un-premultiplied — see the module docs. Note that
/// `a` and `alpha` are unrelated: `a` is a chroma axis and gets interpolated,
/// `alpha` is coverage and gets composited.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Oklab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
    /// Opacity, 0 = invisible, 1 = fully covering.
    pub alpha: f32,
}

/// Light-linear RGB, 0..1 per channel. Values outside that range are physically
/// unrepresentable and must be gamut-mapped before display.
///
/// `alpha` is opacity in 0..1, un-premultiplied.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinearRgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// Opacity, 0 = invisible, 1 = fully covering.
    pub alpha: f32,
}

impl Oklab {
    /// A fully opaque colour.
    pub const fn new(l: f32, a: f32, b: f32) -> Self {
        Self { l, a, b, alpha: 1.0 }
    }

    pub const fn with_alpha(l: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self { l, a, b, alpha }
    }

    /// Weighted average of several colours. Used by the keyframe surface; kept
    /// here so the mixing stays in the perceptual space.
    ///
    /// Colour is averaged with weights `w · α` while opacity is averaged with
    /// plain `w`. That asymmetry is the whole point: a fully transparent
    /// keyframe must pull the result toward *transparent*, not toward its own
    /// invisible colour. Weighting colour by `w` alone would let a
    /// `#00ff0000` keyframe tint everything near it green.
    ///
    /// When every contributor is fully transparent there is no colour to
    /// average and the result is transparent black. That is a discontinuity in
    /// hue, but only at opacity zero, where nothing is visible to be
    /// discontinuous about.
    pub fn blend(items: impl IntoIterator<Item = (f32, Oklab)>) -> Self {
        let (mut l, mut a, mut b) = (0.0, 0.0, 0.0);
        // `weight` normalises opacity; `cover` normalises colour.
        let (mut weight, mut cover) = (0.0f32, 0.0f32);
        for (w, c) in items {
            let wa = w * c.alpha;
            l += wa * c.l;
            a += wa * c.a;
            b += wa * c.b;
            weight += w;
            cover += wa;
        }
        if weight <= 0.0 {
            return Self::default();
        }
        if cover <= 0.0 {
            return Self::with_alpha(0.0, 0.0, 0.0, 0.0);
        }
        Self::with_alpha(l / cover, a / cover, b / cover, cover / weight)
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
            // Un-premultiplied, so opacity survives the colour conversion
            // untouched. Both directions are pure colour maths.
            alpha: self.alpha,
        }
    }
}

impl LinearRgb {
    /// The unlit strip: what the whole stack is finally composited onto.
    pub const BLACK: Self = Self::with_alpha(0.0, 0.0, 0.0, 1.0);

    /// Nothing at all — the identity the layer stack accumulates from.
    ///
    /// Not the same as [`Self::BLACK`], and the difference is the whole point of
    /// the stack: black *covers* what is under it, transparent does not.
    pub const CLEAR: Self = Self::with_alpha(0.0, 0.0, 0.0, 0.0);

    /// A fully opaque colour.
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, alpha: 1.0 }
    }

    pub const fn with_alpha(r: f32, g: f32, b: f32, alpha: f32) -> Self {
        Self { r, g, b, alpha }
    }

    /// Source-over composite: `self` painted on top of `backdrop`.
    ///
    /// In linear light, which is the only place compositing is physically
    /// meaningful — two lights add, and their gamma-encoded bytes do not.
    ///
    /// This is the operator that gives opacity its meaning, and the fold the
    /// layer stack is built from — see [`super::render::Renderer::render_stack`].
    /// With [`LinearRgb::BLACK`] as the backdrop it reduces to `colour × alpha`,
    /// which is why a *single* translucent layer looks like nothing more than a
    /// dimmer one, and why that stops being true the moment there is something
    /// underneath it.
    pub fn over(self, backdrop: Self) -> Self {
        let src = self.alpha.clamp(0.0, 1.0);
        let under = backdrop.alpha.clamp(0.0, 1.0) * (1.0 - src);
        let alpha = src + under;
        if alpha <= 0.0 {
            return Self::with_alpha(0.0, 0.0, 0.0, 0.0);
        }
        // Divide back out, so the result stays un-premultiplied like its inputs.
        Self::with_alpha(
            (self.r * src + backdrop.r * under) / alpha,
            (self.g * src + backdrop.g * under) / alpha,
            (self.b * src + backdrop.b * under) / alpha,
            alpha,
        )
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
            alpha: self.alpha,
        }
    }

    /// Clamp into the representable cube, and opacity into 0..1.
    ///
    /// Per-channel clamping shifts hue for badly out-of-gamut colours, so it is
    /// only an acceptable substitute for real gamut mapping when excursions are
    /// tiny. They are here: see [`Self::gamut_excursion`].
    pub fn clamped(self) -> Self {
        Self {
            r: self.r.clamp(0.0, 1.0),
            g: self.g.clamp(0.0, 1.0),
            b: self.b.clamp(0.0, 1.0),
            alpha: self.alpha.clamp(0.0, 1.0),
        }
    }

    /// How far outside the representable cube the worst colour channel falls, in
    /// linear units. Zero when fully displayable. Opacity is not a colour
    /// channel and has no gamut, so it is not considered here.
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

/// Decode an sRGB hex triplet (`0xRRGGBB`) to fully opaque linear light.
pub fn srgb_hex_to_linear(hex: u32) -> LinearRgb {
    let ch = |shift: u32| srgb_to_linear(((hex >> shift) & 0xFF) as f32 / 255.0);
    LinearRgb::new(ch(16), ch(8), ch(0))
}

/// Decode an sRGB hex quadruplet (`0xRRGGBBAA`) to linear light with opacity.
///
/// Only the colour channels get the sRGB transfer function. Alpha is coverage
/// rather than a light level, so it is a plain `/ 255` — the same convention CSS
/// `#rrggbbaa` uses, which is what makes an authored value mean the same thing
/// in the editor and on the strip.
pub fn srgba_hex_to_linear(hex: u32) -> LinearRgb {
    let ch = |shift: u32| srgb_to_linear(((hex >> shift) & 0xFF) as f32 / 255.0);
    LinearRgb::with_alpha(ch(24), ch(16), ch(8), (hex & 0xFF) as f32 / 255.0)
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

    #[test]
    fn hex_decodes_opacity_without_a_transfer_function() {
        let opaque = srgb_hex_to_linear(0xFF2000);
        let same = srgba_hex_to_linear(0xFF2000FF);
        assert!(close(opaque.r, same.r, 1e-6) && close(opaque.alpha, 1.0, 1e-6));

        // Half coverage is 128/255, *not* the sRGB decode of it (~0.216). Get
        // this wrong and every authored opacity reads far too low.
        assert!(close(srgba_hex_to_linear(0x00000080).alpha, 128.0 / 255.0, 1e-6));
        assert!(close(srgba_hex_to_linear(0x00000000).alpha, 0.0, 1e-6));
    }

    /// Opacity is not colour, so it must survive the round trip untouched rather
    /// than being smeared into lightness.
    #[test]
    fn opacity_passes_through_the_colour_conversions() {
        for alpha in [0.0f32, 0.25, 0.5, 1.0] {
            let rgb = LinearRgb::with_alpha(0.3, 0.6, 0.1, alpha);
            assert!(close(rgb.to_oklab().alpha, alpha, 1e-9));
            assert!(close(rgb.to_oklab().to_linear_rgb().alpha, alpha, 1e-9));
        }
    }

    /// The colour a translucent value carries must not depend on its opacity —
    /// that is what un-premultiplied storage buys, and it is why fading a
    /// keyframe out and back in returns the colour that was authored.
    #[test]
    fn colour_is_independent_of_opacity() {
        let base = srgb_hex_to_linear(0xFF2000).to_oklab();
        for alpha in [0.0f32, 0.1, 0.5, 1.0] {
            let faded = Oklab::with_alpha(base.l, base.a, base.b, alpha);
            let rgb = faded.to_linear_rgb();
            let opaque = base.to_linear_rgb();
            assert!(
                close(rgb.r, opaque.r, 1e-6)
                    && close(rgb.g, opaque.g, 1e-6)
                    && close(rgb.b, opaque.b, 1e-6),
                "alpha {alpha} changed the colour: {rgb:?} vs {opaque:?}"
            );
        }
    }

    /// Over black — the only backdrop there is until layers land — opacity has
    /// to behave exactly like scaling the light.
    #[test]
    fn compositing_over_black_scales_the_light() {
        let red = srgb_hex_to_linear(0xFF0000);
        for i in 0..=10 {
            let alpha = i as f32 / 10.0;
            let out = LinearRgb::with_alpha(red.r, red.g, red.b, alpha).over(LinearRgb::BLACK);
            assert!(close(out.r, red.r * alpha, 1e-6), "alpha {alpha} gave {out:?}");
            assert!(close(out.alpha, 1.0, 1e-6), "black backdrop must come out opaque");
        }
    }

    /// ...and over anything else it must not, or the channel would be a
    /// pointless second brightness control. This is the case layer stacking
    /// exists for.
    #[test]
    fn compositing_over_a_lit_backdrop_differs_from_dimming() {
        let red = srgb_hex_to_linear(0xFF0000);
        let blue = srgb_hex_to_linear(0x0000FF);
        let half = LinearRgb::with_alpha(red.r, red.g, red.b, 0.5);

        let over_blue = half.over(blue);
        let over_black = half.over(LinearRgb::BLACK);
        assert!(over_blue.b > 0.4 * blue.b, "the backdrop did not show through: {over_blue:?}");
        assert!(
            (over_blue.b - over_black.b).abs() > 0.05,
            "compositing over blue matched compositing over black"
        );
    }

    #[test]
    fn compositing_edge_cases_hold() {
        let red = srgb_hex_to_linear(0xFF0000);
        let blue = srgb_hex_to_linear(0x0000FF);

        // Opaque source hides the backdrop entirely.
        let opaque = red.over(blue);
        assert!(close(opaque.r, red.r, 1e-6) && close(opaque.b, red.b, 1e-6));

        // Transparent source leaves the backdrop exactly as it was.
        let invisible = LinearRgb::with_alpha(red.r, red.g, red.b, 0.0).over(blue);
        assert!(close(invisible.r, blue.r, 1e-6) && close(invisible.b, blue.b, 1e-6));

        // Nothing over nothing is still nothing, and must not divide by zero.
        let empty = LinearRgb::with_alpha(1.0, 1.0, 1.0, 0.0)
            .over(LinearRgb::with_alpha(1.0, 1.0, 1.0, 0.0));
        assert!(close(empty.alpha, 0.0, 1e-9) && empty.r.is_finite());
    }

    /// A transparent contributor lowers opacity without lending its hue. The
    /// failure this guards against is subtle and ugly: an invisible green
    /// keyframe quietly tinting everything around it.
    #[test]
    fn blending_weights_colour_by_opacity() {
        let red = Oklab::with_alpha(0.6, 0.2, 0.1, 1.0);
        let green = Oklab::with_alpha(0.8, -0.2, 0.1, 0.0);

        let mixed = Oklab::blend([(1.0, red), (1.0, green)]);
        assert!(
            close(mixed.l, red.l, 1e-6) && close(mixed.a, red.a, 1e-6),
            "invisible green leaked into {mixed:?}"
        );
        assert!(close(mixed.alpha, 0.5, 1e-6), "opacity should average to 0.5, got {}", mixed.alpha);

        // Equal opacities reduce to the plain weighted average it always was.
        let a = Oklab::with_alpha(0.4, 0.0, 0.0, 0.5);
        let b = Oklab::with_alpha(0.8, 0.0, 0.0, 0.5);
        let even = Oklab::blend([(1.0, a), (1.0, b)]);
        assert!(close(even.l, 0.6, 1e-6) && close(even.alpha, 0.5, 1e-6), "got {even:?}");
    }

    #[test]
    fn blending_nothing_visible_yields_nothing() {
        let ghost = Oklab::with_alpha(0.9, 0.1, 0.1, 0.0);
        let out = Oklab::blend([(1.0, ghost), (2.0, ghost)]);
        assert!(close(out.alpha, 0.0, 1e-9) && out.l.is_finite(), "got {out:?}");
        assert!(Oklab::blend([]).l.is_finite());
    }
}
