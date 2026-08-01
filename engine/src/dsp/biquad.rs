//! Bandpass biquad and envelope follower, used for the fast transient path.
//!
//! The bass tier of the STFT uses an 8192-point window (171 ms) because that is
//! what it takes to resolve a 40 Hz band cleanly. That is accurate but sluggish:
//! a kick drum smears. The fix is not to shorten the window — the uncertainty
//! principle forbids having both — but to run a *second*, low-latency estimate
//! alongside it and take the maximum:
//!
//!   band_level = max(slow_clean_fft, fast_biquad_envelope)
//!
//! The FFT supplies an accurate, ghost-free steady-state level; the biquad
//! supplies the attack. Perceived bass latency drops from ~112 ms to ~25 ms
//! while the artefact rejection of the long window is fully retained.

/// Transposed Direct Form II biquad. TDF-II is used rather than DF-I because it
/// has better numerical behaviour at the low centre frequencies this is used at,
/// where coefficients cluster near the unit circle.
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// Constant 0 dB peak-gain bandpass (RBJ cookbook).
    ///
    /// `q` trades selectivity against ring time: settling is roughly `q / (π·f0)`
    /// seconds, so at 60 Hz a Q of 4 rings for ~21 ms. Keep Q modest here — this
    /// path exists for speed, and the STFT is already handling accuracy.
    pub fn bandpass(f0: f32, q: f32, sample_rate: f32) -> Self {
        let w0 = std::f32::consts::TAU * f0 / sample_rate;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha;
        Self {
            b0: alpha / a0,
            b1: 0.0,
            b2: -alpha / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Asymmetric one-pole envelope follower on `|x|`.
///
/// Fast attack catches transients; slow release makes the result readable as
/// light rather than as flicker.
#[derive(Clone, Copy, Debug)]
pub struct EnvelopeFollower {
    attack: f32,
    release: f32,
    env: f32,
}

impl EnvelopeFollower {
    /// Time constants in seconds. Coefficients use `1 - exp(-dt/τ)`, the exact
    /// one-pole step response, rather than the `dt/τ` approximation — at these
    /// hop sizes τ is often comparable to `dt` and the approximation would
    /// overshoot past 1.0 and go unstable.
    pub fn new(attack_secs: f32, release_secs: f32, dt: f32) -> Self {
        Self {
            attack: coefficient(attack_secs, dt),
            release: coefficient(release_secs, dt),
            env: 0.0,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let target = x.abs();
        let c = if target > self.env { self.attack } else { self.release };
        self.env += c * (target - self.env);
        self.env
    }

    #[inline]
    pub fn value(&self) -> f32 {
        self.env
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
    }
}

/// One-pole smoothing coefficient for time constant `tau` at timestep `dt`.
/// Clamped to 1.0 so a τ shorter than one step means "jump immediately".
#[inline]
pub fn coefficient(tau_secs: f32, dt: f32) -> f32 {
    if tau_secs <= 0.0 {
        return 1.0;
    }
    (1.0 - (-dt / tau_secs).exp()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn tone_response(filter: &mut Biquad, freq: f32) -> f32 {
        let mut peak: f32 = 0.0;
        // Long enough for the filter to settle before measuring.
        for n in 0..96_000 {
            let x = (std::f32::consts::TAU * freq * n as f32 / SR).sin();
            let y = filter.process(x);
            if n > 48_000 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn bandpass_passes_centre_and_rejects_far_bands() {
        let mut bp = Biquad::bandpass(60.0, 4.0, SR);
        let at_centre = tone_response(&mut bp, 60.0);
        assert!((at_centre - 1.0).abs() < 0.05, "centre gain {at_centre}, expected ~1.0");

        for (freq, max_gain) in [(1000.0, 0.05), (5000.0, 0.02)] {
            let mut bp = Biquad::bandpass(60.0, 4.0, SR);
            let g = tone_response(&mut bp, freq);
            assert!(g < max_gain, "{freq} Hz leaked through at {g}");
        }
    }

    #[test]
    fn envelope_attacks_fast_and_releases_slow() {
        let dt = 1.0 / SR;
        let mut env = EnvelopeFollower::new(0.001, 0.150, dt);

        // 5 ms of full-scale signal should get the envelope most of the way up.
        for _ in 0..(SR * 0.005) as usize {
            env.process(1.0);
        }
        assert!(env.value() > 0.9, "attack too slow: {}", env.value());

        // After the same 5 ms of silence it should still be holding high.
        for _ in 0..(SR * 0.005) as usize {
            env.process(0.0);
        }
        assert!(env.value() > 0.9, "release too fast: {}", env.value());
    }

    #[test]
    fn coefficient_is_stable_when_tau_is_shorter_than_timestep() {
        // hop-rate ballistics routinely ask for τ < dt; must clamp, not explode.
        let c = coefficient(0.001, 0.00533);
        assert!((0.0..=1.0).contains(&c), "coefficient {c} out of range");
    }
}
