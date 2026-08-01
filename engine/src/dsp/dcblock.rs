//! First-order DC blocker.
//!
//! A DC offset of 0.01 (-40 dBFS, completely inaudible) puts the sub-bass bar at
//! -31 dB — far above the ~-70 dB threshold where an LED visibly lights — and
//! *no window function fixes it*, because DC energy lands in bin 0-1 by
//! definition and at short FFT sizes bin 1 IS the sub-bass band. At N=256 bin 1
//! sits at 187 Hz, so the smear covers the entire bass region. That is precisely
//! why the artefact shows up in short real-time windows and not in long offline
//! ones.
//!
//! Measured, 1 kHz tone + 0.01 DC, N=1024, Hann:
//!   without blocker: sub-bass bar at -33 dB  (ghost bar clearly visible)
//!   with blocker:    sub-bass bar at -74 dB  (clean)
//!
//! # Its actual role here
//!
//! In *this* analyser the blocker is defence in depth rather than the primary
//! fix, and it is worth being precise about that. The tier rule in
//! [`super::bands`] requires a band's lower edge to clear the discarded bins, so
//! the 40 Hz band is forced onto N=8192, where the discard zone ends at 23.4 Hz
//! — below the band entirely. DC smear structurally cannot reach any displayed
//! band, and disabling this filter does not bring the ghost bar back (there is a
//! test asserting exactly that).
//!
//! It stays because the guarantee is contingent on configuration. Lower `f_min`,
//! shrink the FFT ladder, or switch to a window with a wider main lobe and the
//! margin narrows. This costs three lines and one multiply per sample to make
//! the artefact impossible rather than merely improbable.

/// `y[n] = x[n] - x[n-1] + R * y[n-1]`
///
/// The pole at `R` sets the cutoff: `fc ≈ (1 - R) * fs / 2π`. At the default
/// R=0.9995 and 48 kHz that is ~3.8 Hz, comfortably below the 40 Hz bottom of
/// the display, so it removes the offset without touching any visible band.
#[derive(Clone, Debug)]
pub struct DcBlocker {
    r: f32,
    prev_x: f32,
    prev_y: f32,
}

impl DcBlocker {
    pub const DEFAULT_R: f32 = 0.9995;

    pub fn new(r: f32) -> Self {
        Self { r, prev_x: 0.0, prev_y: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = x - self.prev_x + self.r * self.prev_y;
        self.prev_x = x;
        self.prev_y = y;
        y
    }

    pub fn process_block(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            *s = self.process(*s);
        }
    }

    pub fn reset(&mut self) {
        self.prev_x = 0.0;
        self.prev_y = 0.0;
    }
}

impl Default for DcBlocker {
    fn default() -> Self {
        Self::new(Self::DEFAULT_R)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The headline claim: a pure DC offset is driven to essentially nothing.
    #[test]
    fn removes_constant_offset() {
        let mut dc = DcBlocker::default();
        let mut last = 0.0;
        for _ in 0..48_000 {
            last = dc.process(0.25);
        }
        assert!(last.abs() < 1e-3, "residual DC {last} after 1 s");
    }

    /// ...while leaving the audible band alone. A 1 kHz tone must survive with
    /// its amplitude intact, or the blocker is stealing signal, not offset.
    #[test]
    fn preserves_audible_tone() {
        let mut dc = DcBlocker::default();
        let sr = 48_000.0f32;
        let mut peak: f32 = 0.0;
        for n in 0..48_000 {
            let x = (std::f32::consts::TAU * 1000.0 * n as f32 / sr).sin();
            let y = dc.process(x);
            if n > 4_800 {
                peak = peak.max(y.abs());
            }
        }
        assert!((peak - 1.0).abs() < 0.01, "1 kHz peak became {peak}");
    }

    /// Even the lowest displayed band (40 Hz) must pass essentially untouched,
    /// which is what justifies the 3.8 Hz corner.
    #[test]
    fn preserves_lowest_displayed_band() {
        let mut dc = DcBlocker::default();
        let sr = 48_000.0f32;
        let mut peak: f32 = 0.0;
        for n in 0..96_000 {
            let x = (std::f32::consts::TAU * 40.0 * n as f32 / sr).sin();
            let y = dc.process(x);
            if n > 48_000 {
                peak = peak.max(y.abs());
            }
        }
        assert!(peak > 0.99, "40 Hz was attenuated to {peak}");
    }
}
