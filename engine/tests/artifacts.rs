//! End-to-end artefact tests.
//!
//! These encode the bug that motivated the project: with short-window real-time
//! FFT, low-frequency bars light up even for a static pure tone. The measured
//! cause was a DC offset — inaudible at -40 dBFS — landing in bin 0-1, which at
//! short FFT sizes *is* the sub-bass band.

use djled_engine::dsp::fastpath::FastPathConfig;
use djled_engine::dsp::window::WindowKind;
use djled_engine::{Engine, EngineConfig};

const SR: f64 = 48_000.0;

/// Threshold at which a bar becomes visible, relative to the loudest bar. Ties
/// to the -70 dB default floor in `PostConfig`.
const VISIBLE_DB: f32 = -70.0;

fn run(cfg: &EngineConfig, secs: f64, signal: impl Fn(usize) -> f32) -> Engine {
    let mut engine = Engine::new(cfg, SR);
    let total = (SR * secs) as usize;
    // Pushed in 10 ms blocks, matching what WASAPI actually delivers.
    let block = (SR * 0.010) as usize;
    let mut buf = vec![0.0f32; block];
    let mut i = 0;
    while i < total {
        for (k, s) in buf.iter_mut().enumerate() {
            *s = signal(i + k);
        }
        engine.push(&buf);
        i += block;
    }
    engine
}

/// Level of each band relative to the loudest band, in dB.
fn relative_db(engine: &Engine) -> Vec<f32> {
    let mags = engine.magnitudes();
    let peak = mags.iter().cloned().fold(0.0f32, f32::max).max(1e-30);
    mags.iter().map(|&m| 20.0 * (m / peak).log10()).collect()
}

/// The exact signal from the diagnosis: a 1 kHz tone at -6 dBFS riding on a
/// 0.01 DC offset.
fn tone_with_dc(i: usize) -> f32 {
    0.5 * (std::f64::consts::TAU * 1000.0 * i as f64 / SR).sin() as f32 + 0.01
}

/// The headline regression test. A clean tone must leave every bar below 200 Hz
/// dark, despite an inaudible DC offset on the input.
#[test]
fn static_tone_does_not_light_low_frequency_bars() {
    let engine = run(&EngineConfig::default(), 2.0, tone_with_dc);
    let rel = relative_db(&engine);

    for (i, band) in engine.plan().bands.iter().enumerate() {
        if band.hi <= 200.0 {
            assert!(
                rel[i] < VISIBLE_DB,
                "ghost bar: band {i} ({:.0}-{:.0} Hz) at {:.1} dB below peak",
                band.lo, band.hi, rel[i]
            );
        }
    }
}

/// Confirms the artefact these tests guard against is real, so the suppression
/// results above are not merely asserting that nothing ever happens.
///
/// This deliberately does *not* use the crate's analyser. It computes, with a
/// local single-bin DFT, what the obvious first implementation would see: one
/// 1024-point FFT for the whole spectrum, with bin 1 read as "sub-bass". At that
/// size a bin is 47 Hz wide, so bin 1 sits squarely in the sub-bass region and
/// collects the DC smear — while the tone itself is up at bin 21.
///
/// Going through an independent path matters. The analyser refuses to read those
/// bins at all, so it structurally cannot reproduce the bug; measuring the naive
/// result separately is the only way to show the hazard is genuine rather than
/// hypothetical.
#[test]
fn a_naive_analyser_would_show_the_ghost_bar() {
    const N: usize = 1024;

    let window = |i: usize| {
        let t = std::f64::consts::TAU * i as f64 / N as f64;
        0.35875 - 0.48829 * t.cos() + 0.14128 * (2.0 * t).cos() - 0.01168 * (3.0 * t).cos()
    };
    let bin_magnitude = |k: usize| {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for i in 0..N {
            let s = tone_with_dc(i) as f64 * window(i);
            let a = std::f64::consts::TAU * k as f64 * i as f64 / N as f64;
            re += s * a.cos();
            im -= s * a.sin();
        }
        re.hypot(im)
    };

    let ghost = bin_magnitude(1);
    let tone = bin_magnitude(21);
    let rel = 20.0 * (ghost / tone).log10();

    assert!(
        rel > VISIBLE_DB as f64,
        "the naive analyser was expected to ghost, but bin 1 sat {rel:.1} dB down"
    );

    // The same signal through this crate's analyser leaves the region dark.
    let engine = run(&EngineConfig::default(), 2.0, tone_with_dc);
    let ours = relative_db(&engine);
    let worst = engine
        .plan()
        .bands
        .iter()
        .enumerate()
        .filter(|(_, b)| b.hi <= 200.0)
        .map(|(i, _)| ours[i])
        .fold(f32::NEG_INFINITY, f32::max);

    assert!(
        worst < VISIBLE_DB,
        "naive analyser ghosts at {rel:.1} dB and ours at {worst:.1} dB, which is still visible"
    );
}

/// The structural reason the default configuration never needs that rescue: the
/// tier rule forces the lowest band onto an FFT large enough that the discarded
/// bins — where DC and its window smear live — stop below the band's lower edge.
/// This is why disabling the DC blocker does not resurrect the artefact.
#[test]
fn lowest_band_sits_clear_of_the_discarded_bins() {
    let engine = Engine::new(&EngineConfig::default(), SR);
    let band = &engine.plan().bands[0];
    let bin = SR / band.fft_size as f64;
    let smear_top = WindowKind::BlackmanHarris.discard_bins() as f64 * bin;

    assert!(
        band.lo > smear_top,
        "band 0 starts at {:.1} Hz but DC smear on N={} reaches {:.1} Hz",
        band.lo, band.fft_size, smear_top
    );
}

/// Leakage, as distinct from DC. Even with the offset removed, a tone must not
/// smear across the display — this is what the Blackman-Harris window buys.
#[test]
fn clean_tone_stays_confined_to_its_own_region() {
    let engine = run(&EngineConfig::default(), 2.0, |i| {
        0.5 * (std::f64::consts::TAU * 1000.0 * i as f64 / SR).sin() as f32
    });
    let rel = relative_db(&engine);

    for (i, band) in engine.plan().bands.iter().enumerate() {
        // Anything more than an octave away from the tone should be dark.
        if band.hi < 500.0 || band.lo > 2000.0 {
            assert!(
                rel[i] < VISIBLE_DB,
                "1 kHz leaked into band {i} ({:.0}-{:.0} Hz) at {:.1} dB",
                band.lo, band.hi, rel[i]
            );
        }
    }
}

/// Silence must be black. Any self-noise shows up as a permanently shimmering
/// strip on the wall.
#[test]
fn silence_is_black() {
    let engine = run(&EngineConfig::default(), 1.0, |_| 0.0);
    assert!(
        engine.levels().iter().all(|&v| v < 0.01),
        "silence produced levels up to {}",
        engine.levels().iter().cloned().fold(0.0f32, f32::max)
    );
}

/// A swept tone must move the lit band monotonically upward. Catches band
/// ordering errors, tier-boundary discontinuities, and mirrored spectra.
#[test]
fn swept_tone_moves_upward_monotonically() {
    let mut engine = Engine::new(&EngineConfig::default(), SR);
    let block = (SR * 0.050) as usize;
    let mut phase = 0.0f64;
    let mut previous = 0usize;
    let mut buf = vec![0.0f32; block];
    let mut regressions = 0;

    // Log sweep, 100 Hz to 8 kHz, slow enough for every tier to keep up.
    for step in 0..120 {
        let freq = 100.0 * (8000f64 / 100.0).powf(step as f64 / 119.0);
        for s in buf.iter_mut() {
            phase += std::f64::consts::TAU * freq / SR;
            *s = phase.sin() as f32;
        }
        engine.push(&buf);

        if step > 10 {
            let hot = engine
                .magnitudes()
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(i, _)| i)
                .unwrap();
            if hot < previous {
                regressions += 1;
            }
            previous = hot;
        }
    }

    assert_eq!(regressions, 0, "lit band moved backwards during an upward sweep");
}

/// With the fast path disabled the display must still be correct, just slower.
/// Guards against the accurate path silently depending on the fast one.
#[test]
fn artefact_suppression_does_not_depend_on_the_fast_path() {
    let cfg = EngineConfig { fast_path: None, ..Default::default() };
    let engine = run(&cfg, 2.0, tone_with_dc);
    let rel = relative_db(&engine);

    for (i, band) in engine.plan().bands.iter().enumerate() {
        if band.hi <= 200.0 {
            assert!(rel[i] < VISIBLE_DB, "ghost bar without fast path: band {i} at {:.1} dB", rel[i]);
        }
    }
}

/// A single un-cascaded bandpass leaks a 1 kHz tone into the 52 Hz band at
/// -31 dB. Cascading plus gating is what keeps the fast path from reintroducing
/// the artefact; this pins that down at the level of the whole engine.
#[test]
fn fast_path_does_not_reintroduce_the_ghost_bar() {
    let cfg = EngineConfig {
        fast_path: Some(FastPathConfig { stages: 1, ..Default::default() }),
        ..Default::default()
    };
    let engine = run(&cfg, 2.0, tone_with_dc);
    let rel = relative_db(&engine);

    // Even with the deliberately leaky single-stage filter, the STFT peak-hold
    // gate must keep the bass bars dark.
    for (i, band) in engine.plan().bands.iter().enumerate() {
        if band.hi <= 200.0 {
            assert!(
                rel[i] < VISIBLE_DB,
                "single-stage fast path leaked into band {i} ({:.0}-{:.0} Hz) at {:.1} dB",
                band.lo, band.hi, rel[i]
            );
        }
    }
}
