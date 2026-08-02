//! Band levels to LED bytes.
//!
//! Output is RGB order. WS2812B is physically GRB, but FastLED handles that
//! reordering in the firmware via its template parameter, so the wire protocol
//! stays in the order humans expect.

use super::intensity::IntensityConfig;
use super::oklab::LinearRgb;
use super::strip::{ControlPoint, LayoutConfig, StripMap};
use super::surface::{ColorSurface, SurfaceConfig};

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

pub struct Renderer {
    surface: ColorSurface,
    cfg: RenderConfig,
    map: StripMap,
    /// Carried quantisation error per control point per channel.
    error: Vec<[f32; 3]>,
    pixels: Vec<[u8; 3]>,
}

impl Renderer {
    pub fn new(
        cfg: RenderConfig,
        surface: &SurfaceConfig,
        layout: LayoutConfig,
        intensity: IntensityConfig,
        geometry: Geometry,
    ) -> Result<Self, String> {
        let points = geometry.points;
        Ok(Self {
            surface: ColorSurface::new(surface)?,
            cfg,
            map: StripMap::new(layout, intensity, &geometry.centers, points, geometry.leds),
            error: vec![[0.0; 3]; points],
            pixels: vec![[0; 3]; points],
        })
    }

    pub fn set_layout(&mut self, layout: LayoutConfig) {
        self.map.set_layout(layout);
    }

    pub fn set_intensity(&mut self, intensity: IntensityConfig) {
        self.map.set_intensity(intensity);
    }

    pub fn layout(&self) -> &LayoutConfig {
        self.map.layout()
    }

    pub fn intensity(&self) -> &IntensityConfig {
        self.map.intensity()
    }

    pub fn config(&self) -> &RenderConfig {
        &self.cfg
    }

    pub fn set_config(&mut self, cfg: RenderConfig) {
        self.cfg = cfg;
    }

    /// Replace the colour surface, keeping dither state so an edit does not
    /// flash the strip.
    pub fn set_surface(&mut self, surface: &SurfaceConfig) -> Result<(), String> {
        self.surface = ColorSurface::new(surface)?;
        Ok(())
    }

    pub fn surface(&self) -> &ColorSurface {
        &self.surface
    }

    /// One RGB triplet per control point, ready for the wire.
    pub fn pixels(&self) -> &[[u8; 3]] {
        &self.pixels
    }

    pub fn render(&mut self, levels: &[f32]) -> &[[u8; 3]] {
        let trim = (self.cfg.gamma - 1.0).abs() > 1e-6;
        let master = self.cfg.brightness.clamp(0.0, 1.0);

        // Borrowed separately from `self.surface`, which the loop also needs.
        let points: &[ControlPoint] = self.map.map(levels);

        for (i, point) in points.iter().enumerate() {
            let lab = self.surface.sample(point.x, point.y);
            let mut rgb = lab.to_linear_rgb().clamped();

            if trim {
                rgb = LinearRgb::new(
                    rgb.r.powf(self.cfg.gamma),
                    rgb.g.powf(self.cfg.gamma),
                    rgb.b.powf(self.cfg.gamma),
                );
            }

            // The intensity curve and the master both scale linear light, which
            // is where a brightness belongs: the LED is driven linearly, so
            // halving the byte really does halve the light.
            let scale = 255.0 * master * point.gain.clamp(0.0, 1.0);
            let channels = [rgb.r * scale, rgb.g * scale, rgb.b * scale];

            for (c, &target) in channels.iter().enumerate() {
                let carried = if self.cfg.dither {
                    target + self.error[i][c]
                } else {
                    target
                };
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::strip::norm_to_hz;

    const BANDS: usize = 48;

    fn renderer(cfg: RenderConfig) -> Renderer {
        // Intensity stage disabled, so these keep measuring quantisation and
        // master brightness rather than the curve. It gets its own test below.
        renderer_with(cfg, IntensityConfig::pass_through())
    }

    fn renderer_with(cfg: RenderConfig, intensity: IntensityConfig) -> Renderer {
        let centers: Vec<f32> =
            (0..BANDS).map(|i| norm_to_hz(i as f32 / (BANDS - 1) as f32)).collect();
        Renderer::new(
            cfg,
            &SurfaceConfig::default(),
            LayoutConfig::spanning(150),
            intensity,
            Geometry { centers, points: BANDS, leds: 150 },
        )
        .unwrap()
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
}
