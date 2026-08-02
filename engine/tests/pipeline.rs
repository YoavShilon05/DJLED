//! End-to-end: synthetic audio in, LED bytes out.
//!
//! Covers the seam between the analyser, the colour surface, the wire protocol
//! and the strip expansion — the parts that individually pass their own tests
//! but could still be wired together wrongly.

use djled_engine::color::{RenderConfig, Renderer, SurfaceConfig};
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
        let engine = Engine::new(&EngineConfig::default(), SR);
        let renderer = Renderer::new(
            RenderConfig::default(),
            &SurfaceConfig::default(),
            engine.band_count(),
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
