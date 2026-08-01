//! Band layout and FFT tier assignment.
//!
//! # Why the band scale is not pure logarithmic
//!
//! The obvious choice for an EQ display is log spacing, and it is wrong at the
//! bottom. 60 log bands from 30 Hz to 16 kHz are each 11% wide, so the lowest
//! band spans just 3.3 Hz — which needs a bin width under 1.65 Hz, an FFT of
//! ~29000 points, a 600 ms window. Unbuildable, and pointless: it demands
//! frequency resolution the ear does not possess. Human critical bandwidth down
//! there is ~100 Hz.
//!
//! So the scale is Bark-like: fixed-width bands below `linear_below`, log-spaced
//! above. This is both physically achievable and perceptually correct.
//!
//! # Why each band gets its own FFT size
//!
//! Time-frequency uncertainty (Δf·Δt ≳ 1) means one FFT size cannot serve the
//! whole spectrum. A window long enough to resolve 40 Hz is far longer than
//! treble needs, and paying that latency at 10 kHz is pure waste. Each band
//! therefore draws from the *smallest* (fastest) FFT that can still resolve it:
//!
//!   a band may use size N only if
//!     1. its width      >= 2 * (sr / N)          — resolvable at all
//!     2. its lower edge >= discard * (sr / N)    — clear of the main lobe at DC
//!
//! Condition 2 is what stops the lowest bar from reading the window's own DC
//! smear. See [`super::window`] for the measurements behind `discard`.
//!
//! The assignment is solved at construction from the real sample rate, never
//! hardcoded, so it stays correct at 44.1 kHz, 96 kHz or anything else.

use super::window::WindowKind;

/// FFT sizes the analyser may choose from, ascending.
///
/// The top of the ladder exists for high sample rates, not for 48 kHz. Bin width
/// is `sr/N`, so a 96 kHz device needs twice the FFT size to reach the same
/// resolution in Hz; without 16384 and 32768 present, the lowest bands become
/// unresolvable above 48 kHz. Sizes never selected cost nothing — [`BandPlan`]
/// only plans the ones actually referenced.
pub const DEFAULT_FFT_SIZES: &[usize] = &[128, 512, 2048, 4096, 8192, 16_384, 32_768];

#[derive(Clone, Debug)]
pub struct BandScaleConfig {
    /// Bottom of the display. 40 Hz rather than 20 Hz: below this, wall-mounted
    /// strips are showing content most rooms and speakers cannot reproduce, and
    /// the window cost to resolve it is steep.
    pub f_min: f64,
    pub f_max: f64,
    /// Width of the fixed-width bands at the bottom, near human critical
    /// bandwidth at low frequency. The crossover to log spacing is *derived*
    /// from this rather than configured — see [`band_edges`].
    pub linear_width: f64,
    pub band_count: usize,
}

impl Default for BandScaleConfig {
    fn default() -> Self {
        Self {
            f_min: 40.0,
            f_max: 16_000.0,
            linear_width: 27.0,
            band_count: 48,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub lo: f64,
    pub hi: f64,
    /// Geometric centre — the right average on a log axis.
    pub center: f64,
    /// FFT size serving this band.
    pub fft_size: usize,
    /// True when no available FFT size satisfies both conditions. The band still
    /// renders (using the largest size) but its level is not trustworthy; the
    /// constructor surfaces this so a bad config is visible rather than silent.
    pub unresolved: bool,
}

impl Band {
    pub fn width(&self) -> f64 {
        self.hi - self.lo
    }
}

#[derive(Clone, Debug)]
pub struct BandPlan {
    pub sample_rate: f64,
    pub bands: Vec<Band>,
    /// Distinct FFT sizes actually referenced, ascending. The analyser plans and
    /// runs only these, so an unused tier costs nothing.
    pub fft_sizes: Vec<usize>,
}

impl BandPlan {
    pub fn new(cfg: &BandScaleConfig, sample_rate: f64, window: WindowKind) -> Self {
        Self::with_sizes(cfg, sample_rate, window, DEFAULT_FFT_SIZES)
    }

    pub fn with_sizes(
        cfg: &BandScaleConfig,
        sample_rate: f64,
        window: WindowKind,
        sizes: &[usize],
    ) -> Self {
        let edges = band_edges(cfg);
        let discard = window.discard_bins() as f64;

        let mut sorted: Vec<usize> = sizes.to_vec();
        sorted.sort_unstable();
        let largest = *sorted.last().expect("at least one FFT size required");

        let bands: Vec<Band> = edges
            .windows(2)
            .map(|w| {
                let (lo, hi) = (w[0], w[1]);
                let width = hi - lo;

                // Smallest size satisfying both conditions: fastest tier that
                // can still do the job.
                let chosen = sorted.iter().copied().find(|&n| {
                    let bin = sample_rate / n as f64;
                    width >= 2.0 * bin && lo >= discard * bin
                });

                Band {
                    lo,
                    hi,
                    center: (lo * hi).sqrt(),
                    fft_size: chosen.unwrap_or(largest),
                    unresolved: chosen.is_none(),
                }
            })
            .collect();

        let mut fft_sizes: Vec<usize> = bands.iter().map(|b| b.fft_size).collect();
        fft_sizes.sort_unstable();
        fft_sizes.dedup();

        Self { sample_rate, bands, fft_sizes }
    }

    pub fn len(&self) -> usize {
        self.bands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bands.is_empty()
    }

    /// Bands the tier rule could not satisfy. Empty for sane configurations;
    /// non-empty means `f_min` is too low or the FFT ladder too short.
    pub fn unresolved(&self) -> impl Iterator<Item = (usize, &Band)> {
        self.bands.iter().enumerate().filter(|(_, b)| b.unresolved)
    }

    /// Human-readable tier ladder, for startup logging. Collapses the per-band
    /// assignment into contiguous frequency spans per FFT size.
    pub fn describe_tiers(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut iter = self.bands.iter().peekable();
        while let Some(first) = iter.next() {
            let size = first.fft_size;
            let lo = first.lo;
            let mut hi = first.hi;
            let mut count = 1;
            while iter.peek().is_some_and(|b| b.fft_size == size) {
                hi = iter.next().unwrap().hi;
                count += 1;
            }
            let span_ms = size as f64 / self.sample_rate * 1000.0;
            let bin = self.sample_rate / size as f64;
            out.push(format!(
                "N={size:<5} {span_ms:6.1} ms  bin {bin:7.2} Hz  {lo:7.1} - {hi:8.1} Hz  ({count} bands)"
            ));
        }
        out
    }
}

/// Band edges, returning `band_count + 1` values.
///
/// Each band's width is `max(linear_width, rel · f)`: constant at the bottom,
/// constant-relative (log) above, with the crossover falling out at
/// `linear_width / rel`. The relative width `rel` is solved by bisection so the
/// last edge lands exactly on `f_max`.
///
/// Defining the two regions this way rather than splicing them at a configured
/// frequency matters: a hand-placed join almost always leaves widths
/// *decreasing* across it (log bands just above the join are narrower than the
/// fixed bands just below). Non-monotonic widths break the tier ladder, because
/// a narrower band needs a *larger* FFT — so the assignment jumps back up and
/// bass bars end up on inconsistent windows. Here widths are non-decreasing by
/// construction, so the ladder is monotonic for free.
pub fn band_edges(cfg: &BandScaleConfig) -> Vec<f64> {
    let walk = |rel: f64| {
        let mut f = cfg.f_min;
        for _ in 0..cfg.band_count {
            f += cfg.linear_width.max(rel * f);
        }
        f
    };

    // `walk` is monotonically increasing in `rel`, so bisection is exact enough.
    let (mut lo, mut hi) = (0.0, 1.0);
    while walk(hi) < cfg.f_max && hi < 1e6 {
        hi *= 2.0;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if walk(mid) < cfg.f_max {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let rel = 0.5 * (lo + hi);

    let mut edges = Vec::with_capacity(cfg.band_count + 1);
    let mut f = cfg.f_min;
    edges.push(f);
    for _ in 0..cfg.band_count {
        f += cfg.linear_width.max(rel * f);
        edges.push(f);
    }
    // Absorb bisection residue so the top edge is exactly f_max.
    if let Some(last) = edges.last_mut() {
        *last = cfg.f_max;
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48_000.0;

    fn plan() -> BandPlan {
        BandPlan::new(&BandScaleConfig::default(), SR, WindowKind::BlackmanHarris)
    }

    #[test]
    fn edges_are_monotonic_and_span_the_configured_range() {
        let cfg = BandScaleConfig::default();
        let e = band_edges(&cfg);
        assert_eq!(e.len(), cfg.band_count + 1);
        assert!(e.windows(2).all(|w| w[1] > w[0]), "edges not strictly increasing");
        assert!((e[0] - cfg.f_min).abs() < 1e-9);
        assert!((e[e.len() - 1] - cfg.f_max).abs() < 1e-6, "top edge {}", e[e.len() - 1]);
    }

    /// The core invariant. Every band must satisfy both halves of the tier rule,
    /// or its level is contaminated by DC smear or unresolvable in principle.
    #[test]
    fn every_band_satisfies_the_tier_rule() {
        let p = plan();
        let discard = WindowKind::BlackmanHarris.discard_bins() as f64;
        for (i, b) in p.bands.iter().enumerate() {
            assert!(!b.unresolved, "band {i} ({:.1}-{:.1} Hz) unresolved", b.lo, b.hi);
            let bin = SR / b.fft_size as f64;
            assert!(
                b.width() >= 2.0 * bin,
                "band {i} ({:.1}-{:.1} Hz) width {:.2} < 2 bins ({:.2}) at N={}",
                b.lo, b.hi, b.width(), 2.0 * bin, b.fft_size
            );
            assert!(
                b.lo >= discard * bin,
                "band {i} lower edge {:.1} inside the discarded {discard} bins ({:.1} Hz) at N={}",
                b.lo, discard * bin, b.fft_size
            );
        }
    }

    /// Each band should use the *smallest* qualifying size — picking a larger one
    /// would mean paying latency for nothing.
    #[test]
    fn assignment_is_minimal() {
        let p = plan();
        let discard = WindowKind::BlackmanHarris.discard_bins() as f64;
        for b in &p.bands {
            for &smaller in DEFAULT_FFT_SIZES.iter().filter(|&&n| n < b.fft_size) {
                let bin = SR / smaller as f64;
                assert!(
                    !(b.width() >= 2.0 * bin && b.lo >= discard * bin),
                    "band {:.1}-{:.1} Hz used N={} but N={smaller} also qualified",
                    b.lo, b.hi, b.fft_size
                );
            }
        }
    }

    /// Widths must never decrease with frequency. This is the property that
    /// makes the tier ladder monotonic; a kink at the linear/log join would
    /// silently push mid bands back onto larger FFTs.
    #[test]
    fn band_widths_are_non_decreasing() {
        let p = plan();
        for w in p.bands.windows(2) {
            assert!(
                w[1].width() >= w[0].width() - 1e-9,
                "width shrank from {:.2} Hz at {:.1} Hz to {:.2} Hz at {:.1} Hz",
                w[0].width(), w[0].lo, w[1].width(), w[1].lo
            );
        }
    }

    /// The bottom of the scale must actually be fixed-width (critical-band-like),
    /// not log — that is the whole reason the scale is not pure log.
    #[test]
    fn low_bands_are_fixed_width() {
        let cfg = BandScaleConfig::default();
        let p = BandPlan::new(&cfg, SR, WindowKind::BlackmanHarris);
        assert!(
            (p.bands[0].width() - cfg.linear_width).abs() < 1e-6,
            "lowest band is {:.2} Hz wide, expected the linear width {:.2}",
            p.bands[0].width(), cfg.linear_width
        );
    }

    /// Low bands must get long windows and high bands short ones — the whole
    /// point of the ladder. Assignment must be monotonically non-increasing.
    #[test]
    fn tier_sizes_decrease_with_frequency() {
        let p = plan();
        assert!(
            p.bands.windows(2).all(|w| w[0].fft_size >= w[1].fft_size),
            "tier ladder not monotonic: {:?}",
            p.bands.iter().map(|b| b.fft_size).collect::<Vec<_>>()
        );
    }

    /// A non-48 kHz device must still produce a valid plan; nothing is hardcoded.
    #[test]
    fn rule_holds_at_other_sample_rates() {
        for sr in [44_100.0, 96_000.0, 192_000.0] {
            let p = BandPlan::new(&BandScaleConfig::default(), sr, WindowKind::BlackmanHarris);
            assert!(
                p.unresolved().next().is_none(),
                "unresolved bands at {sr} Hz: {:?}",
                p.unresolved().map(|(i, _)| i).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn only_referenced_fft_sizes_are_planned() {
        let p = plan();
        for &n in &p.fft_sizes {
            assert!(p.bands.iter().any(|b| b.fft_size == n), "N={n} planned but unused");
        }
    }
}
