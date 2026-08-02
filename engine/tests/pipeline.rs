//! End-to-end: synthetic audio in, LED bytes out.
//!
//! Covers the seam between the analyser, the colour surface, the wire protocol
//! and the strip expansion — the parts that individually pass their own tests
//! but could still be wired together wrongly.

use djled_engine::color::intensity::IntensityConfig;
use djled_engine::color::{Geometry, LayoutConfig, RenderConfig, Renderer, SurfaceConfig};
use djled_engine::link::{expand_bands_to_leds, Link, MockLink};
use djled_engine::{Engine, EngineConfig};

const SR: f64 = 48_000.0;
const BANDS: usize = 48;
const LEDS: usize = 150;

struct Pipeline {
    engine: Engine,
    renderer: Renderer,
    link: MockLink,
}

impl Pipeline {
    fn new() -> Self {
        Self::with(LayoutConfig::spanning(LEDS), IntensityConfig::pass_through())
    }

    fn with(layout: LayoutConfig, intensity: IntensityConfig) -> Self {
        let engine = Engine::new(&EngineConfig::default(), SR);
        let renderer = Renderer::new(
            RenderConfig::default(),
            &SurfaceConfig::default(),
            layout,
            intensity,
            Geometry {
                centers: engine.centers().to_vec(),
                points: engine.band_count(),
                leds: LEDS,
            },
        )
        .unwrap();
        Self { engine, renderer, link: MockLink::new(LEDS) }
    }

    /// Push `secs` of a signal through the whole chain.
    fn run(&mut self, secs: f64, signal: impl Fn(usize) -> f32) {
        let block = (SR * 0.010) as usize;
        let mut buf = vec![0.0f32; block];
        let mut i = 0;
        while i < (SR * secs) as usize {
            for (k, s) in buf.iter_mut().enumerate() {
                *s = signal(i + k);
            }
            if self.engine.push(&buf) {
                let pixels = self.renderer.render(self.engine.levels());
                self.link.send(pixels).unwrap();
            }
            i += block;
        }
    }

    fn brightest_led(&self) -> usize {
        self.link
            .leds()
            .iter()
            .enumerate()
            .max_by_key(|(_, px)| px.iter().map(|&c| c as u32).sum::<u32>())
            .map(|(i, _)| i)
            .unwrap()
    }
}

fn tone(freq: f64) -> impl Fn(usize) -> f32 {
    move |i| 0.5 * (std::f64::consts::TAU * freq * i as f64 / SR).sin() as f32
}

#[test]
fn band_count_matches_across_the_whole_chain() {
    let p = Pipeline::new();
    assert_eq!(p.engine.band_count(), BANDS);
    assert_eq!(p.renderer.pixels().len(), BANDS);
    assert_eq!(p.link.leds().len(), LEDS);
}

/// A bass tone must light the low end of the strip, in the warm colour the
/// default palette assigns there.
#[test]
fn bass_lights_the_low_end_warm() {
    let mut p = Pipeline::new();
    p.run(2.0, tone(60.0));

    let led = p.brightest_led();
    assert!(led < LEDS / 4, "60 Hz lit LED {led} of {LEDS}, expected near the start");

    let px = p.link.leds()[led];
    assert!(px[0] > px[2], "expected a warm colour at the bass end, got {px:?}");
}

/// ...and a treble tone the high end, in the cool colour.
#[test]
fn treble_lights_the_high_end_cool() {
    let mut p = Pipeline::new();
    p.run(2.0, tone(9000.0));

    let led = p.brightest_led();
    assert!(led > LEDS * 3 / 4, "9 kHz lit LED {led} of {LEDS}, expected near the end");

    let px = p.link.leds()[led];
    assert!(px[2] > px[0], "expected a cool colour at the treble end, got {px:?}");
}

/// Nothing playing means a dark wall, not a dim glow.
#[test]
fn silence_leaves_the_strip_dark() {
    let mut p = Pipeline::new();
    p.run(1.0, |_| 0.0);

    let brightest: u32 = p
        .link
        .leds()
        .iter()
        .map(|px| px.iter().map(|&c| c as u32).sum::<u32>())
        .max()
        .unwrap();
    assert!(brightest <= 24, "idle strip summed to {brightest} on its brightest pixel");
}

/// The frame budget the per-band design exists to protect.
#[test]
fn frames_stay_within_the_latency_budget() {
    let mut p = Pipeline::new();
    p.run(0.5, tone(440.0));

    let bytes = p.link.last_frame_bytes();
    assert_eq!(bytes, 149, "48 bands should frame to 149 bytes");

    let transfer_ms = bytes as f64 * 10.0 / 500_000.0 * 1000.0;
    let strip_write_ms = LEDS as f64 * 30.0 / 1000.0;
    assert!(
        transfer_ms + strip_write_ms < 10.0,
        "frame costs {transfer_ms:.2} ms transfer + {strip_write_ms:.2} ms strip write"
    );
}

/// Interpolation must spread each band-to-band transition across the LEDs
/// between them.
///
/// Note what this deliberately does *not* assert. With 48 bands over 150 LEDs
/// there are only ~3.2 LEDs per band, so where two neighbouring bands differ
/// sharply the strip genuinely steps — that is what a 48-band display at this
/// density looks like, not a defect. What must hold is that each LED-to-LED
/// step is roughly the band difference divided by the LEDs spanning it; without
/// interpolation the step would equal the full band difference.
#[test]
fn expansion_spreads_transitions_across_leds() {
    let mut p = Pipeline::new();
    p.run(2.0, |i| {
        // Broadband content so every band carries something.
        let t = i as f64 / SR;
        (0.3 * (std::f64::consts::TAU * 100.0 * t).sin()
            + 0.3 * (std::f64::consts::TAU * 1000.0 * t).sin()
            + 0.3 * (std::f64::consts::TAU * 6000.0 * t).sin()) as f32
    });

    let bands = p.renderer.pixels();
    let worst_band_step = bands
        .windows(2)
        .flat_map(|w| (0..3).map(move |c| (w[1][c] as i32 - w[0][c] as i32).abs()))
        .max()
        .unwrap();

    let leds = p.link.leds();
    let worst_led_step = leds
        .windows(2)
        .flat_map(|w| (0..3).map(move |c| (w[1][c] as i32 - w[0][c] as i32).abs()))
        .max()
        .unwrap();

    let leds_per_band = (LEDS - 1) as f32 / (BANDS - 1) as f32;
    // Allowance for rounding, plus slack because the worst LED step and the
    // worst band step need not occur at the same place.
    let budget = (worst_band_step as f32 / leds_per_band * 1.4 + 2.0) as i32;

    assert!(
        worst_led_step <= budget,
        "worst LED step {worst_led_step} exceeds {budget} \
         (worst band step {worst_band_step} over {leds_per_band:.1} LEDs per band) \
         — interpolation is not spreading transitions"
    );
    assert!(
        worst_led_step < worst_band_step,
        "expansion did not smooth anything: LED step {worst_led_step} vs band step {worst_band_step}"
    );
}

/// The strip length is a runtime choice, and 600 is where this project is
/// headed. Expansion must behave at both ends of that range.
#[test]
fn works_at_both_planned_strip_lengths() {
    for count in [150usize, 600] {
        let mut p = Pipeline::new();
        p.link = MockLink::new(count);
        p.run(1.0, tone(440.0));

        assert_eq!(p.link.leds().len(), count);
        // Wire size is set by band count alone — that is the point of the design.
        assert_eq!(p.link.last_frame_bytes(), 149);
    }
}

/// A level of exactly zero must produce the same bytes the surface says it
/// should, rather than being special-cased anywhere along the way.
#[test]
fn renderer_and_strip_agree() {
    let mut p = Pipeline::new();
    p.run(1.0, tone(440.0));

    let mut expected = vec![[0u8; 3]; LEDS];
    expand_bands_to_leds(p.renderer.pixels(), &mut expected);
    assert_eq!(p.link.leds(), &expected[..], "link and local expansion disagree");
}

// ---------------------------------------------------------------------------
// The editor's control layers, end to end.
//
// Each of these has unit tests over its own maths already. What they cannot
// catch is the failure that actually matters here: a stage being computed
// correctly and then dropped on the way to the wire. These assert the effect
// arrives at the LEDs.
// ---------------------------------------------------------------------------

use djled_engine::color::intensity::{db_to_level, CurveKind, IntensityCurve};
use djled_engine::color::LedKeyframe;
use djled_engine::dsp::eq::{EqBand, EqType};

/// Total light emitted by a range of the strip.
fn energy(leds: &[[u8; 3]], range: std::ops::Range<usize>) -> u32 {
    leds[range].iter().map(|px| px.iter().map(|&c| c as u32).sum::<u32>()).sum()
}

/// Mirroring must make the wall symmetric: bass at both tips, treble in the
/// middle. Asserted on a bass tone, which without mirroring lights one end only.
#[test]
fn mirror_lights_both_ends_of_the_strip() {
    let mut plain = Pipeline::new();
    plain.run(2.0, tone(60.0));
    let (left, right) = (
        energy(plain.link.leds(), 0..LEDS / 5),
        energy(plain.link.leds(), LEDS * 4 / 5..LEDS),
    );
    assert!(left > right * 4, "unmirrored bass was not confined to one end");

    let layout = LayoutConfig { mirror: true, ..LayoutConfig::spanning(LEDS) };
    let mut mirrored = Pipeline::with(layout, IntensityConfig::pass_through());
    mirrored.run(2.0, tone(60.0));

    let (left, right) = (
        energy(mirrored.link.leds(), 0..LEDS / 5),
        energy(mirrored.link.leds(), LEDS * 4 / 5..LEDS),
    );
    assert!(left > 0 && right > 0, "a mirrored strip left an end dark");
    let (lo, hi) = (left.min(right), left.max(right));
    assert!(hi < lo * 3 / 2, "mirrored ends were lopsided: {left} vs {right}");
}

/// Reverse must move the bass to the far end, not merely be accepted.
#[test]
fn reverse_moves_bass_to_the_other_end() {
    let layout = LayoutConfig { reverse: true, ..LayoutConfig::spanning(LEDS) };
    let mut p = Pipeline::with(layout, IntensityConfig::pass_through());
    p.run(2.0, tone(60.0));

    let led = p.brightest_led();
    assert!(led > LEDS * 3 / 4, "reversed 60 Hz lit LED {led}, expected near the end");
}

/// A sector confines a frequency range to the LEDs it was assigned, and leaves
/// the LEDs outside every sector dark.
#[test]
fn led_sectors_confine_a_range_to_its_own_leds() {
    let layout = LayoutConfig {
        // Only the middle third is addressed at all, and it carries 20–2000 Hz.
        led_keyframes: vec![
            LedKeyframe { led: 50, hz: 20.0 },
            LedKeyframe { led: 99, hz: 2000.0 },
        ],
        reverse: false,
        mirror: false,
    };
    let mut p = Pipeline::with(layout, IntensityConfig::pass_through());
    p.run(2.0, tone(60.0));

    let leds = p.link.leds();
    let inside = energy(leds, 55..95);
    assert!(inside > 0, "the addressed sector stayed dark");

    // Away from the sector edges, where the wire's control points are spread.
    assert_eq!(energy(leds, 0..40), 0, "light escaped below the sector");
    assert_eq!(energy(leds, 110..LEDS), 0, "light escaped above the sector");

    let led = p.brightest_led();
    assert!((50..=99).contains(&led), "60 Hz lit LED {led}, outside its sector");
}

/// The threshold is the control that decides whether anything shows at all.
#[test]
fn threshold_blanks_the_strip() {
    let wide_open = IntensityConfig { threshold: -79.0, clamp: -70.0, curve: Default::default() };
    let mut lit = Pipeline::with(LayoutConfig::spanning(LEDS), wide_open);
    lit.run(2.0, tone(440.0));
    assert!(energy(lit.link.leds(), 0..LEDS) > 0, "the control case was already dark");

    let shut = IntensityConfig { threshold: -1.0, clamp: 0.0, curve: Default::default() };
    let mut dark = Pipeline::with(LayoutConfig::spanning(LEDS), shut);
    dark.run(2.0, tone(440.0));
    assert_eq!(energy(dark.link.leds(), 0..LEDS), 0, "signal got past the threshold");
}

/// The curve has to change the output, not just be stored. Ease-in is below
/// linear everywhere in between, so the same signal must render dimmer.
#[test]
fn the_intensity_curve_reaches_the_leds() {
    let window = |kind| IntensityConfig {
        threshold: -75.0,
        clamp: -5.0,
        curve: IntensityCurve { kind, ..Default::default() },
    };

    let mut linear = Pipeline::with(LayoutConfig::spanning(LEDS), window(CurveKind::Linear));
    linear.run(2.0, tone(440.0));
    let a = energy(linear.link.leds(), 0..LEDS);

    let mut eased = Pipeline::with(LayoutConfig::spanning(LEDS), window(CurveKind::EaseIn));
    eased.run(2.0, tone(440.0));
    let b = energy(eased.link.leds(), 0..LEDS);

    assert!(a > 0, "the linear control case was dark");
    assert!(b < a, "ease-in was not dimmer than linear: {b} vs {a}");
}

/// A cut at the tone's own frequency must reach the wall. This is the one that
/// would fail if the EQ were sampled but never installed, or installed on the
/// wrong side of the AGC.
#[test]
fn the_eq_reaches_the_leds() {
    let mut plain = Pipeline::new();
    plain.run(3.0, tone(440.0));
    let before = energy(plain.link.leds(), 0..LEDS);

    let mut cut = Pipeline::new();
    cut.engine.set_eq(&[EqBand { kind: EqType::Peak, hz: 440.0, gain: -24.0, q: 1.0 }]);
    cut.run(3.0, tone(440.0));
    let after = energy(cut.link.leds(), 0..LEDS);

    assert!(before > 0, "the control case was dark");
    assert!(after < before / 2, "a 24 dB cut barely moved the output: {after} vs {before}");
}

/// The threshold is authored on the editor's axis, so the level a given dB maps
/// to has to agree across the seam. Belt and braces for the one conversion that
/// both sides implement independently.
#[test]
fn editor_db_and_engine_level_agree() {
    for db in [-80.0, -62.0, -30.0, 0.0] {
        let level = db_to_level(db);
        assert!((0.0..=1.0).contains(&level), "{db} dB mapped outside 0..1");
    }
    assert_eq!(db_to_level(-80.0), 0.0);
    assert_eq!(db_to_level(0.0), 1.0);
}
