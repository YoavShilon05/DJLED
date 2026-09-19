//! End-to-end: synthetic audio in, LED bytes out.
//!
//! Covers the seam between the analyser, the colour surface, the wire protocol
//! and the strip expansion — the parts that individually pass their own tests
//! but could still be wired together wrongly.

use djled_engine::color::intensity::IntensityConfig;
use djled_engine::color::{
    Geometry, LayerVisual, LayoutConfig, RenderConfig, Renderer, SurfaceConfig, Timeline,
};
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

// ---------------------------------------------------------------------------
// The layer stack, end to end.
//
// The compositor has unit tests over its own maths. What they cannot catch is
// the failure that matters here: a layer being composited correctly and then
// dropped between the renderer and the wire, or a stack that quietly renders
// only its bottom layer because the levels never reached the top one.
// ---------------------------------------------------------------------------

/// Two layers driven by one analyser, which is what two layers on one device
/// really are.
struct Stack {
    engine: Engine,
    renderer: Renderer,
    link: MockLink,
}

impl Stack {
    fn new(layers: &[LayerVisual]) -> Self {
        let engine = Engine::new(&EngineConfig::default(), SR);
        let renderer = Renderer::stacked(
            RenderConfig { dither: false, ..Default::default() },
            layers,
            engine.band_count(),
            LEDS,
        )
        .unwrap();
        Self { engine, renderer, link: MockLink::new(LEDS) }
    }

    fn run(&mut self, secs: f64, signal: impl Fn(usize) -> f32) {
        let block = (SR * 0.010) as usize;
        let mut buf = vec![0.0f32; block];
        let mut i = 0;
        while i < (SR * secs) as usize {
            for (k, s) in buf.iter_mut().enumerate() {
                *s = signal(i + k);
            }
            if self.engine.push(&buf) {
                let levels = self.engine.levels();
                let each: Vec<&[f32]> = vec![levels; self.renderer.layer_count()];
                let pixels = self.renderer.render_stack(&each);
                self.link.send(pixels).unwrap();
            }
            i += block;
        }
    }

    fn brightest_led(&self) -> [u8; 3] {
        *self
            .link
            .leds()
            .iter()
            .max_by_key(|px| px.iter().map(|&c| c as u32).sum::<u32>())
            .unwrap()
    }
}

fn layer(color: &str, opacity: f32, centers: &[f32]) -> LayerVisual {
    LayerVisual {
        surface: SurfaceConfig {
            keyframes: vec![
                djled_engine::color::Keyframe::new(0.0, 0.0, color),
                djled_engine::color::Keyframe::new(1.0, 1.0, color),
            ],
            sigma: 0.5,
        },
        timeline: Timeline::default(),
        cycle: false,
        layout: LayoutConfig::spanning(LEDS),
        intensity: IntensityConfig::pass_through(),
        channel_colors: Vec::new(),
        opacity,
        centers: centers.to_vec(),
    }
}

/// A layer painting `color` only within `radius` of the bass end of its field.
///
/// Two keyframes rather than one because the radius is Euclidean over
/// (position, level) and the level axis is as tall as the position axis is
/// wide: confining a whole *column* of the field takes more reach than the
/// horizontal distance alone suggests.
fn confined_layer(color: &str, radius: f32, centers: &[f32]) -> LayerVisual {
    LayerVisual {
        surface: SurfaceConfig {
            keyframes: vec![
                djled_engine::color::Keyframe::within(0.0, 0.0, color, radius),
                djled_engine::color::Keyframe::within(0.0, 1.0, color, radius),
            ],
            sigma: 0.5,
        },
        ..layer(color, 1.0, centers)
    }
}

fn centers() -> Vec<f32> {
    Engine::new(&EngineConfig::default(), SR).centers().to_vec()
}

/// The claim the feature was asked for, all the way to the LED bytes: red under
/// green is green on the wall.
#[test]
fn the_top_layer_reaches_the_leds() {
    let c = centers();
    let mut stacked = Stack::new(&[layer("#ff2000", 1.0, &c), layer("#40ff60", 1.0, &c)]);
    stacked.run(1.5, tone(440.0));

    let mut alone = Stack::new(&[layer("#40ff60", 1.0, &c)]);
    alone.run(1.5, tone(440.0));

    assert_eq!(
        stacked.link.leds(),
        alone.link.leds(),
        "the top layer did not survive the trip to the wire"
    );

    // ...and it is genuinely the green one, not merely equal to something.
    let px = stacked.brightest_led();
    assert!(px[1] > px[0], "the strip is not showing the top layer: {px:?}");
}

/// A layer faded halfway lands between the two on the strip, rather than
/// halfway to black. This is the difference between opacity and brightness, and
/// it only becomes visible once there is something underneath.
#[test]
fn layer_opacity_blends_on_the_wall() {
    let c = centers();
    let mut blended = Stack::new(&[layer("#ff2000", 1.0, &c), layer("#40ff60", 0.5, &c)]);
    blended.run(1.5, tone(440.0));

    let mut red = Stack::new(&[layer("#ff2000", 1.0, &c)]);
    red.run(1.5, tone(440.0));
    let mut green = Stack::new(&[layer("#40ff60", 1.0, &c)]);
    green.run(1.5, tone(440.0));

    for (i, px) in blended.link.leds().iter().enumerate() {
        for (ch, &got) in px.iter().enumerate() {
            let want =
                0.5 * green.link.leds()[i][ch] as f32 + 0.5 * red.link.leds()[i][ch] as f32;
            assert!(
                (got as f32 - want).abs() <= 3.0,
                "LED {i} channel {ch}: got {got}, expected about {want}"
            );
        }
    }
}

/// The other half of the design: opaque black covers. A quiet top layer with an
/// opaque field blanks the strip, and the same layer authored transparent does
/// not — which is what makes "black blocks, opacity blends" a choice the editor
/// can actually express.
#[test]
fn opaque_black_blanks_the_strip_and_transparent_black_does_not() {
    let c = centers();

    let mut blocked = Stack::new(&[layer("#ff2000", 1.0, &c), layer("#000000", 1.0, &c)]);
    blocked.run(1.5, tone(440.0));
    assert!(
        blocked.link.leds().iter().all(|px| px == &[0, 0, 0]),
        "opaque black did not reach the wire as a blackout"
    );

    let mut passed = Stack::new(&[layer("#ff2000", 1.0, &c), layer("#00000000", 1.0, &c)]);
    passed.run(1.5, tone(440.0));
    let mut alone = Stack::new(&[layer("#ff2000", 1.0, &c)]);
    alone.run(1.5, tone(440.0));
    assert_eq!(passed.link.leds(), alone.link.leds(), "transparent black blanked the strip");
}

/// The wire is unchanged by the stack: still one colour per control point,
/// still 149 bytes, however many layers went into computing them. That is the
/// whole reason no firmware change was needed.
#[test]
fn the_wire_format_is_unchanged_by_the_stack() {
    let c = centers();
    let layers: Vec<LayerVisual> =
        (0..5).map(|_| layer("#40c0ff", 0.6, &c)).collect();
    let mut p = Stack::new(&layers);
    p.run(1.0, tone(440.0));

    assert_eq!(p.renderer.pixels().len(), BANDS);
    assert_eq!(p.link.last_frame_bytes(), 149);
    assert_eq!(p.link.leds().len(), LEDS);
}

/// A keyframe's area of effect, all the way to the LED bytes: past it the layer
/// paints nothing, so the strip shows what is underneath rather than the colour
/// the same keyframe would have reached with if it were unconfined.
///
/// Driven by a bass tone *and* a treble one, because an LED with no level is
/// black whatever is painted on it — the two ends of the strip both have to be
/// lit before covering one of them is a statement about coverage.
///
/// The `assert_ne` on the unconfined stack is the half that matters. Without it
/// this passes on a build where the radius is parsed, stored and then ignored.
#[test]
fn a_confined_keyframe_leaves_the_rest_of_the_strip_to_the_layer_below() {
    let c = centers();
    let both_ends = |i: usize| 0.5 * (tone(60.0)(i) + tone(8_000.0)(i));

    let mut alone = Stack::new(&[layer("#ff2000", 1.0, &c)]);
    alone.run(1.5, &both_ends);

    // Picked from the bottom layer rather than assumed, so the assertions below
    // are about coverage and not about whether anything is lit there at all.
    let treble = brightest_in(alone.link.leds(), LEDS * 2 / 3..LEDS);
    let bass = brightest_in(alone.link.leds(), 0..LEDS / 3);
    assert!(light(alone.link.leds()[treble]) > 0 && light(alone.link.leds()[bass]) > 0);

    let mut covered = Stack::new(&[layer("#ff2000", 1.0, &c), layer("#ffffff", 1.0, &c)]);
    covered.run(1.5, &both_ends);
    assert_ne!(
        covered.link.leds()[treble],
        alone.link.leds()[treble],
        "an unconfined top layer did not reach the treble end, so this proves nothing"
    );

    let mut confined =
        Stack::new(&[layer("#ff2000", 1.0, &c), confined_layer("#ffffff", 0.6, &c)]);
    confined.run(1.5, &both_ends);
    assert_eq!(
        confined.link.leds()[treble],
        alone.link.leds()[treble],
        "the area of effect did not stop the top layer covering the treble end"
    );
    assert_ne!(
        confined.link.leds()[bass],
        alone.link.leds()[bass],
        "the area of effect swallowed the keyframe where it was supposed to reach"
    );
}

/// A layer at the treble end of its field, confined, painting the *bass* end of
/// the strip — which is only possible because the two ends are the same place
/// once the axis is joined.
///
/// The uncycled half is what makes this a statement about the flag rather than
/// about the radius: the same layer with cycling off leaves the bass end to the
/// layer below, exactly as it did before the flag existed.
#[test]
fn a_cycling_layer_carries_a_colour_past_the_end_of_the_strip() {
    let c = centers();
    let both_ends = |i: usize| 0.5 * (tone(60.0)(i) + tone(8_000.0)(i));

    // Confined to the treble end of the field, tall enough to cover the level
    // axis there — the radius is Euclidean over (position, level).
    let top = |cycle: bool| LayerVisual {
        surface: SurfaceConfig {
            keyframes: vec![
                djled_engine::color::Keyframe::within(1.0, 0.0, "#ffffff", 0.6),
                djled_engine::color::Keyframe::within(1.0, 1.0, "#ffffff", 0.6),
            ],
            sigma: 0.5,
        },
        cycle,
        ..layer("#ffffff", 1.0, &c)
    };

    let mut alone = Stack::new(&[layer("#ff2000", 1.0, &c)]);
    alone.run(1.5, &both_ends);
    let bass = brightest_in(alone.link.leds(), 0..LEDS / 3);
    assert!(light(alone.link.leds()[bass]) > 0, "nothing is lit at the bass end to cover");

    let mut plain = Stack::new(&[layer("#ff2000", 1.0, &c), top(false)]);
    plain.run(1.5, &both_ends);
    assert_eq!(
        plain.link.leds()[bass],
        alone.link.leds()[bass],
        "a treble-confined layer reached the bass end without cycling"
    );

    let mut cycling = Stack::new(&[layer("#ff2000", 1.0, &c), top(true)]);
    cycling.run(1.5, &both_ends);
    assert_ne!(
        cycling.link.leds()[bass],
        alone.link.leds()[bass],
        "the area of effect did not wrap around to the bass end"
    );
}

/// Total light in one pixel.
fn light(px: [u8; 3]) -> u32 {
    px.iter().map(|&c| c as u32).sum()
}

fn brightest_in(leds: &[[u8; 3]], range: std::ops::Range<usize>) -> usize {
    range.max_by_key(|&i| light(leds[i])).unwrap()
}

/// A layer with no colour keyframes at all paints nothing — which is not the
/// same as painting black. The stack below must come through untouched, or
/// deleting the last keyframe would blank the wall instead of emptying one
/// layer.
#[test]
fn an_empty_colour_field_paints_nothing() {
    let c = centers();

    let empty = LayerVisual {
        surface: SurfaceConfig { keyframes: Vec::new(), sigma: 0.5 },
        ..layer("#ffffff", 1.0, &c)
    };

    let mut stacked = Stack::new(&[layer("#ff2000", 1.0, &c), empty]);
    stacked.run(1.5, tone(440.0));

    let mut alone = Stack::new(&[layer("#ff2000", 1.0, &c)]);
    alone.run(1.5, tone(440.0));

    assert_eq!(stacked.link.leds(), alone.link.leds(), "an empty field changed the strip");
}

// ---------------------------------------------------------------------------
// A layer that listens to nothing
// ---------------------------------------------------------------------------

/// A show driven through `LiveStack` rather than through an analyser the test
/// holds itself.
///
/// That is not incidental. A still layer has no device and no signal, so the
/// levels it paints with are invented by the stack — testing it any other way
/// would be testing a constant this file wrote down, which is exactly the class
/// of test `pipeline.rs` exists to be the opposite of. Nothing here opens a
/// device: a show of still layers alone is the one show that runs on a machine
/// with no audio hardware at all.
struct StillShow {
    stack: djled_engine::LiveStack,
    renderer: Renderer,
    link: MockLink,
}

impl StillShow {
    fn new(layers: Vec<djled_engine::Layer>) -> Self {
        // Through JSON, because normalising a show — filling the blank ids the
        // stack matches telemetry by — happens on the way in.
        let json = serde_json::to_string(&djled_engine::ShowConfig { layers }).unwrap();
        let show: djled_engine::ShowConfig = serde_json::from_str(&json).unwrap();

        let base = EngineConfig::default();
        let stack = djled_engine::LiveStack::open(&show, &base, 0.25);
        let renderer = Renderer::stacked(
            RenderConfig { dither: false, ..Default::default() },
            &stack.visuals(),
            stack.points().max(1),
            LEDS,
        )
        .unwrap();
        Self { stack, renderer, link: MockLink::new(LEDS) }
    }

    /// Render frames until `count` of them have reached the wire, or give up.
    ///
    /// A still layer reports on a clock rather than on arriving samples, so
    /// this waits the way the run loop does instead of assuming a poll produces
    /// a frame.
    fn run(&mut self, count: usize) -> usize {
        let mut frames = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while frames < count && std::time::Instant::now() < deadline {
            if !self.stack.poll() {
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
            let pixels = self.renderer.render_stack(&self.stack.all_levels());
            self.link.send(pixels).unwrap();
            frames += 1;
        }
        frames
    }

    /// The strip at one instant of the loop, with the clock moved there first.
    ///
    /// Mirrors the order the run loop uses — seek, then render — because a
    /// timeline is a function of the clock rather than of arriving samples, and
    /// rendering before seeking would show every frame one behind.
    fn at(&mut self, seconds: f64) -> Vec<[u8; 3]> {
        self.renderer.seek(seconds);
        let pixels = self.renderer.render_stack(&self.stack.all_levels());
        self.link.send(pixels).unwrap();
        self.link.leds().to_vec()
    }
}

/// A still layer spanning the strip, painted with one colour authored at `db`
/// on the editor's axis — 0 is the top of the field, -80 the bottom.
fn still_layer(color: &str, db: f32) -> djled_engine::Layer {
    let y = djled_engine::color::intensity::db_to_level(db);
    djled_engine::Layer {
        source: djled_engine::Source::nothing(),
        surface: SurfaceConfig {
            keyframes: vec![
                djled_engine::color::Keyframe::new(0.0, y, color),
                djled_engine::color::Keyframe::new(1.0, y, color),
            ],
            sigma: 0.5,
        },
        ..djled_engine::Layer::spanning(LEDS)
    }
}

/// The claim the feature was asked for, all the way to the LED bytes: a layer
/// listening to nothing lights the whole strip with the colour it was given,
/// with no audio anywhere in the chain.
#[test]
fn a_layer_with_no_source_lights_the_strip_on_its_own() {
    let mut show = StillShow::new(vec![still_layer("#ff2000", 0.0)]);
    assert!(show.run(3) > 0, "a show of still layers alone never reported a frame");

    let leds = show.link.leds();
    assert!(
        leds.iter().all(|px| light(*px) > 0),
        "a still layer left part of the strip dark: {:?}",
        leds.iter().position(|px| light(*px) == 0),
    );
    // Red, not merely lit: the authored colour is what reached the wire.
    assert!(
        leds.iter().all(|px| px[0] > px[1] && px[0] > px[2]),
        "the still layer is not the colour it was authored",
    );
}

/// The field is read along *one* row, so where a colour was authored on the
/// intensity axis cannot change what the strip does. A layer whose whole field
/// sits at the bottom of the plot paints exactly what the same field at the top
/// paints — which is what "the graph is one dimensional" has to mean once it
/// reaches the wall.
#[test]
fn a_still_layer_reads_its_field_along_one_row() {
    let mut low = StillShow::new(vec![still_layer("#40c0ff", -80.0)]);
    let mut high = StillShow::new(vec![still_layer("#40c0ff", 0.0)]);
    assert!(low.run(3) > 0 && high.run(3) > 0);

    assert_eq!(
        low.link.leds(),
        high.link.leds(),
        "a still layer's colour moved when its keyframes moved up the intensity axis",
    );
}

/// The intensity stage is taken out of a still layer's way, rather than left to
/// happen to agree. A threshold dragged to the top of the axis would close a
/// band that is *at* the top of the axis, so without this a layer whose whole
/// point is that it does not react would go black for a setting it does not
/// have a control for any more.
#[test]
fn a_still_layer_ignores_a_threshold_that_would_blank_it() {
    let closed = djled_engine::Layer { threshold: 0.0, clamp: 0.0, ..still_layer("#ffffff", 0.0) };
    let mut show = StillShow::new(vec![closed]);
    assert!(show.run(3) > 0);

    assert!(
        show.link.leds().iter().all(|px| light(*px) > 0),
        "a threshold blanked a layer with no signal to threshold",
    );
}

/// Still under reactive: the two compose like any other pair, so a still wash
/// is something to put a spectrum on top of rather than a mode the show is in.
#[test]
fn a_still_layer_sits_under_a_layer_that_paints_nothing() {
    let empty = djled_engine::Layer {
        surface: SurfaceConfig { keyframes: Vec::new(), sigma: 0.5 },
        ..still_layer("#ffffff", 0.0)
    };
    let mut stacked = StillShow::new(vec![still_layer("#ff2000", 0.0), empty]);
    let mut alone = StillShow::new(vec![still_layer("#ff2000", 0.0)]);
    assert!(stacked.run(3) > 0 && alone.run(3) > 0);

    assert_eq!(
        stacked.link.leds(),
        alone.link.leds(),
        "an empty field over a still one changed the strip",
    );
}

/// A still layer whose field is a loop of `colors`, spread evenly around it.
///
/// Still rather than reactive because it isolates what is being measured: with
/// no audio anywhere in the chain, anything that changes on the wire changed
/// because the clock moved.
fn animated_layer(colors: &[&str], length: f32, db: f32) -> djled_engine::Layer {
    let y = djled_engine::color::intensity::db_to_level(db);
    let field = |color: &str| SurfaceConfig {
        keyframes: vec![
            djled_engine::color::Keyframe::new(0.0, y, color),
            djled_engine::color::Keyframe::new(1.0, y, color),
        ],
        sigma: 0.5,
    };
    djled_engine::Layer {
        timeline: Timeline {
            enabled: true,
            length,
            keys: colors
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    djled_engine::color::TimelineKey::new(
                        i as f32 / colors.len() as f32,
                        field(c),
                    )
                })
                .collect(),
        },
        // The mirror a peer that knows nothing about timelines would paint. The
        // keys win, and the test below is what says so.
        surface: field(colors[0]),
        ..still_layer(colors[0], db)
    }
}

/// The claim the feature was asked for, all the way to the LED bytes: a layer's
/// timeline replaces its colour field as the loop runs, with no audio in the
/// chain at all.
#[test]
fn a_timeline_reaches_the_leds() {
    let mut show = StillShow::new(vec![animated_layer(&["#ff2000", "#2040ff"], 4.0, 0.0)]);

    // Phase 0 and phase 0.5 of a four second loop, taken from the same clock
    // the run loop reads.
    let start = show.at(0.0);
    let half = show.at(2.0);

    assert!(
        start.iter().all(|px| px[0] > px[2]),
        "the first key is not the warm colour it was authored: {:?}",
        start[0]
    );
    assert!(
        half.iter().all(|px| px[2] > px[0]),
        "half a loop later the second key had not arrived: {:?}",
        half[0]
    );
}

/// Between two keys the wall is between two colours, rather than holding one
/// until it switches. The whole difference between a timeline and a playlist.
#[test]
fn a_timeline_crossfades_rather_than_switching() {
    let mut show = StillShow::new(vec![animated_layer(&["#ff2000", "#2040ff"], 4.0, 0.0)]);

    let start = show.at(0.0)[LEDS / 2];
    let quarter = show.at(1.0)[LEDS / 2];
    let half = show.at(2.0)[LEDS / 2];

    assert!(
        quarter[2] > start[2] && quarter[2] < half[2],
        "the blue channel stepped instead of crossing: {start:?} then {quarter:?} then {half:?}",
    );
    assert!(
        quarter[0] < start[0] && quarter[0] > half[0],
        "the red channel stepped instead of crossing: {start:?} then {quarter:?} then {half:?}",
    );
}

/// The loop closes: a whole length later the wall is back where it started, to
/// the byte. A show left up for a set depends on this and nothing else.
#[test]
fn a_timeline_comes_back_round_to_where_it_started() {
    let mut show = StillShow::new(vec![animated_layer(&["#ff2000", "#2040ff"], 4.0, 0.0)]);

    let start = show.at(1_750_000_000.0);
    let later = show.at(1_750_000_004.0);
    assert_eq!(start, later, "a full loop did not return to the same bytes");
}

/// A still layer's field is read along one row, and a timeline's keys are no
/// exception — each of them is projected onto it. Without that, animating the
/// one kind of layer that has no level axis would paint whatever its keys
/// happened to hold along the top edge.
#[test]
fn a_still_layers_timeline_is_read_along_one_row() {
    let mut low = StillShow::new(vec![animated_layer(&["#40c0ff", "#ff2000"], 4.0, -80.0)]);
    let mut high = StillShow::new(vec![animated_layer(&["#40c0ff", "#ff2000"], 4.0, 0.0)]);

    for seconds in [0.0, 1.0, 2.0, 3.0] {
        assert_eq!(
            low.at(seconds),
            high.at(seconds),
            "at {seconds}s the timeline's colour moved with the intensity axis",
        );
    }
}

/// Every show written before timelines existed must be deaf to the clock. This
/// is the byte-identity half of that promise — the parse half is in `show.rs`.
#[test]
fn a_layer_without_a_timeline_ignores_the_clock() {
    let mut show = StillShow::new(vec![still_layer("#ff2000", 0.0)]);

    let start = show.at(0.0);
    for seconds in [0.5, 2.0, 1_750_000_000.0] {
        assert_eq!(show.at(seconds), start, "the clock moved a layer that does not animate");
    }
}
