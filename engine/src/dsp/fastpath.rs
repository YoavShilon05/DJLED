//! Low-latency parallel estimate for the slow tiers.
//!
//! The bass tiers use windows of 43-171 ms because that is genuinely what it
//! takes to resolve those bands — the uncertainty principle is not negotiable.
//! But the *attack* of a kick drum does not need frequency resolution at all,
//! only energy detection, and that can be had in a few milliseconds.
//!
//! So every band served by a slow tier also gets a bandpass tuned to its edges,
//! feeding an envelope follower that runs per-sample. The final level is
//! `max(stft_magnitude, gated_fast_envelope)`: the STFT contributes an accurate
//! steady-state reading, the biquad contributes the leading edge.
//!
//! # Two things make this safe rather than a source of new artefacts
//!
//! **A single biquad is nowhere near selective enough.** A 2-pole bandpass rolls
//! off only 6 dB/octave, so a 1 kHz tone leaks into a 52 Hz band at -31 dB —
//! comfortably visible, and precisely the phantom-bass artefact the STFT design
//! works so hard to eliminate. Cascading identical sections fixes the far field
//! (measured: -31 dB at one stage, -55 at two, -77 at three).
//!
//! **But steepness alone is not enough.** Three stages still pass a 300 Hz tone
//! into the 52 Hz band at about -45 dB, which maps to a plainly lit bar. No
//! practical filter escapes this: selectivity and speed trade against each other
//! exactly as they do for the FFT.
//!
//! The resolution is to stop treating the fast path as an independent level
//! meter and treat it as what it actually is — a *transient accelerator*. Its
//! output is clamped to a slowly-decaying peak hold of the band's own STFT
//! magnitude, so it can only ever pull a band up toward a level the accurate
//! path has recently confirmed. A steady 300 Hz tone leaves the 52 Hz STFT bin
//! dark, the peak hold sits at zero, and the fast path is clamped to nothing.
//! The ghost bar cannot appear.
//!
//! The cost is that the very first transient after silence arrives at STFT
//! speed, since nothing has confirmed the band yet. The hold outlasts a musical
//! beat, so every subsequent hit is fast — and a single late onset after a break
//! is imperceptible.

use super::bands::BandPlan;
use super::biquad::{coefficient, Biquad, EnvelopeFollower};

/// Peak-to-RMS conversion for a sinusoid. The envelope follower tracks the peak
/// of the bandpassed signal, while the STFT reports RMS; without this the fast
/// path would read 3 dB hot and always win the `max`, quietly disabling the
/// accurate path entirely.
const PEAK_TO_RMS: f32 = std::f32::consts::FRAC_1_SQRT_2;

#[derive(Clone, Debug)]
pub struct FastPathConfig {
    /// Bands whose tier window is longer than this get a fast path. Shorter
    /// tiers are already responsive enough that the extra filter earns nothing.
    pub min_span_ms: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    /// Bandpass sections cascaded per band. Three gives ~77 dB rejection an
    /// octave-and-a-half out; one gives 31 dB, which is not enough.
    pub stages: usize,
    /// Decay time of the STFT peak hold that clamps this path. Must outlast the
    /// gap between beats (400 ms covers anything above 150 bpm) but stay short
    /// enough that the gate reseals promptly when a band genuinely goes quiet.
    pub hold_ms: f32,
}

impl Default for FastPathConfig {
    fn default() -> Self {
        Self {
            min_span_ms: 40.0,
            attack_ms: 2.0,
            release_ms: 90.0,
            stages: 3,
            hold_ms: 400.0,
        }
    }
}

struct Entry {
    band: usize,
    stages: Vec<Biquad>,
    envelope: EnvelopeFollower,
    /// Decaying peak of the band's confirmed STFT magnitude.
    hold: f32,
}

pub struct FastPath {
    entries: Vec<Entry>,
    hold_decay: f32,
}

impl FastPath {
    pub fn new(plan: &BandPlan, cfg: &FastPathConfig) -> Self {
        let sr = plan.sample_rate as f32;
        let dt = 1.0 / sr;
        let stages = cfg.stages.max(1);

        // Cascading n identical sections narrows the combined -3 dB width by
        // sqrt(2^(1/n) - 1), so each section is widened by that factor to keep
        // the cascade matched to the band it stands in for.
        let widen = (2f32.powf(1.0 / stages as f32) - 1.0).sqrt();

        let entries = plan
            .bands
            .iter()
            .enumerate()
            .filter(|(_, b)| (b.fft_size as f32 / sr) * 1000.0 > cfg.min_span_ms)
            .map(|(i, b)| {
                let q_target = (b.center / b.width()).max(0.5) as f32;
                let q_stage = (q_target * widen).max(0.25);
                Entry {
                    band: i,
                    stages: vec![Biquad::bandpass(b.center as f32, q_stage, sr); stages],
                    envelope: EnvelopeFollower::new(
                        cfg.attack_ms / 1000.0,
                        cfg.release_ms / 1000.0,
                        dt,
                    ),
                    hold: 0.0,
                }
            })
            .collect();

        // Applied once per analysis frame, not per sample.
        let hop_dt = super::mrstft::DEFAULT_HOP as f32 / sr;
        Self {
            entries,
            hold_decay: 1.0 - coefficient(cfg.hold_ms / 1000.0, hop_dt),
        }
    }

    /// Number of bands carrying a fast path.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn push(&mut self, samples: &[f32]) {
        for entry in &mut self.entries {
            for &s in samples {
                let mut v = s;
                for stage in &mut entry.stages {
                    v = stage.process(v);
                }
                entry.envelope.process(v);
            }
        }
    }

    /// Raise `levels` toward the fast estimate, clamped by what `stft` has
    /// recently confirmed for each band.
    pub fn apply(&mut self, stft: &[f32], levels: &mut [f32]) {
        for entry in &mut self.entries {
            let Some(&confirmed) = stft.get(entry.band) else { continue };
            entry.hold = (entry.hold * self.hold_decay).max(confirmed);

            if let Some(v) = levels.get_mut(entry.band) {
                let fast = entry.envelope.value() * PEAK_TO_RMS;
                *v = v.max(fast.min(entry.hold));
            }
        }
    }

    pub fn reset(&mut self) {
        for entry in &mut self.entries {
            for stage in &mut entry.stages {
                stage.reset();
            }
            entry.envelope.reset();
            entry.hold = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::bands::{BandPlan, BandScaleConfig};
    use crate::dsp::window::WindowKind;

    const SR: f64 = 48_000.0;

    fn plan() -> BandPlan {
        BandPlan::new(&BandScaleConfig::default(), SR, WindowKind::BlackmanHarris)
    }

    fn tone(freq: f64, secs: f64) -> Vec<f32> {
        (0..(SR * secs) as usize)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / SR).sin() as f32)
            .collect()
    }

    #[test]
    fn covers_slow_tiers_only() {
        let p = plan();
        let fp = FastPath::new(&p, &FastPathConfig::default());
        assert!(!fp.is_empty(), "no band got a fast path");
        assert!(fp.len() < p.len(), "every band got one; treble does not need it");
    }

    /// The point of the module: reach most of the way to full level far sooner
    /// than the 171 ms window of the slowest tier could. The gate is held open
    /// here to isolate the filter's own speed.
    #[test]
    fn bass_envelope_responds_within_milliseconds() {
        let p = plan();
        let mut fp = FastPath::new(&p, &FastPathConfig::default());
        fp.push(&tone(p.bands[0].center, 0.020));

        let open = vec![1.0f32; p.len()];
        let mut levels = vec![0.0f32; p.len()];
        fp.apply(&open, &mut levels);
        assert!(levels[0] > 0.25, "after 20 ms band 0 only reached {}", levels[0]);
    }

    /// The artefact this module could have introduced. A steady mid tone must
    /// not light a bass bar, even though the bandpass alone passes it at
    /// roughly -45 dB — the gate is what makes this hold.
    #[test]
    fn steady_mid_tone_cannot_light_a_bass_bar() {
        let p = plan();
        let mut fp = FastPath::new(&p, &FastPathConfig::default());
        fp.push(&tone(300.0, 1.0));

        // The STFT correctly reports nothing in the bass bands for a 300 Hz tone.
        let stft = vec![0.0f32; p.len()];
        let mut levels = vec![0.0f32; p.len()];
        for _ in 0..200 {
            fp.apply(&stft, &mut levels);
        }
        assert!(
            levels.iter().all(|&v| v < 1e-6),
            "300 Hz tone lit a bass bar at {}",
            levels.iter().cloned().fold(0.0f32, f32::max)
        );
    }

    /// The gate must not be so tight that it defeats the purpose: once the STFT
    /// has confirmed a band, the fast path should be free to run ahead of it.
    #[test]
    fn gate_opens_once_the_stft_confirms_the_band() {
        let p = plan();
        let mut fp = FastPath::new(&p, &FastPathConfig::default());
        fp.push(&tone(p.bands[0].center, 0.5));

        let mut stft = vec![0.0f32; p.len()];
        stft[0] = 0.5;
        let mut levels = vec![0.0f32; p.len()];
        fp.apply(&stft, &mut levels);
        assert!(levels[0] > 0.1, "gate stayed shut on a confirmed band: {}", levels[0]);
    }

    /// `apply` must never lower an existing level — the STFT stays authoritative
    /// wherever it already reports more.
    #[test]
    fn never_reduces_an_existing_level() {
        let p = plan();
        let mut fp = FastPath::new(&p, &FastPathConfig::default());
        fp.push(&vec![0.0; 4096]);

        let stft = vec![1.0f32; p.len()];
        let mut levels = vec![0.9f32; p.len()];
        fp.apply(&stft, &mut levels);
        assert!(levels.iter().all(|&v| v >= 0.9), "fast path pulled a level down");
    }

    #[test]
    fn silence_yields_nothing() {
        let p = plan();
        let mut fp = FastPath::new(&p, &FastPathConfig::default());
        fp.push(&vec![0.0; 48_000]);

        let stft = vec![1.0f32; p.len()];
        let mut levels = vec![0.0f32; p.len()];
        fp.apply(&stft, &mut levels);
        assert!(levels.iter().all(|&v| v < 1e-6), "fast path invented signal");
    }
}
