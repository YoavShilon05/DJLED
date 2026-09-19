//! Band levels to LED bytes, folded over the layer stack.
//!
//! Output is RGB order. WS2812B is physically GRB, but FastLED handles that
//! reordering in the firmware via its template parameter, so the wire protocol
//! stays in the order humans expect.
//!
//! # What a layer contributes
//!
//! Every layer produces, for each control point, a colour and an *opacity*, and
//! the stack is folded bottom to top with `source-over` in linear light. Two
//! decisions make that fold mean what the editor says it means:
//!
//! - **The intensity stage scales light, not coverage.** A band the threshold
//!   has closed down is painted with its field's colour at zero intensity —
//!   black in the stock palette — at whatever opacity that keyframe carries. So
//!   an opaque quiet band *blocks*, exactly like an opaque bright one. Black is
//!   a colour, not an absence, and a layer wanting to let the one below show
//!   through in its quiet regions says so by authoring opacity there.
//! - **A position no sector reaches contributes nothing at all**, rather than
//!   contributing black. That is not the same statement, and the difference is
//!   invisible until there is a layer underneath: sectors are how a layer is
//!   confined to part of the wall, and a layer confined to the first fifty LEDs
//!   must not blank the other hundred.
//!
//! Over the unlit strip — one layer, or the bottom of any stack — both reduce to
//! what the renderer did before layers existed, byte for byte. That is deliberate
//! and is pinned by `tests/pipeline.rs`.

use super::intensity::IntensityConfig;
use super::oklab::LinearRgb;
use super::strip::{LayoutConfig, StripMap};
use super::surface::{ColorSurface, SurfaceConfig};
use super::timeline::Timeline;

#[derive(Clone, Debug)]
pub struct RenderConfig {
    /// Master brightness, applied in linear light so it scales actual output
    /// rather than perceived lightness. Also the software half of the power cap:
    /// FastLED enforces the hardware limit, this keeps the whole strip below it
    /// without the driver having to claw brightness back mid-frame.
    pub brightness: f32,
    /// Taste trim only. The Oklab -> linear RGB conversion already lands
    /// perceived brightness proportional to lightness, so 1.0 is correct for a
    /// linear strip; see [`super::oklab`]. Raise slightly if your strip reads
    /// top-heavy.
    pub gamma: f32,
    /// Temporal dithering. WS2812B has 8 bits per channel and visibly steps at
    /// the bottom of a fade; feeding the quantisation error into the next frame
    /// recovers roughly two more bits, invisibly at 70 fps.
    pub dither: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self { brightness: 1.0, gamma: 1.0, dither: true }
    }
}

/// How many colours go on the wire, and what they are spread across.
#[derive(Clone, Debug)]
pub struct Geometry {
    /// Band centre frequencies, for looking a level up by frequency.
    pub centers: Vec<f32>,
    /// Colours per frame. Kept at the band count so the wire format is unchanged.
    pub points: usize,
    pub leds: usize,
}

/// One layer as the renderer needs it: a colour field, where it lands on the
/// wall, how a level becomes brightness, and how strongly it covers what is
/// beneath.
///
/// Centres are per layer rather than per renderer because two layers can be
/// reading two different devices — 48 audio bands under 88 semitones of MIDI —
/// and each has to look its own levels up on its own axis before the two meet
/// in strip space.
#[derive(Clone, Debug)]
pub struct LayerVisual {
    /// The still field, read only when the timeline has no keys of its own.
    pub surface: SurfaceConfig,
    /// The same field over time. Its keys win wherever there are any — see
    /// [`ColorSurface::for_layer`].
    pub timeline: Timeline,
    pub layout: LayoutConfig,
    pub intensity: IntensityConfig,
    /// Master opacity for the layer, multiplied into every sample's own.
    pub opacity: f32,
    /// Band centre frequencies for *this layer's* source.
    pub centers: Vec<f32>,
}

impl LayerVisual {
    /// A layer covering the whole strip with the default field — what a
    /// one-layer show is.
    pub fn new(
        surface: SurfaceConfig,
        layout: LayoutConfig,
        intensity: IntensityConfig,
        centers: Vec<f32>,
    ) -> Self {
        Self {
            surface,
            timeline: Timeline::default(),
            layout,
            intensity,
            opacity: 1.0,
            centers,
        }
    }
}

struct CompiledLayer {
    surface: ColorSurface,
    map: StripMap,
    opacity: f32,
}

impl CompiledLayer {
    fn new(visual: &LayerVisual, points: usize, leds: usize) -> Result<Self, String> {
        Ok(Self {
            surface: ColorSurface::for_layer(&visual.surface, &visual.timeline)?,
            map: StripMap::new(
                visual.layout.clone(),
                visual.intensity,
                &visual.centers,
                points,
                leds,
            ),
            opacity: visual.opacity.clamp(0.0, 1.0),
        })
    }
}

pub struct Renderer {
    layers: Vec<CompiledLayer>,
    cfg: RenderConfig,
    points: usize,
    leds: usize,
    /// The stack, accumulated bottom to top, un-premultiplied. One per control
    /// point, kept between frames only to avoid reallocating it.
    stack: Vec<LinearRgb>,
    /// Carried quantisation error per control point per channel.
    error: Vec<[f32; 3]>,
    pixels: Vec<[u8; 3]>,
}

impl Renderer {
    /// A single-layer renderer — the whole show before layers existed, and
    /// still what every unit test and the colour reference are written against.
    pub fn new(
        cfg: RenderConfig,
        surface: &SurfaceConfig,
        layout: LayoutConfig,
        intensity: IntensityConfig,
        geometry: Geometry,
    ) -> Result<Self, String> {
        let visual = LayerVisual::new(surface.clone(), layout, intensity, geometry.centers);
        Self::stacked(cfg, std::slice::from_ref(&visual), geometry.points, geometry.leds)
    }

    /// A renderer for a whole stack, bottom layer first.
    ///
    /// Every layer is compiled against the *same* number of control points, so
    /// they meet in strip space rather than in anyone's band index — which is
    /// what lets a MIDI layer sit over an audio one without either knowing.
    pub fn stacked(
        cfg: RenderConfig,
        layers: &[LayerVisual],
        points: usize,
        leds: usize,
    ) -> Result<Self, String> {
        let compiled = layers
            .iter()
            .map(|l| CompiledLayer::new(l, points, leds))
            .collect::<Result<Vec<_>, String>>()?;

        Ok(Self {
            layers: compiled,
            cfg,
            points,
            leds,
            stack: vec![LinearRgb::CLEAR; points],
            error: vec![[0.0; 3]; points],
            pixels: vec![[0; 3]; points],
        })
    }

    /// Replace the whole stack, keeping dither state so an edit does not flash
    /// the strip.
    ///
    /// All or nothing: a layer with an unparseable colour rejects the edit and
    /// leaves the running stack alone, rather than dropping half of it.
    pub fn set_layers(&mut self, layers: &[LayerVisual]) -> Result<(), String> {
        let compiled = layers
            .iter()
            .map(|l| CompiledLayer::new(l, self.points, self.leds))
            .collect::<Result<Vec<_>, String>>()?;
        self.layers = compiled;
        Ok(())
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Whether anything in the stack moves on its own.
    ///
    /// The run loop asks because a timeline is the one thing here that has
    /// something new to draw with no new audio: every other stage is a function
    /// of levels that have not changed. Without this a show of animated layers
    /// would freeze the moment the music stopped — on the wall only, since the
    /// editor draws its preview on its own clock.
    pub fn animates(&self) -> bool {
        self.layers.iter().any(|l| l.surface.animates())
    }

    /// Move every animated layer to where its own loop has reached.
    ///
    /// `seconds` is seconds since the Unix epoch rather than an uptime, so the
    /// editor — reading the same machine's clock — previews the same instant of
    /// the loop without the phase ever crossing the wire. Lengths are per layer,
    /// so each one wraps on its own. See [`super::timeline`].
    pub fn seek(&mut self, seconds: f64) {
        for layer in &mut self.layers {
            layer.surface.seek(seconds);
        }
    }

    pub fn points(&self) -> usize {
        self.points
    }

    pub fn leds(&self) -> usize {
        self.leds
    }

    pub fn config(&self) -> &RenderConfig {
        &self.cfg
    }

    pub fn set_config(&mut self, cfg: RenderConfig) {
        self.cfg = cfg;
    }

    /// One RGB triplet per control point, ready for the wire.
    pub fn pixels(&self) -> &[[u8; 3]] {
        &self.pixels
    }

    /// Render a single-layer show. See [`Self::render_stack`].
    pub fn render(&mut self, levels: &[f32]) -> &[[u8; 3]] {
        self.render_stack(std::slice::from_ref(&levels))
    }

    /// Render the stack. `levels` is one slice per layer, in the same order the
    /// layers were built in — each read on its own frequency axis.
    ///
    /// A layer with no levels to show is skipped rather than treated as silent,
    /// which is the difference between a source that has not delivered a frame
    /// yet and one that is delivering silence.
    pub fn render_stack(&mut self, levels: &[&[f32]]) -> &[[u8; 3]] {
        let trim = (self.cfg.gamma - 1.0).abs() > 1e-6;
        let master = self.cfg.brightness.clamp(0.0, 1.0);
        let gamma = self.cfg.gamma;

        self.stack.fill(LinearRgb::CLEAR);

        for (layer, band_levels) in self.layers.iter_mut().zip(levels) {
            if band_levels.is_empty() {
                continue;
            }
            // Borrowed separately from `layer.surface`, which the loop needs too.
            let points = layer.map.map(band_levels);

            for (i, point) in points.iter().enumerate() {
                // Not addressed by this layer at all: leave the stack alone.
                // Contributing black here would blank every layer beneath, and
                // "outside my sectors" is not a colour.
                if !point.covered {
                    continue;
                }

                let lab = layer.surface.sample(point.x, point.y);
                let mut rgb = lab.to_linear_rgb().clamped();

                if trim {
                    rgb = LinearRgb::with_alpha(
                        rgb.r.powf(gamma),
                        rgb.g.powf(gamma),
                        rgb.b.powf(gamma),
                        // Opacity is coverage, not light, so the gamma trim has
                        // no business touching it.
                        rgb.alpha,
                    );
                }

                // The intensity curve scales *light*, leaving coverage alone.
                // That is what makes a quiet opaque band block rather than fade:
                // it is painted black, and black covers. See the module docs.
                let gain = point.gain.clamp(0.0, 1.0);
                let src = LinearRgb::with_alpha(
                    rgb.r * gain,
                    rgb.g * gain,
                    rgb.b * gain,
                    rgb.alpha * layer.opacity,
                );

                self.stack[i] = src.over(self.stack[i]);
            }
        }

        for i in 0..self.points {
            // Composite the finished stack onto the unlit strip. With one layer
            // this is the whole fold, and reduces to scaling the colour by its
            // opacity — which is why a one-layer show is byte-identical to what
            // the renderer produced before the stack existed.
            let rgb = self.stack[i].over(LinearRgb::BLACK);

            // The master scales linear light, which is where a brightness
            // belongs: the LED is driven linearly, so halving the byte really
            // does halve the light.
            let scale = 255.0 * master;
            let channels = [rgb.r * scale, rgb.g * scale, rgb.b * scale];

            for (c, &target) in channels.iter().enumerate() {
                let carried =
                    if self.cfg.dither { target + self.error[i][c] } else { target };
                let quantised = carried.round().clamp(0.0, 255.0);
                // Bounded by construction: rounding leaves |error| <= 0.5, so
                // this cannot wind up over successive frames.
                self.error[i][c] = if self.cfg.dither { carried - quantised } else { 0.0 };
                self.pixels[i][c] = quantised as u8;
            }
        }

        &self.pixels
    }

    pub fn reset(&mut self) {
        self.error.fill([0.0; 3]);
        self.pixels.fill([0; 3]);
        self.stack.fill(LinearRgb::CLEAR);
    }
}

/// Single-layer helpers, kept because the tests and the colour reference are
/// written against a one-layer renderer and gain nothing from a stack.
impl Renderer {
    /// Replace the bottom layer's colour surface.
    pub fn set_surface(&mut self, surface: &SurfaceConfig) -> Result<(), String> {
        let compiled = ColorSurface::new(surface)?;
        if let Some(layer) = self.layers.first_mut() {
            layer.surface = compiled;
        }
        Ok(())
    }

    pub fn set_layout(&mut self, layout: LayoutConfig) {
        if let Some(layer) = self.layers.first_mut() {
            layer.map.set_layout(layout);
        }
    }

    pub fn set_intensity(&mut self, intensity: IntensityConfig) {
        if let Some(layer) = self.layers.first_mut() {
            layer.map.set_intensity(intensity);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    use super::super::strip::{norm_to_hz, LedKeyframe};
    use super::super::surface::Keyframe;

    const BANDS: usize = 48;

    fn renderer(cfg: RenderConfig) -> Renderer {
        // Intensity stage disabled, so these keep measuring quantisation and
        // master brightness rather than the curve. It gets its own test below.
        renderer_with(cfg, IntensityConfig::pass_through())
    }

    fn renderer_with(cfg: RenderConfig, intensity: IntensityConfig) -> Renderer {
        renderer_for(cfg, intensity, &SurfaceConfig::default())
    }

    fn renderer_for(
        cfg: RenderConfig,
        intensity: IntensityConfig,
        surface: &SurfaceConfig,
    ) -> Renderer {
        let centers: Vec<f32> =
            (0..BANDS).map(|i| norm_to_hz(i as f32 / (BANDS - 1) as f32)).collect();
        Renderer::new(
            cfg,
            surface,
            LayoutConfig::spanning(150),
            intensity,
            Geometry { centers, points: BANDS, leds: 150 },
        )
        .unwrap()
    }

    /// One colour everywhere, so a render measures the colour pipeline and
    /// nothing about the shape of the field.
    fn flat_surface(color: &str) -> SurfaceConfig {
        SurfaceConfig {
            keyframes: vec![Keyframe::new(0.0, 0.0, color), Keyframe::new(1.0, 1.0, color)],
            sigma: 0.5,
        }
    }

    #[test]
    fn silence_renders_essentially_black() {
        let mut r = renderer(RenderConfig::default());
        let out = r.render(&[0.0; BANDS]);
        for px in out {
            assert!(
                px.iter().all(|&c| c <= 8),
                "idle strip would glow at {px:?}"
            );
        }
    }

    #[test]
    fn full_level_renders_bright() {
        let mut r = renderer(RenderConfig::default());
        let out = r.render(&[1.0; BANDS]);
        for px in out {
            assert!(px.iter().any(|&c| c > 128), "full level only reached {px:?}");
        }
    }

    /// Brightness must rise with level for every band, or bars look wrong on the
    /// way up.
    #[test]
    fn brightness_increases_with_level() {
        let mut r = renderer(RenderConfig { dither: false, ..Default::default() });
        let mut previous = [0u32; BANDS];
        for step in 0..=10 {
            let level = step as f32 / 10.0;
            let out = r.render(&[level; BANDS]).to_vec();
            for (i, px) in out.iter().enumerate() {
                let sum: u32 = px.iter().map(|&c| c as u32).sum();
                assert!(
                    sum + 6 >= previous[i],
                    "band {i} dimmed going from level {} to {level}",
                    level - 0.1
                );
                previous[i] = sum;
            }
        }
    }

    /// The point of dithering: a level that falls between two byte values must
    /// alternate over time rather than snapping to one.
    #[test]
    fn dithering_alternates_between_adjacent_values() {
        let mut r = renderer(RenderConfig::default());
        let levels = vec![0.031f32; BANDS];

        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..64 {
            seen.insert(r.render(&levels)[10][0]);
        }
        assert!(seen.len() >= 2, "dither produced only {seen:?}");

        // ...and the alternation must average to the right value, not just
        // wobble around arbitrarily.
        let mut undithered = renderer(RenderConfig { dither: false, ..Default::default() });
        let exact = undithered.render(&levels)[10][0] as f32;
        let mean = seen.iter().map(|&v| v as f32).sum::<f32>() / seen.len() as f32;
        assert!((mean - exact).abs() <= 1.5, "dither mean {mean} strayed from {exact}");
    }

    #[test]
    fn dithering_can_be_disabled() {
        let mut r = renderer(RenderConfig { dither: false, ..Default::default() });
        let levels = vec![0.031f32; BANDS];
        let first = r.render(&levels).to_vec();
        for _ in 0..16 {
            assert_eq!(r.render(&levels), &first[..], "output changed with dither off");
        }
    }

    #[test]
    fn brightness_scales_output() {
        let mut full = renderer(RenderConfig { dither: false, ..Default::default() });
        let bright = full.render(&[1.0; BANDS]).to_vec();

        let mut half = renderer(RenderConfig { brightness: 0.5, dither: false, ..Default::default() });
        let dim = half.render(&[1.0; BANDS]).to_vec();

        for (b, d) in bright.iter().zip(&dim) {
            for c in 0..3 {
                assert!(d[c] <= b[c], "half brightness was not dimmer: {d:?} vs {b:?}");
            }
        }
    }

    /// The intensity stage has to actually reach the wire — the tests above
    /// deliberately bypass it, so this is the one that would catch it being
    /// computed and then dropped.
    #[test]
    fn threshold_reaches_the_output() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let intensity =
            IntensityConfig { threshold: -40.0, clamp: -10.0, curve: Default::default() };

        // -72 dB on the editor's axis: comfortably under the threshold.
        let mut r = renderer_with(cfg.clone(), intensity);
        assert!(
            r.render(&[0.1; BANDS]).iter().all(|px| px == &[0, 0, 0]),
            "signal under the threshold still lit the strip"
        );

        let mut open = renderer_with(cfg, IntensityConfig::pass_through());
        assert!(
            open.render(&[0.1; BANDS]).iter().any(|px| px.iter().any(|&c| c > 0)),
            "the same signal was dark without a threshold, so the test proves nothing"
        );
    }

    /// Reverse must reorder the wire colours, not merely be accepted and lost.
    #[test]
    fn reverse_flips_the_control_points() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let levels: Vec<f32> = (0..BANDS).map(|i| i as f32 / (BANDS - 1) as f32).collect();

        let mut forward = renderer_with(cfg.clone(), IntensityConfig::pass_through());
        let a = forward.render(&levels).to_vec();

        let mut backward = renderer_with(cfg, IntensityConfig::pass_through());
        backward.set_layout(LayoutConfig { reverse: true, ..LayoutConfig::spanning(150) });
        let b = backward.render(&levels).to_vec();

        assert_ne!(a, b, "reverse changed nothing");
        for (i, px) in b.iter().enumerate() {
            assert_eq!(px, &a[BANDS - 1 - i], "point {i} is not the mirror of the forward render");
        }
    }

    /// Opacity has to survive all the way to the wire rather than being computed
    /// and dropped. Over the unlit strip it composites down to a scaling of the
    /// light, which is what makes it measurable end to end today — the case
    /// where it stops being a scaling is the one layer stacking will add.
    #[test]
    fn opacity_reaches_the_output() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let render = |color: &str| {
            renderer_for(cfg.clone(), IntensityConfig::pass_through(), &flat_surface(color))
                .render(&[1.0; BANDS])
                .to_vec()
        };

        let opaque = render("#40ff60");
        let half = render("#40ff6080");
        let clear = render("#40ff6000");

        assert!(opaque.iter().any(|px| px.iter().any(|&c| c > 128)), "the opaque case is not lit");

        for (i, px) in half.iter().enumerate() {
            for c in 0..3 {
                let want = opaque[i][c] as f32 * (128.0 / 255.0);
                assert!(
                    (px[c] as f32 - want).abs() <= 1.5,
                    "point {i} channel {c}: half opacity gave {}, expected about {want}",
                    px[c]
                );
            }
        }

        assert!(
            clear.iter().all(|px| px == &[0, 0, 0]),
            "a fully transparent surface still lit the strip"
        );
    }

    /// The colour a translucent keyframe carries must not depend on its opacity,
    /// so opening one back up returns the colour that was authored rather than
    /// something that has been dragged toward black on the way.
    #[test]
    fn opacity_does_not_change_the_hue_on_the_wire() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let render = |color: &str| {
            renderer_for(cfg.clone(), IntensityConfig::pass_through(), &flat_surface(color))
                .render(&[1.0; BANDS])
                .to_vec()
        };

        let opaque = render("#40ff60");
        let faded = render("#40ff6040");

        // Compare hue as channel ratios, which is what a scaling leaves alone.
        for (i, px) in faded.iter().enumerate() {
            let (a, b) = (px[1] as f32, opaque[i][1] as f32);
            if b < 16.0 {
                continue;
            }
            for c in [0usize, 2] {
                let ratio = px[c] as f32 / a.max(1.0);
                let want = opaque[i][c] as f32 / b;
                assert!(
                    (ratio - want).abs() < 0.05,
                    "point {i} channel {c}: hue shifted from {want} to {ratio}"
                );
            }
        }
    }

    /// Output must stay valid for hostile input — the analyser clamps to 0..1,
    /// but nothing downstream should depend on that.
    #[test]
    fn survives_out_of_range_levels() {
        let mut r = renderer(RenderConfig::default());
        for levels in [vec![-1.0; BANDS], vec![2.0; BANDS], vec![f32::NAN; BANDS]] {
            let out = r.render(&levels);
            assert_eq!(out.len(), BANDS);
        }
    }

    // ---- the stack ----

    fn stacked(cfg: RenderConfig, layers: &[LayerVisual]) -> Renderer {
        Renderer::stacked(cfg, layers, BANDS, 150).unwrap()
    }

    fn visual(surface: &SurfaceConfig, opacity: f32) -> LayerVisual {
        let centers: Vec<f32> =
            (0..BANDS).map(|i| norm_to_hz(i as f32 / (BANDS - 1) as f32)).collect();
        LayerVisual {
            surface: surface.clone(),
            timeline: Timeline::default(),
            layout: LayoutConfig::spanning(150),
            intensity: IntensityConfig::pass_through(),
            opacity,
            centers,
        }
    }

    /// Every layer at full level, which is what measures the fold rather than
    /// the analysis feeding it.
    fn full(r: &mut Renderer, layers: usize) -> Vec<[u8; 3]> {
        let levels = vec![1.0f32; BANDS];
        let refs: Vec<&[f32]> = (0..layers).map(|_| levels.as_slice()).collect();
        r.render_stack(&refs).to_vec()
    }

    /// The headline behaviour, in the words it was asked for: red under green
    /// is green.
    #[test]
    fn an_opaque_layer_overrides_the_one_below() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let red = flat_surface("#ff2000");
        let green = flat_surface("#40ff60");

        let mut stack = stacked(cfg.clone(), &[visual(&red, 1.0), visual(&green, 1.0)]);
        let composited = full(&mut stack, 2);

        let mut alone = stacked(cfg, &[visual(&green, 1.0)]);
        let green_only = full(&mut alone, 1);

        assert_eq!(composited, green_only, "the top layer did not take over");
    }

    /// The other half of the same claim: a one-layer stack must be exactly what
    /// the renderer produced before layers existed, or every show that already
    /// exists changed appearance the day this landed.
    #[test]
    fn one_layer_is_unchanged_by_the_stack() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let levels: Vec<f32> = (0..BANDS).map(|i| i as f32 / (BANDS - 1) as f32).collect();

        let mut single = renderer_with(cfg.clone(), IntensityConfig::pass_through());
        let a = single.render(&levels).to_vec();

        let mut stack = stacked(cfg, &[visual(&SurfaceConfig::default(), 1.0)]);
        let b = stack.render_stack(&[&levels]).to_vec();

        assert_eq!(a, b);
    }

    /// Layer opacity is the Photoshop slider: half of it lands halfway between
    /// the two layers, not halfway to black.
    #[test]
    fn layer_opacity_lerps_toward_what_is_underneath() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let red = flat_surface("#ff2000");
        let green = flat_surface("#40ff60");

        let mut lower = stacked(cfg.clone(), &[visual(&red, 1.0)]);
        let red_only = full(&mut lower, 1);
        let mut upper = stacked(cfg.clone(), &[visual(&green, 1.0)]);
        let green_only = full(&mut upper, 1);

        let mut half = stacked(cfg, &[visual(&red, 1.0), visual(&green, 0.5)]);
        let blended = full(&mut half, 2);

        for i in 0..BANDS {
            for c in 0..3 {
                let want = 0.5 * green_only[i][c] as f32 + 0.5 * red_only[i][c] as f32;
                assert!(
                    (blended[i][c] as f32 - want).abs() <= 1.5,
                    "point {i} channel {c}: got {}, expected about {want}",
                    blended[i][c]
                );
            }
        }
    }

    /// A fully transparent layer is not there. That property is what lets a
    /// layer be faded out and back in without disturbing the stack.
    #[test]
    fn a_transparent_layer_shows_what_is_underneath() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let red = flat_surface("#ff2000");
        let green = flat_surface("#40ff60");

        let mut alone = stacked(cfg.clone(), &[visual(&red, 1.0)]);
        let red_only = full(&mut alone, 1);

        let mut covered = stacked(cfg, &[visual(&red, 1.0), visual(&green, 0.0)]);
        assert_eq!(full(&mut covered, 2), red_only);
    }

    /// Black blocks. A layer whose field is opaque black hides the one below,
    /// which is the distinction the whole design rests on — it is *not* the
    /// same as being transparent, even though the two are indistinguishable
    /// over an unlit strip.
    #[test]
    fn opaque_black_blocks_and_transparent_black_does_not() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let red = flat_surface("#ff2000");

        let mut blocked =
            stacked(cfg.clone(), &[visual(&red, 1.0), visual(&flat_surface("#000000"), 1.0)]);
        assert!(
            full(&mut blocked, 2).iter().all(|px| px == &[0, 0, 0]),
            "opaque black did not block the layer below"
        );

        let mut passed =
            stacked(cfg.clone(), &[visual(&red, 1.0), visual(&flat_surface("#00000000"), 1.0)]);
        let mut alone = stacked(cfg, &[visual(&red, 1.0)]);
        assert_eq!(
            full(&mut passed, 2),
            full(&mut alone, 1),
            "transparent black blocked the layer below"
        );
    }

    /// The same distinction on the intensity axis, and the reason it matters: a
    /// band the threshold has closed is painted black, so it blocks. Wanting
    /// the layer below to show through in the gaps is expressed by authoring
    /// opacity at the bottom of the field instead.
    #[test]
    fn a_band_under_the_threshold_blocks_when_it_is_opaque() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let closed = IntensityConfig { threshold: -20.0, clamp: -6.0, curve: Default::default() };
        // -68 dB on the editor axis: far under that threshold.
        let quiet = vec![0.15f32; BANDS];

        let mut top = visual(&flat_surface("#40ff60"), 1.0);
        top.intensity = closed;
        let mut stack = stacked(cfg.clone(), &[visual(&flat_surface("#ff2000"), 1.0), top]);
        let out = stack.render_stack(&[&quiet, &quiet]).to_vec();
        assert!(
            out.iter().all(|px| px == &[0, 0, 0]),
            "a quiet opaque layer let the one below through: {:?}",
            out[0]
        );

        // Fade that same field out and the layer below returns, which is what
        // makes this a choice rather than a limitation.
        let mut clear_top = visual(&flat_surface("#40ff6000"), 1.0);
        clear_top.intensity = closed;
        let mut open = stacked(cfg, &[visual(&flat_surface("#ff2000"), 1.0), clear_top]);
        assert!(
            open.render_stack(&[&quiet, &quiet]).iter().any(|px| px.iter().any(|&c| c > 0)),
            "the transparent version blocked too, so the test proves nothing"
        );
    }

    /// A layer confined to part of the strip must leave the rest of the stack
    /// alone. Contributing black outside its sectors would turn any layer that
    /// uses them into a full-strip blackout.
    #[test]
    fn sectors_confine_a_layer_without_blanking_the_rest() {
        let cfg = RenderConfig { dither: false, ..Default::default() };

        let mut top = visual(&flat_surface("#40ff60"), 1.0);
        top.layout = LayoutConfig {
            // The upper half of the strip only.
            led_keyframes: vec![
                LedKeyframe { led: 75, hz: 20.0 },
                LedKeyframe { led: 149, hz: 20_000.0 },
            ],
            reverse: false,
            mirror: false,
        };

        let mut stack = stacked(cfg.clone(), &[visual(&flat_surface("#ff2000"), 1.0), top]);
        let out = full(&mut stack, 2);

        let mut alone = stacked(cfg, &[visual(&flat_surface("#ff2000"), 1.0)]);
        let red_only = full(&mut alone, 1);

        assert_eq!(out[0], red_only[0], "the sectored layer blanked a point it does not cover");
        assert_ne!(out[BANDS - 1], red_only[BANDS - 1], "the sectored layer covered nothing");
    }

    /// Layers can be reading different devices, so their level arrays can be
    /// different lengths. Each is read on its own axis and the two meet in
    /// strip space; nothing here may index one by the length of the other.
    #[test]
    fn layers_may_have_different_band_counts() {
        let cfg = RenderConfig { dither: false, ..Default::default() };

        let mut wide = visual(&flat_surface("#40ff60"), 1.0);
        wide.centers = (0..88).map(|i| norm_to_hz(i as f32 / 87.0)).collect();

        let mut stack = stacked(cfg, &[visual(&flat_surface("#ff2000"), 1.0), wide]);
        let low = vec![1.0f32; BANDS];
        let high = vec![1.0f32; 88];
        let out = stack.render_stack(&[&low, &high]).to_vec();
        assert_eq!(out.len(), BANDS);
        assert!(out.iter().any(|px| px.iter().any(|&c| c > 0)));
    }

    /// A layer whose source has not produced a frame yet is not silent, it is
    /// absent — and the difference is whether it blacks out everything below.
    #[test]
    fn a_layer_with_no_levels_yet_is_skipped() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let red = flat_surface("#ff2000");

        let mut alone = stacked(cfg.clone(), &[visual(&red, 1.0)]);
        let red_only = full(&mut alone, 1);

        let mut stack = stacked(cfg, &[visual(&red, 1.0), visual(&flat_surface("#000000"), 1.0)]);
        let levels = vec![1.0f32; BANDS];
        let out = stack.render_stack(&[&levels, &[]]).to_vec();
        assert_eq!(out, red_only);
    }

    /// The fold starts from nothing on every frame, so it cannot leak the
    /// previous one into the next.
    #[test]
    fn the_stack_does_not_accumulate_across_frames() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let mut stack = stacked(cfg, &[visual(&flat_surface("#40ff60"), 0.5)]);
        let first = full(&mut stack, 1);
        for _ in 0..8 {
            assert_eq!(full(&mut stack, 1), first, "the fold carried over a frame");
        }
    }

    /// Replacing the stack is all or nothing: a layer with an unparseable
    /// colour leaves the running stack exactly as it was, rather than dropping
    /// half of an edit.
    #[test]
    fn a_rejected_edit_leaves_the_running_stack_alone() {
        let cfg = RenderConfig { dither: false, ..Default::default() };
        let red = flat_surface("#ff2000");
        let mut stack = stacked(cfg, &[visual(&red, 1.0)]);
        let before = full(&mut stack, 1);

        let broken =
            SurfaceConfig { keyframes: vec![Keyframe::new(0.0, 0.0, "not a colour")], sigma: 0.25 };
        assert!(stack.set_layers(&[visual(&red, 1.0), visual(&broken, 1.0)]).is_err());
        assert_eq!(stack.layer_count(), 1);
        assert_eq!(full(&mut stack, 1), before);
    }
}
