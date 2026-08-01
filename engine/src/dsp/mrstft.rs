//! Multi-resolution short-time Fourier transform.
//!
//! One shared sample ring feeds several FFT sizes running in parallel. Every
//! `hop` samples each tier transforms the *most recent* N samples it needs, and
//! each band reads from whichever tier [`BandPlan`] assigned it. Bass gets a long
//! accurate window, treble gets a short fast one, and neither pays the other's
//! cost.
//!
//! # Normalisation
//!
//! Band levels are computed by summing *power* across bins, so the scaling must
//! be energy-preserving, not amplitude-preserving. By Parseval, for a windowed
//! real signal:
//!
//! ```text
//!   Σ_k c_k |X[k]|²  =  N · Σ_n |x[n]·w[n]|²        c_k = 1 at DC/Nyquist, else 2
//! ```
//!
//! so dividing by `N² · noise_gain` (where `noise_gain = mean(w²)`) makes the sum
//! over a tone's main lobe come out to `A²/2` — its true mean-square power —
//! independent of window, FFT size, and where in the band the tone sits. A
//! full-scale sine therefore reads its RMS, ~-3 dB.
//!
//! Note this uses the window's *noise* gain, not the coherent gain used for
//! single-bin amplitude readings. Using coherent gain here would misreport every
//! band by the window's crest factor.
//!
//! # Why summed power rather than power density
//!
//! Summing (not averaging) means a pure tone reads the same level regardless of
//! how wide the band it lands in is. It also makes the display naturally flat for
//! pink noise — energy per log band is constant when PSD ∝ 1/f — and music is
//! approximately pink. Averaging would instead impose a downward slope that the
//! spectral tilt would then have to undo.

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

use super::bands::BandPlan;
use super::window::WindowKind;

/// Analysis hop in samples. 256 at 48 kHz is 5.33 ms — ~187 frames/second, well
/// above the ~70 fps the LED link can carry, which keeps the ballistics in
/// [`super::post`] smooth rather than stepped.
pub const DEFAULT_HOP: usize = 256;

struct Tier {
    size: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    /// `|X[k]|²` already normalised and doubled for the one-sided spectrum.
    power: Vec<f32>,
    norm: f32,
}

impl Tier {
    fn new(planner: &mut RealFftPlanner<f32>, size: usize, kind: WindowKind) -> Self {
        let fft = planner.plan_fft_forward(size);
        let window = kind.generate(size);

        // Measured rather than tabulated, so it stays exact for any window.
        let noise_gain =
            window.iter().map(|&w| (w as f64) * (w as f64)).sum::<f64>() / size as f64;
        let norm = 1.0 / (size as f64 * size as f64 * noise_gain);

        Self {
            size,
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            power: vec![0.0; size / 2 + 1],
            fft,
            window,
            norm: norm as f32,
        }
    }

    /// Transform whatever is currently in `input`, which must already be
    /// windowed, into normalised one-sided power.
    fn transform(&mut self) {
        self.fft
            .process(&mut self.input, &mut self.spectrum)
            .expect("FFT buffer sizes are fixed at construction");

        let last = self.power.len() - 1;
        for (k, c) in self.spectrum.iter().enumerate() {
            // Fold the negative-frequency half back in, except at DC/Nyquist
            // which have no mirror partner.
            let c_k = if k == 0 || k == last { 1.0 } else { 2.0 };
            self.power[k] = c_k * c.norm_sqr() * self.norm;
        }
    }

    /// Transform the newest `size` samples sitting in `ring`.
    fn run(&mut self, ring: &[f32], write: usize) {
        let cap = ring.len();
        let start = (write + cap - self.size) % cap;

        // Copy out of the circular buffer, applying the window in the same pass.
        let first = (cap - start).min(self.size);
        for i in 0..first {
            self.input[i] = ring[start + i] * self.window[i];
        }
        for i in first..self.size {
            self.input[i] = ring[i - first] * self.window[i];
        }

        self.transform();
    }

    /// Load a windowed unit-amplitude sine, used only for calibration.
    fn load_tone(&mut self, freq: f64, sample_rate: f64) {
        for i in 0..self.size {
            let s = (std::f64::consts::TAU * freq * i as f64 / sample_rate).sin() as f32;
            self.input[i] = s * self.window[i];
        }
    }
}

/// Precomputed bin span and weights for one band.
struct BandBins {
    tier: usize,
    start: usize,
    weights: Vec<f32>,
}

/// Upper bound on the per-band calibration gain, +12 dB. A band far narrower
/// than its tier's main lobe would otherwise demand a large correction, and
/// scaling up a band that captures little signal also scales up its noise.
const MAX_CALIBRATION_GAIN: f32 = 4.0;

pub struct MultiResStft {
    plan: BandPlan,
    hop: usize,
    tiers: Vec<Tier>,
    band_bins: Vec<BandBins>,
    /// Per-band power correction, measured at construction. See [`Self::calibrate`].
    calibration: Vec<f32>,
    ring: Vec<f32>,
    write: usize,
    /// Samples accumulated toward the next hop.
    pending: usize,
    /// Total samples ever written, used to suppress startup output.
    written: usize,
    warmup: usize,
    magnitudes: Vec<f32>,
}

impl MultiResStft {
    pub fn new(plan: BandPlan, kind: WindowKind) -> Self {
        Self::with_hop(plan, kind, DEFAULT_HOP)
    }

    pub fn with_hop(plan: BandPlan, kind: WindowKind, hop: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let mut tiers: Vec<Tier> = plan
            .fft_sizes
            .iter()
            .map(|&n| Tier::new(&mut planner, n, kind))
            .collect();

        let max_size = plan.fft_sizes.iter().copied().max().unwrap_or(1);
        let cap = max_size.next_power_of_two();
        let discard = kind.discard_bins();
        let sr = plan.sample_rate;

        let band_bins: Vec<BandBins> = plan
            .bands
            .iter()
            .map(|band| {
                let tier = tiers
                    .iter()
                    .position(|t| t.size == band.fft_size)
                    .expect("BandPlan only references planned FFT sizes");
                let bin_w = sr / band.fft_size as f64;
                let n_bins = band.fft_size / 2 + 1;

                // Bin k is treated as covering [(k-0.5)·bw, (k+0.5)·bw]. Partial
                // overlap at the edges is weighted by the fraction covered, so a
                // band's level does not step as its edges move between bins.
                let lo_bin = ((band.lo / bin_w - 0.5).floor() as isize).max(discard as isize) as usize;
                let hi_bin = (((band.hi / bin_w + 0.5).ceil()) as usize).min(n_bins - 1);

                let mut weights = Vec::new();
                for k in lo_bin..=hi_bin.max(lo_bin) {
                    let bin_lo = (k as f64 - 0.5) * bin_w;
                    let bin_hi = (k as f64 + 0.5) * bin_w;
                    let overlap = band.hi.min(bin_hi) - band.lo.max(bin_lo);
                    weights.push(if overlap > 0.0 { (overlap / bin_w) as f32 } else { 0.0 });
                }

                // A band narrower than one bin still needs to read something;
                // fall back to the single nearest usable bin.
                if weights.iter().all(|&w| w <= 0.0) {
                    let k = ((band.center / bin_w).round() as usize).clamp(discard, n_bins - 1);
                    return BandBins { tier, start: k, weights: vec![1.0] };
                }

                BandBins { tier, start: lo_bin, weights }
            })
            .collect();

        let band_count = plan.len();
        let calibration = Self::calibrate(&mut tiers, &band_bins, &plan);

        Self {
            plan,
            hop,
            tiers,
            band_bins,
            calibration,
            ring: vec![0.0; cap],
            write: 0,
            pending: 0,
            written: 0,
            // Hold output until the longest window is full of real audio.
            // Otherwise its first frames straddle the zero-filled ring and the
            // step edge splatters broadband energy across every bar.
            warmup: max_size,
            magnitudes: vec![0.0; band_count],
        }
    }

    /// Measure and invert each band's energy-capture fraction.
    ///
    /// The tier rule guarantees a band is at least 2 bins wide, which is enough
    /// to *resolve* a tone but not always enough to *contain* one: the
    /// Blackman-Harris main lobe is ~8 bins across, so a band only a few bins
    /// wide loses the skirts to its neighbours and reads low. Measured at 5 kHz
    /// on the N=512 tier that shortfall is 1.7 dB.
    ///
    /// A constant per-band error would be harmless on its own — AGC would absorb
    /// it — but it is *not* constant: it jumps at every tier boundary, so a
    /// swept tone would step in brightness as it crossed. Here each band is
    /// calibrated by pushing a unit sine at its centre frequency through the
    /// exact code path used at runtime and comparing against the known answer
    /// (mean-square power of a unit sine is 0.5). Calibrating against the real
    /// path rather than an analytic model means the correction cannot drift out
    /// of step if the windowing or weighting changes.
    fn calibrate(tiers: &mut [Tier], band_bins: &[BandBins], plan: &BandPlan) -> Vec<f32> {
        band_bins
            .iter()
            .zip(&plan.bands)
            .map(|(bins, band)| {
                let tier = &mut tiers[bins.tier];
                tier.load_tone(band.center, plan.sample_rate);
                tier.transform();

                let captured: f32 = bins
                    .weights
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &w)| tier.power.get(bins.start + i).map(|&p| w * p))
                    .sum();

                if captured > 1e-12 {
                    (0.5 / captured).clamp(1.0, MAX_CALIBRATION_GAIN)
                } else {
                    1.0
                }
            })
            .collect()
    }

    pub fn plan(&self) -> &BandPlan {
        &self.plan
    }

    pub fn band_count(&self) -> usize {
        self.magnitudes.len()
    }

    pub fn hop(&self) -> usize {
        self.hop
    }

    /// RMS magnitude per band from the most recent analysis frame.
    pub fn magnitudes(&self) -> &[f32] {
        &self.magnitudes
    }

    /// Feed samples. Returns the number of analysis frames produced; callers
    /// that only care whether the output changed can test for non-zero.
    ///
    /// Blocks larger than one hop produce several frames, and only the last is
    /// retained — intermediate frames are superseded before anything could read
    /// them, and dropping them keeps latency bounded when a device delivers a
    /// burst after a stall.
    pub fn push(&mut self, samples: &[f32]) -> usize {
        let cap = self.ring.len();
        let mut frames = 0;

        for &s in samples {
            self.ring[self.write] = s;
            self.write = (self.write + 1) % cap;
            self.pending += 1;
            self.written = self.written.saturating_add(1);

            if self.pending >= self.hop {
                self.pending = 0;
                if self.written >= self.warmup {
                    self.analyze();
                    frames += 1;
                }
            }
        }
        frames
    }

    fn analyze(&mut self) {
        for tier in &mut self.tiers {
            tier.run(&self.ring, self.write);
        }

        for ((mag, bins), &cal) in self
            .magnitudes
            .iter_mut()
            .zip(&self.band_bins)
            .zip(&self.calibration)
        {
            let power = &self.tiers[bins.tier].power;
            let mut sum = 0.0f32;
            for (i, &w) in bins.weights.iter().enumerate() {
                if let Some(&p) = power.get(bins.start + i) {
                    sum += w * p;
                }
            }
            *mag = (sum * cal).max(0.0).sqrt();
        }
    }

    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.write = 0;
        self.pending = 0;
        self.written = 0;
        self.magnitudes.fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::bands::{BandPlan, BandScaleConfig};

    const SR: f64 = 48_000.0;

    fn analyzer() -> MultiResStft {
        let plan = BandPlan::new(&BandScaleConfig::default(), SR, WindowKind::BlackmanHarris);
        MultiResStft::new(plan, WindowKind::BlackmanHarris)
    }

    fn feed_tone(a: &mut MultiResStft, freq: f64, amp: f32, secs: f64) {
        let n = (SR * secs) as usize;
        let buf: Vec<f32> = (0..n)
            .map(|i| amp * (std::f64::consts::TAU * freq * i as f64 / SR).sin() as f32)
            .collect();
        a.push(&buf);
    }

    fn hot_band(a: &MultiResStft) -> usize {
        a.magnitudes()
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.partial_cmp(y.1).unwrap())
            .map(|(i, _)| i)
            .unwrap()
    }

    /// Amplitude calibration: a full-scale sine must read its RMS (~0.707),
    /// regardless of which tier serves it. This is what makes the dB range in
    /// [`super::post`] mean something absolute.
    #[test]
    fn full_scale_sine_reads_rms() {
        for freq in [60.0, 440.0, 1000.0, 5000.0] {
            let mut a = analyzer();
            feed_tone(&mut a, freq, 1.0, 1.0);
            let peak = a.magnitudes().iter().cloned().fold(0.0f32, f32::max);
            assert!(
                (peak - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.08,
                "{freq} Hz read {peak}, expected ~0.707 RMS"
            );
        }
    }

    /// Energy must not depend on where inside a band the tone sits, otherwise
    /// bars flicker as a note drifts. Checks a tone at a band centre against one
    /// deliberately placed at a band edge.
    #[test]
    fn level_is_stable_across_band_position() {
        let plan = BandPlan::new(&BandScaleConfig::default(), SR, WindowKind::BlackmanHarris);
        let band = &plan.bands[30];
        let mut peaks = Vec::new();
        for freq in [band.lo * 1.02, band.center, band.hi * 0.98] {
            let mut a = analyzer();
            feed_tone(&mut a, freq, 1.0, 1.0);
            peaks.push(a.magnitudes().iter().cloned().fold(0.0f32, f32::max));
        }
        let (min, max) = (
            peaks.iter().cloned().fold(f32::MAX, f32::min),
            peaks.iter().cloned().fold(0.0f32, f32::max),
        );
        assert!(max / min < 1.35, "level varied {peaks:?} across one band");
    }

    /// A tone must light the band that actually contains it.
    #[test]
    fn tone_lands_in_the_right_band() {
        for freq in [55.0, 300.0, 2000.0, 9000.0] {
            let mut a = analyzer();
            feed_tone(&mut a, freq, 1.0, 1.0);
            let b = &a.plan().bands[hot_band(&a)];
            assert!(
                freq >= b.lo * 0.9 && freq <= b.hi * 1.1,
                "{freq} Hz lit band {:.1}-{:.1} Hz",
                b.lo, b.hi
            );
        }
    }

    /// Silence in, silence out — no self-noise from the analyser itself.
    #[test]
    fn silence_produces_no_output() {
        let mut a = analyzer();
        a.push(&vec![0.0; 48_000]);
        assert!(
            a.magnitudes().iter().all(|&m| m < 1e-9),
            "analyser generated energy from silence"
        );
    }

    /// The warm-up guard must suppress the startup step edge, which would
    /// otherwise flash every bar at once when the app launches.
    #[test]
    fn no_output_before_the_longest_window_is_full() {
        let mut a = analyzer();
        let max_size = *a.plan().fft_sizes.iter().max().unwrap();
        let buf: Vec<f32> = (0..max_size / 2)
            .map(|i| (std::f64::consts::TAU * 1000.0 * i as f64 / SR).sin() as f32)
            .collect();
        assert_eq!(a.push(&buf), 0, "emitted a frame before warm-up completed");
    }
}
