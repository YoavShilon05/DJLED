//! Getting frames to the strip.
//!
//! [`protocol`] defines the wire format, [`serial`] speaks it to a real Arduino,
//! and [`mock`] renders frames in the terminal so the entire pipeline can be
//! developed and demonstrated before any hardware exists.

pub mod mock;
pub mod protocol;
pub mod serial;

use anyhow::Result;

pub use mock::MockLink;
pub use protocol::{Hello, TestPattern};
pub use serial::SerialLink;

/// A destination for rendered frames.
pub trait Link {
    /// Human-readable description, for startup logging.
    fn describe(&self) -> String;

    /// Send one frame of per-band colours.
    ///
    /// Returns `false` if the device was not ready within its timeout, which is
    /// a dropped frame rather than an error — the next one will catch up, and
    /// the display is better off skipping than queueing stale data.
    fn send(&mut self, bands: &[[u8; 3]]) -> Result<bool>;

    /// Display a diagnostic pattern. Not all links support every pattern.
    fn send_test(&mut self, _pattern: TestPattern) -> Result<()> {
        Ok(())
    }

    /// Blank the strip.
    fn blackout(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Expand per-band colours across the physical strip.
///
/// This mirrors what the firmware does, and exists here so the mock link
/// previews the real thing and so the logic is testable without hardware.
///
/// Interpolation is linear in 8-bit RGB rather than in Oklab. That is a
/// deliberate shortcut: adjacent bands are sampled from a continuous colour
/// surface, so consecutive colours are always close together, and over a short
/// hop the perceptual error is far below what an 8-bit LED can show. Doing it
/// properly would mean Oklab conversions inside an AVR interrupt-disabled
/// section, which is not affordable.
pub fn expand_bands_to_leds(bands: &[[u8; 3]], leds: &mut [[u8; 3]]) {
    let (b, n) = (bands.len(), leds.len());
    if b == 0 || n == 0 {
        return;
    }
    if b == 1 || n == 1 {
        leds.fill(bands[0]);
        return;
    }

    let span = (b - 1) as f32 / (n - 1) as f32;
    for (i, led) in leds.iter_mut().enumerate() {
        let pos = i as f32 * span;
        let j = (pos.floor() as usize).min(b - 2);
        let t = pos - j as f32;
        for ((out, &lo), &hi) in led.iter_mut().zip(&bands[j]).zip(&bands[j + 1]) {
            let (lo, hi) = (lo as f32, hi as f32);
            *out = (lo + (hi - lo) * t).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_map_to_the_first_and_last_band() {
        let bands = [[10, 20, 30], [40, 50, 60], [70, 80, 90]];
        let mut leds = [[0u8; 3]; 150];
        expand_bands_to_leds(&bands, &mut leds);
        assert_eq!(leds[0], bands[0]);
        assert_eq!(leds[149], bands[2]);
    }

    #[test]
    fn interpolation_is_monotonic_between_bands() {
        let bands = [[0, 0, 0], [255, 255, 255]];
        let mut leds = [[0u8; 3]; 150];
        expand_bands_to_leds(&bands, &mut leds);
        assert!(
            leds.windows(2).all(|w| w[1][0] >= w[0][0]),
            "ramp was not monotonic"
        );
    }

    /// Both strip sizes in play: 150 today, 600 when the 10 m run arrives.
    #[test]
    fn handles_both_planned_strip_lengths() {
        let bands: Vec<[u8; 3]> = (0..48).map(|i| [i * 5, 255 - i * 5, 128]).collect();
        for count in [150usize, 600] {
            let mut leds = vec![[0u8; 3]; count];
            expand_bands_to_leds(&bands, &mut leds);
            assert_eq!(leds[0], bands[0]);
            assert_eq!(leds[count - 1], bands[47]);
            assert!(leds.iter().all(|p| p.iter().any(|&c| c > 0)));
        }
    }

    /// More bands than LEDs is legal and must not panic or read out of bounds.
    #[test]
    fn survives_more_bands_than_leds() {
        let bands: Vec<[u8; 3]> = (0..48).map(|i| [i, i, i]).collect();
        let mut leds = [[0u8; 3]; 8];
        expand_bands_to_leds(&bands, &mut leds);
        assert_eq!(leds[0], [0, 0, 0]);
        assert_eq!(leds[7], [47, 47, 47]);
    }

    #[test]
    fn degenerate_inputs_are_safe() {
        let mut leds = [[9u8; 3]; 4];
        expand_bands_to_leds(&[], &mut leds);
        assert_eq!(leds, [[9; 3]; 4], "empty band list should leave output alone");

        expand_bands_to_leds(&[[1, 2, 3]], &mut leds);
        assert_eq!(leds, [[1, 2, 3]; 4], "single band should fill uniformly");

        let mut one = [[0u8; 3]; 1];
        expand_bands_to_leds(&[[1, 2, 3], [4, 5, 6]], &mut one);
        assert_eq!(one[0], [1, 2, 3]);
    }
}
