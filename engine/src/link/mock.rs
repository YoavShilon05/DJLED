//! A link that goes nowhere, for developing without hardware.
//!
//! Expands frames exactly as the firmware does and keeps the resulting pixels
//! for inspection, so the whole pipeline — capture, analysis, colour surface,
//! protocol framing, strip expansion — can be exercised and watched before the
//! Arduino is wired up. The caller renders [`MockLink::leds`] however it likes;
//! [`MockLink::render_ansi`] produces a truecolour strip for a terminal.

use anyhow::Result;

use super::protocol::{self, TestPattern};
use super::{expand_bands_to_leds, Link};

pub struct MockLink {
    leds: Vec<[u8; 3]>,
    /// Encoded bytes of the last frame, so wire size is visible even with no
    /// device attached.
    last_frame_bytes: usize,
    frames: u64,
    scratch: Vec<u8>,
}

impl MockLink {
    pub fn new(led_count: usize) -> Self {
        Self {
            leds: vec![[0; 3]; led_count],
            last_frame_bytes: 0,
            frames: 0,
            scratch: Vec::new(),
        }
    }

    /// The strip as the firmware would drive it.
    pub fn leds(&self) -> &[[u8; 3]] {
        &self.leds
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Bytes the last frame occupied on the wire, and what that costs in time.
    pub fn last_frame_bytes(&self) -> usize {
        self.last_frame_bytes
    }

    /// Render the strip as a line of truecolour blocks, downsampled to `width`
    /// characters by averaging so the preview is faithful at any terminal size.
    pub fn render_ansi(&self, width: usize) -> String {
        if self.leds.is_empty() || width == 0 {
            return String::new();
        }

        let mut out = String::with_capacity(width * 20 + 8);
        for col in 0..width {
            let start = col * self.leds.len() / width;
            let end = ((col + 1) * self.leds.len() / width).max(start + 1).min(self.leds.len());

            let n = (end - start) as u32;
            let mut acc = [0u32; 3];
            for px in &self.leds[start..end] {
                for c in 0..3 {
                    acc[c] += px[c] as u32;
                }
            }

            out.push_str(&format!(
                "\x1b[48;2;{};{};{}m ",
                acc[0] / n,
                acc[1] / n,
                acc[2] / n
            ));
        }
        out.push_str("\x1b[0m");
        out
    }
}

impl Link for MockLink {
    fn describe(&self) -> String {
        format!("mock link — {} LEDs, nothing is driven", self.leds.len())
    }

    fn send(&mut self, bands: &[[u8; 3]]) -> Result<bool> {
        // Encode for real so framing limits are enforced here too; a band count
        // the protocol cannot carry must fail with or without hardware.
        protocol::encode_band_rgb(bands, &mut self.scratch).map_err(anyhow::Error::msg)?;
        self.last_frame_bytes = self.scratch.len();

        expand_bands_to_leds(bands, &mut self.leds);
        self.frames += 1;
        Ok(true)
    }

    fn send_test(&mut self, pattern: TestPattern) -> Result<()> {
        let colour = match pattern {
            TestPattern::Rgb => [255, 0, 0],
            TestPattern::Chase => [0, 0, 0],
            TestPattern::White => [255, 255, 255],
        };
        self.leds.fill(colour);
        Ok(())
    }

    fn blackout(&mut self) -> Result<()> {
        self.leds.fill([0; 3]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_bands_across_the_configured_strip() {
        let mut link = MockLink::new(150);
        let bands: Vec<[u8; 3]> = (0..48).map(|i| [i * 5, 0, 255 - i * 5]).collect();

        assert!(link.send(&bands).unwrap());
        assert_eq!(link.leds().len(), 150);
        assert_eq!(link.leds()[0], bands[0]);
        assert_eq!(link.leds()[149], bands[47]);
        assert_eq!(link.frames(), 1);
    }

    /// The mock must enforce protocol limits, or a band count that works in
    /// development would fail the moment real hardware is attached.
    #[test]
    fn rejects_frames_the_protocol_cannot_carry() {
        let mut link = MockLink::new(150);
        assert!(link.send(&[[0; 3]; 86]).is_err());
    }

    #[test]
    fn reports_wire_size() {
        let mut link = MockLink::new(150);
        link.send(&[[0; 3]; 48]).unwrap();
        assert_eq!(link.last_frame_bytes(), 149);
    }

    #[test]
    fn ansi_preview_covers_the_requested_width() {
        let mut link = MockLink::new(150);
        link.send(&[[255, 0, 0]; 48]).unwrap();

        let line = link.render_ansi(60);
        assert_eq!(line.matches("\x1b[48;2;").count(), 60);
        assert!(line.ends_with("\x1b[0m"));
    }

    /// Downsampling must not divide by zero or read out of bounds when the
    /// preview is wider than the strip.
    #[test]
    fn ansi_preview_survives_width_above_led_count() {
        let mut link = MockLink::new(8);
        link.send(&[[10, 20, 30]; 48]).unwrap();
        assert_eq!(link.render_ansi(100).matches("\x1b[48;2;").count(), 100);
    }

    #[test]
    fn blackout_clears_the_strip() {
        let mut link = MockLink::new(16);
        link.send(&[[255; 3]; 48]).unwrap();
        link.blackout().unwrap();
        assert!(link.leds().iter().all(|&p| p == [0, 0, 0]));
    }
}
