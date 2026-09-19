//! End-to-end: a preset on disk, out to the LED bytes.
//!
//! `presets.rs` has unit tests over its own store, and they prove a file round
//! trips. What they cannot catch is the failure that would actually be
//! reported: a preset that loads with every field intact and still lights the
//! wall differently from the show it was saved from — a field dropped between
//! the store and the renderer looks like a save bug and is not one.
//!
//! So these render. Same rule as `pipeline.rs`: a preset control is only
//! implemented once it has changed a byte on the wire.

use djled_engine::color::{LayerVisual, RenderConfig, Renderer};
use djled_engine::link::{Link, MockLink};
use djled_engine::presets::Presets;
use djled_engine::{Engine, EngineConfig, ShowConfig};

const SR: f64 = 48_000.0;
const LEDS: usize = 150;

/// A scratch file per test, so they can run in parallel and clean up after
/// themselves.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join("djled-preset-pipeline");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A show with a distinctive look: one layer, one flat colour, a threshold that
/// lets a test tone through.
fn show(color: &str, threshold: f32) -> ShowConfig {
    let mut show = ShowConfig::spanning(LEDS);
    let layer = &mut show.layers[0];
    layer.surface = djled_engine::color::SurfaceConfig {
        keyframes: vec![
            djled_engine::color::Keyframe::new(0.0, 0.0, color),
            djled_engine::color::Keyframe::new(1.0, 1.0, color),
        ],
        sigma: 0.5,
    };
    layer.threshold = threshold;
    show
}

/// Render a show the way the engine does: the stack's shape first, then the
/// colour half over it.
///
/// This is the `LiveStack::visuals` fold without the devices — the same
/// translation, against an analyser fed by hand. A real stack cannot be opened
/// in a test, and the part being defended here is not the capture.
fn render(show: &ShowConfig) -> Vec<[u8; 3]> {
    let mut engine = Engine::new(&EngineConfig::default(), SR);
    let visuals: Vec<LayerVisual> = show
        .layers
        .iter()
        .map(|l| LayerVisual {
            surface: l.surface.clone(),
            timeline: l.timeline.clone(),
            layout: l.layout(),
            intensity: l.intensity(),
            opacity: if l.enabled { l.opacity() } else { 0.0 },
            centers: engine.centers().to_vec(),
        })
        .collect();

    let mut renderer = Renderer::stacked(
        RenderConfig { dither: false, ..Default::default() },
        &visuals,
        engine.band_count(),
        LEDS,
    )
    .unwrap();
    let mut link = MockLink::new(LEDS);

    let block = (SR * 0.010) as usize;
    let mut buf = vec![0.0f32; block];
    let mut i = 0;
    while i < (SR * 1.5) as usize {
        for (k, s) in buf.iter_mut().enumerate() {
            let t = (i + k) as f64;
            *s = 0.5 * (std::f64::consts::TAU * 440.0 * t / SR).sin() as f32;
        }
        if engine.push(&buf) {
            let levels = engine.levels();
            let each: Vec<&[f32]> = vec![levels; renderer.layer_count()];
            link.send(renderer.render_stack(&each)).unwrap();
        }
        i += block;
    }

    link.leds().to_vec()
}

/// The whole promise of the feature: what you saved is what comes back, down to
/// the byte, after the engine has been restarted.
#[test]
fn a_preset_reloaded_from_disk_lights_the_strip_identically() {
    let scratch = Scratch::new("round-trip");

    let authored = show("#ff2000", -62.0);
    let mut presets = Presets::load(scratch.0.clone());
    presets.store(&authored);
    presets.flush();

    let reloaded = Presets::load(scratch.0.clone());
    assert_eq!(
        render(reloaded.active_show().expect("the slot came back empty")),
        render(&authored),
        "a preset round-tripped through the file lights the wall differently"
    );
}

/// ...and two slots are genuinely two shows. The reverse of the test above:
/// bytes identical after a reload only means something if different presets
/// produce different bytes at all.
#[test]
fn switching_slots_changes_what_reaches_the_wire() {
    let scratch = Scratch::new("switch");

    let mut presets = Presets::load(scratch.0.clone());
    presets.store(&show("#ff2000", -62.0));
    presets.select(1);
    presets.store(&show("#2040ff", -62.0));
    presets.flush();

    let presets = Presets::load(scratch.0.clone());
    let warm = render(presets.show(0).unwrap());
    let cool = render(presets.show(1).unwrap());
    assert_ne!(warm, cool, "two presets rendered the same strip");

    // Not merely different — the right way round, so a swap of the two slots
    // would fail rather than pass on inequality alone.
    let brightest = |leds: &[[u8; 3]]| {
        *leds.iter().max_by_key(|px| px.iter().map(|&c| c as u32).sum::<u32>()).unwrap()
    };
    let (w, c) = (brightest(&warm), brightest(&cool));
    assert!(w[0] > w[2], "slot 0 is not the warm preset: {w:?}");
    assert!(c[2] > c[0], "slot 1 is not the cool preset: {c:?}");
}

/// The engine restarts into the slot that was live, not into slot 1. A set
/// interrupted by a crash comes back to the show that was on the wall.
#[test]
fn the_live_slot_is_what_a_restart_puts_back_on_the_wall() {
    let scratch = Scratch::new("live-slot");

    let mut presets = Presets::load(scratch.0.clone());
    presets.store(&show("#ff2000", -62.0));
    presets.select(6);
    presets.store(&show("#2040ff", -62.0));
    presets.flush();

    let restarted = Presets::load(scratch.0.clone());
    assert_eq!(restarted.active(), 6);
    assert_eq!(
        render(restarted.active_show().unwrap()),
        render(restarted.show(6).unwrap()),
        "the engine came back on a different slot from the one it left on"
    );
}

/// A threshold saved in a preset has to reach the wall, not merely survive the
/// file. The control that decides whether anything shows at all is the one
/// worth pinning, and it is stored on the layer rather than in the surface —
/// two different paths out of the same struct.
#[test]
fn a_saved_threshold_reaches_the_leds() {
    let scratch = Scratch::new("threshold");

    let mut presets = Presets::load(scratch.0.clone());
    presets.store(&show("#ff2000", 0.0)); // nothing gets through
    presets.select(1);
    presets.store(&show("#ff2000", -62.0)); // everything does
    presets.flush();

    let presets = Presets::load(scratch.0.clone());
    assert!(
        render(presets.show(0).unwrap()).iter().all(|px| px == &[0, 0, 0]),
        "a preset's threshold did not reach the strip"
    );
    assert!(render(presets.show(1).unwrap()).iter().any(|px| px != &[0, 0, 0]));
}
