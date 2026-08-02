//! Wire protocol between the PC and the Arduino.
//!
//! # Framing
//!
//! ```text
//! MCU -> PC   0x7E                       READY, a bare byte
//!
//! PC  -> MCU  0xA5 0x5A                  magic
//!             u8   kind                  frame type
//!             u8   len                   payload bytes
//!             ..   payload
//!             u8   crc8                  over kind, len and payload
//!
//! MCU -> PC   same framing               for HELLO replies
//! ```
//!
//! # Why READY exists
//!
//! FastLED bit-bangs WS2812B with interrupts disabled for the whole strip write
//! — 30 µs per LED, so 4.5 ms at 150 LEDs and 18 ms at 600. The ATmega328P's
//! UART has a two-byte FIFO, which overruns after about 40 µs at 500 kbaud.
//! Streaming into a deaf MCU therefore loses nearly every frame.
//!
//! So the MCU asks. It emits READY when it can actually receive, the PC sends
//! exactly one frame, and the MCU then goes deaf to write the strip before
//! asking again. The PC computes the next frame during that blackout, so the
//! pipeline overlaps at no cost.
//!
//! (FastLED's `FASTLED_ALLOW_INTERRUPTS` claims to make this unnecessary by
//! retrying corrupted pixels, but WS2812 timing is tight enough that it is not
//! dependable. Explicit flow control is deterministic.)
//!
//! # Why colour is per band rather than per LED
//!
//! At 600 LEDs a full RGB frame is 1800 bytes — 36 ms at 500 kbaud, which would
//! cap the display at 27 fps before the strip write is even considered. Sending
//! one colour per *band* costs 48×3 = 144 bytes regardless of strip length, and
//! the MCU interpolates across the LEDs between them. Bandwidth is decoupled
//! from LED count entirely, and every colour decision stays on the PC where the
//! keyframe surface lives.

/// Sent by the MCU when it is ready to receive exactly one frame.
pub const READY: u8 = 0x7E;
pub const MAGIC: [u8; 2] = [0xA5, 0x5A];

/// Largest payload the length byte can describe.
pub const MAX_PAYLOAD: usize = 255;
/// Bytes of framing around every payload.
pub const OVERHEAD: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// PC asks who is there; MCU replies with the same kind and a [`Hello`] payload.
    Hello = 0x00,
    /// One RGB triplet per band.
    BandRgb = 0x01,
    /// Built-in diagnostic patterns; payload is a single [`TestPattern`].
    Test = 0x02,
    /// Blank the strip immediately.
    Blackout = 0x03,
}

impl Kind {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0x00 => Kind::Hello,
            0x01 => Kind::BandRgb,
            0x02 => Kind::Test,
            0x03 => Kind::Blackout,
            _ => return None,
        })
    }
}

/// Diagnostics for bringing up wiring, before trusting anything about the audio
/// path. Each isolates one failure mode — see `docs/wiring.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TestPattern {
    /// Solid red, then green, then blue. Confirms channel order: WS2812B is
    /// physically GRB, so a swapped strip shows red as green.
    Rgb = 0x00,
    /// A single lit pixel walking the strip. Confirms LED count and direction.
    Chase = 0x01,
    /// Full white. Reveals voltage droop and where power injection is needed —
    /// the far end goes orange before it goes dim.
    White = 0x02,
}

/// What the MCU reports about itself, so the PC can validate its configuration
/// instead of silently sending frames the firmware cannot use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hello {
    pub protocol_version: u8,
    pub firmware_version: u8,
    pub led_count: u16,
    pub max_bands: u8,
}

pub const PROTOCOL_VERSION: u8 = 1;
const HELLO_LEN: usize = 5;

impl Hello {
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.protocol_version);
        out.push(self.firmware_version);
        // Little-endian, matching AVR byte order so the firmware can memcpy.
        out.extend_from_slice(&self.led_count.to_le_bytes());
        out.push(self.max_bands);
    }

    fn decode(payload: &[u8]) -> Option<Self> {
        if payload.len() < HELLO_LEN {
            return None;
        }
        Some(Self {
            protocol_version: payload[0],
            firmware_version: payload[1],
            led_count: u16::from_le_bytes([payload[2], payload[3]]),
            max_bands: payload[4],
        })
    }
}

/// CRC-8, polynomial 0x07, initial value 0.
///
/// Bitwise rather than table-driven: the firmware runs the identical routine on
/// an AVR where 256 bytes of flash for a table is worth more than the ~20 µs
/// this costs over a 150-byte frame.
pub fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

fn frame(kind: Kind, payload: &[u8], out: &mut Vec<u8>) {
    debug_assert!(payload.len() <= MAX_PAYLOAD);
    out.clear();
    out.extend_from_slice(&MAGIC);
    out.push(kind as u8);
    out.push(payload.len() as u8);
    out.extend_from_slice(payload);
    // CRC covers kind, length and payload — everything but the magic, which is
    // only there to resynchronise after a desync.
    out.push(crc8(&out[MAGIC.len()..]));
}

/// Encode one RGB triplet per band.
///
/// Errors if `pixels` exceeds what the length byte can carry (85 bands).
pub fn encode_band_rgb(pixels: &[[u8; 3]], out: &mut Vec<u8>) -> Result<(), String> {
    let len = pixels.len() * 3;
    if len > MAX_PAYLOAD {
        return Err(format!(
            "{} bands need {len} payload bytes, over the {MAX_PAYLOAD}-byte limit",
            pixels.len()
        ));
    }
    let mut payload = Vec::with_capacity(len);
    for px in pixels {
        payload.extend_from_slice(px);
    }
    frame(Kind::BandRgb, &payload, out);
    Ok(())
}

pub fn encode_hello(out: &mut Vec<u8>) {
    frame(Kind::Hello, &[], out);
}

pub fn encode_hello_reply(hello: &Hello, out: &mut Vec<u8>) {
    let mut payload = Vec::with_capacity(HELLO_LEN);
    hello.encode(&mut payload);
    frame(Kind::Hello, &payload, out);
}

pub fn encode_test(pattern: TestPattern, out: &mut Vec<u8>) {
    frame(Kind::Test, &[pattern as u8], out);
}

pub fn encode_blackout(out: &mut Vec<u8>) {
    frame(Kind::Blackout, &[], out);
}

/// Maximum bands the length byte allows.
pub fn max_bands() -> usize {
    MAX_PAYLOAD / 3
}

/// Incremental decoder for the MCU-to-PC direction.
///
/// Resynchronises on its own: bytes that are not a valid frame are discarded
/// while scanning for the magic, so a partially-read buffer or a mid-stream
/// connection recovers without intervention.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Ready,
    Hello(Hello),
    /// A frame arrived intact but was not understood — a newer firmware, most
    /// likely. Surfaced rather than swallowed so version drift is visible.
    Unknown(u8),
    /// Framing was intact but the CRC did not match.
    BadCrc,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Pull the next complete event, or `None` when more bytes are needed.
    pub fn next_event(&mut self) -> Option<Event> {
        loop {
            if self.buf.is_empty() {
                return None;
            }

            if self.buf[0] == READY {
                self.buf.remove(0);
                return Some(Event::Ready);
            }

            // Scan for the start of a frame, dropping anything before it.
            if self.buf[0] != MAGIC[0] {
                let start = self.buf.iter().position(|&b| b == MAGIC[0] || b == READY);
                match start {
                    Some(0) => unreachable!("handled above"),
                    Some(i) => {
                        self.buf.drain(..i);
                        continue;
                    }
                    None => {
                        self.buf.clear();
                        return None;
                    }
                }
            }

            if self.buf.len() < OVERHEAD {
                return None;
            }
            if self.buf[1] != MAGIC[1] {
                // False start: drop the byte and rescan rather than assuming
                // the rest of the buffer is garbage.
                self.buf.remove(0);
                continue;
            }

            let kind = self.buf[2];
            let len = self.buf[3] as usize;
            let total = OVERHEAD + len;
            if self.buf.len() < total {
                return None;
            }

            let body = &self.buf[MAGIC.len()..total - 1];
            let expected = self.buf[total - 1];
            let actual = crc8(body);
            let payload: Vec<u8> = self.buf[4..total - 1].to_vec();
            self.buf.drain(..total);

            if actual != expected {
                return Some(Event::BadCrc);
            }

            return Some(match Kind::from_u8(kind) {
                Some(Kind::Hello) => match Hello::decode(&payload) {
                    Some(h) => Event::Hello(h),
                    None => Event::Unknown(kind),
                },
                _ => Event::Unknown(kind),
            });
        }
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> Hello {
        Hello { protocol_version: PROTOCOL_VERSION, firmware_version: 3, led_count: 150, max_bands: 64 }
    }

    #[test]
    fn band_rgb_frame_has_the_documented_layout() {
        let mut out = Vec::new();
        encode_band_rgb(&[[1, 2, 3], [4, 5, 6]], &mut out).unwrap();

        assert_eq!(&out[..2], &MAGIC);
        assert_eq!(out[2], Kind::BandRgb as u8);
        assert_eq!(out[3], 6, "length byte counts payload bytes, not bands");
        assert_eq!(&out[4..10], &[1, 2, 3, 4, 5, 6]);
        assert_eq!(out[10], crc8(&out[2..10]));
        assert_eq!(out.len(), OVERHEAD + 6);
    }

    /// The frame budget that makes the whole per-band design work: 48 bands must
    /// stay small enough to send in a couple of milliseconds.
    #[test]
    fn a_48_band_frame_fits_the_latency_budget() {
        let mut out = Vec::new();
        encode_band_rgb(&[[0; 3]; 48], &mut out).unwrap();
        assert_eq!(out.len(), 149);

        let ms = out.len() as f64 * 10.0 / 500_000.0 * 1000.0;
        assert!(ms < 3.5, "48 bands take {ms:.2} ms at 500 kbaud");
    }

    #[test]
    fn rejects_more_bands_than_the_length_byte_allows() {
        let mut out = Vec::new();
        assert!(encode_band_rgb(&[[0; 3]; 85], &mut out).is_ok());
        assert!(encode_band_rgb(&[[0; 3]; 86], &mut out).is_err());
        assert_eq!(max_bands(), 85);
    }

    #[test]
    fn hello_round_trips() {
        let mut out = Vec::new();
        encode_hello_reply(&hello(), &mut out);

        let mut d = Decoder::default();
        d.push(&out);
        assert_eq!(d.next_event(), Some(Event::Hello(hello())));
        assert_eq!(d.next_event(), None);
    }

    #[test]
    fn ready_is_decoded_as_a_bare_byte() {
        let mut d = Decoder::default();
        d.push(&[READY, READY]);
        assert_eq!(d.next_event(), Some(Event::Ready));
        assert_eq!(d.next_event(), Some(Event::Ready));
        assert_eq!(d.next_event(), None);
    }

    /// Serial delivers arbitrary fragments; a frame split across reads must
    /// still decode.
    #[test]
    fn frames_split_across_reads_reassemble() {
        let mut out = Vec::new();
        encode_hello_reply(&hello(), &mut out);

        let mut d = Decoder::default();
        for byte in &out {
            assert_eq!(d.next_event(), None, "emitted an event before the frame completed");
            d.push(&[*byte]);
        }
        assert_eq!(d.next_event(), Some(Event::Hello(hello())));
    }

    #[test]
    fn corrupted_payload_is_reported_not_accepted() {
        let mut out = Vec::new();
        encode_hello_reply(&hello(), &mut out);
        out[5] ^= 0xFF;

        let mut d = Decoder::default();
        d.push(&out);
        assert_eq!(d.next_event(), Some(Event::BadCrc));
    }

    /// Connecting mid-stream lands the reader in the middle of a frame. It must
    /// discard the fragment and pick up the next clean one.
    #[test]
    fn resynchronises_after_leading_garbage() {
        let mut frame = Vec::new();
        encode_hello_reply(&hello(), &mut frame);

        let mut stream = vec![0x11, 0x22, 0xA5, 0x33, 0x00, 0xFF];
        stream.extend_from_slice(&frame);

        let mut d = Decoder::default();
        d.push(&stream);

        let mut events = Vec::new();
        while let Some(e) = d.next_event() {
            events.push(e);
        }
        assert!(
            events.contains(&Event::Hello(hello())),
            "did not recover the good frame, saw {events:?}"
        );
    }

    /// A READY arriving between frames must not be swallowed by the frame
    /// scanner — losing one stalls the link until the watchdog fires.
    #[test]
    fn ready_interleaved_with_frames_survives() {
        let mut stream = vec![READY];
        let mut frame = Vec::new();
        encode_hello_reply(&hello(), &mut frame);
        stream.extend_from_slice(&frame);
        stream.push(READY);

        let mut d = Decoder::default();
        d.push(&stream);
        assert_eq!(d.next_event(), Some(Event::Ready));
        assert_eq!(d.next_event(), Some(Event::Hello(hello())));
        assert_eq!(d.next_event(), Some(Event::Ready));
    }

    #[test]
    fn unknown_frame_kinds_are_surfaced() {
        let mut out = Vec::new();
        frame(Kind::Blackout, &[], &mut out);

        let mut d = Decoder::default();
        d.push(&out);
        assert_eq!(d.next_event(), Some(Event::Unknown(Kind::Blackout as u8)));
    }

    /// Known-answer check so the firmware's independent implementation can be
    /// verified against the same vectors.
    #[test]
    fn crc8_matches_known_vectors() {
        assert_eq!(crc8(&[]), 0x00);
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc8(&[0x00]), 0x00);
        assert_eq!(crc8(&[0xFF]), 0xF3);
    }
}
