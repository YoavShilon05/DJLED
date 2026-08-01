//! Analysis windows.
//!
//! Window choice was settled by measurement rather than by reputation. Against a
//! 1 kHz tone at N=1024/48 kHz, leakage into the sub-bass bar (relative to the
//! 1 kHz bar, where roughly -70 dB is the threshold at which an LED visibly lights):
//!
//! | window      | far-field leak | neighbour bar (+110 Hz) | bin 1 after DC block |
//! |-------------|----------------|-------------------------|----------------------|
//! | rectangular | -44 dB         | --                      | -31 dB               |
//! | Hann        | numerical floor | -70 dB                 | -74 dB               |
//! | Blackman-Harris | numerical floor | -98 dB             | -70 dB               |
//!
//! Two conclusions drive the defaults here. Blackman-Harris wins decisively on
//! *neighbour* separation (-98 vs -70 dB), which is what keeps adjacent EQ bars
//! from bleeding into each other. But it is slightly *worse* in bin 1, because its
//! main lobe is ~8 bins wide against Hann's ~4, so residual sub-audio energy
//! re-contaminates the lowest bins. Hence `discard_bins`: the bottom of every tier
//! is thrown away, and the amount depends on the window's main-lobe width.

/// Window functions available to the analyser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WindowKind {
    /// Narrower main lobe, -31 dB sidelobes. Cheaper on low-bin discard, worse
    /// at keeping neighbouring bars separate.
    Hann,
    /// 4-term Blackman-Harris, -92 dB sidelobes. The default: bar-to-bar
    /// separation matters more here than main-lobe width, because the tier
    /// ladder already buys back the lost resolution by using a larger FFT.
    #[default]
    BlackmanHarris,
}

impl WindowKind {
    /// Mean value of the window. A sinusoid of amplitude `A` at bin `k` produces
    /// `|X[k]| = A * n * coherent_gain / 2`, so dividing by this recovers true
    /// amplitude regardless of window or FFT size.
    pub fn coherent_gain(self) -> f32 {
        match self {
            WindowKind::Hann => 0.5,
            WindowKind::BlackmanHarris => 0.358_75,
        }
    }

    /// Number of bins at the bottom of the spectrum rendered unusable by the
    /// window's main lobe smearing residual DC and sub-audio energy upward.
    /// Roughly half the main-lobe width in bins.
    pub fn discard_bins(self) -> usize {
        match self {
            WindowKind::Hann => 2,
            WindowKind::BlackmanHarris => 4,
        }
    }

    /// Generate the periodic (not symmetric) form, which is the correct choice
    /// for spectral analysis — the symmetric form biases the estimate slightly.
    pub fn generate(self, n: usize) -> Vec<f32> {
        let nf = n as f64;
        (0..n)
            .map(|i| {
                let t = std::f64::consts::TAU * i as f64 / nf;
                let v = match self {
                    WindowKind::Hann => 0.5 - 0.5 * t.cos(),
                    WindowKind::BlackmanHarris => {
                        0.358_75 - 0.488_29 * t.cos() + 0.141_28 * (2.0 * t).cos()
                            - 0.011_68 * (3.0 * t).cos()
                    }
                };
                v as f32
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The documented coherent gain must match the window actually generated,
    /// otherwise every magnitude in the system is scaled wrong.
    #[test]
    fn coherent_gain_matches_generated_window() {
        for kind in [WindowKind::Hann, WindowKind::BlackmanHarris] {
            let w = kind.generate(4096);
            let mean = w.iter().map(|&x| x as f64).sum::<f64>() / w.len() as f64;
            assert!(
                (mean - kind.coherent_gain() as f64).abs() < 1e-6,
                "{kind:?}: generated mean {mean} != declared coherent gain {}",
                kind.coherent_gain()
            );
        }
    }

    #[test]
    fn windows_are_non_negative_and_peak_near_one() {
        for kind in [WindowKind::Hann, WindowKind::BlackmanHarris] {
            let w = kind.generate(1024);
            let max = w.iter().cloned().fold(f32::MIN, f32::max);
            assert!(w.iter().all(|&x| x >= -1e-6), "{kind:?} went negative");
            assert!((max - 1.0).abs() < 1e-3, "{kind:?} peak was {max}");
        }
    }
}
