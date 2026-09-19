//! Serial link to the Arduino.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serialport::SerialPort;

use super::protocol::{
    self, Decoder, Event, Hello, TestPattern, PROTOCOL_VERSION,
};
use super::Link;

/// 500 kbaud has 0% error with U2X at 16 MHz, unlike 115200's 2.1%. 1 Mbaud
/// often works on a genuine Uno R3 or an FT232 Nano and halves transfer time,
/// but some CH340 clones are unreliable there, so this is the safe default.
pub const DEFAULT_BAUD: u32 = 500_000;

/// Opening the port asserts DTR, which resets the board into its bootloader.
/// That costs roughly two seconds before the sketch runs, so the handshake has
/// to outwait it.
const RESET_GRACE: Duration = Duration::from_secs(4);

/// How long to wait for READY before giving up on a frame. Comfortably longer
/// than the worst strip write (18 ms at 600 LEDs) plus transfer.
const READY_TIMEOUT: Duration = Duration::from_millis(120);

/// How long a READY is worth acting on.
///
/// The sketch announces itself and then listens for `FRAME_TIMEOUT_MS` (25 ms
/// in `firmware/djled/djled.ino`) before going back round its loop to write the
/// strip with interrupts off. Past that window the board is deaf, so a frame
/// sent on the strength of an older token is a frame torn in half.
const READY_LIFETIME: Duration = Duration::from_millis(25);

/// A backlog this big is not a conversation. READY is one byte and the only
/// other thing the board ever sends is a ten-byte HELLO, so hundreds of queued
/// bytes mean nothing has read this port for a long while — see [`Readiness`].
const STALE_INPUT: usize = 256;

/// Cap on reads per drain, so a board that talks faster than we read cannot
/// hold the loop. 16 × 512 bytes is far more than a sane board produces.
const MAX_DRAIN_READS: usize = 16;

pub struct SerialLink {
    port: Box<dyn SerialPort>,
    decoder: Decoder,
    hello: Hello,
    path: String,
    baud: u32,
    scratch: Vec<u8>,
    read_buf: [u8; 512],
    ready: Readiness,
}

/// The board's readiness: one invitation with a shelf life, not a quantity.
///
/// READY means *send one frame now*. The firmware writes it, listens for
/// `FRAME_TIMEOUT_MS`, then goes round its loop — strip write with interrupts
/// off, UART deaf — and announces again. There is never more than one
/// invitation outstanding, and an old one is worth nothing.
///
/// This used to be a counter, and counting is what broke long sessions. Any
/// stretch where the PC sends slower than the board asks banks tokens: a silent
/// loopback sends no frames at all and nothing reads the port for as long as
/// the silence lasts. The bank was then spent all at once, streaming frames
/// into a board that is deaf for 4.5 ms out of every 7. Bytes lost during each
/// strip write leave the sketch's reader mid-frame, and because the stream
/// never stops there is no gap to resynchronise in — so the wall drops to about
/// a frame a second and runs seconds behind the music while the terminal
/// preview and the editor, which never cross the wire, stay perfectly correct.
#[derive(Default)]
struct Readiness(Option<Instant>);

impl Readiness {
    /// Record a READY. It *supersedes* any outstanding one rather than adding
    /// to it, which is what bounds us to a single frame in flight.
    fn offered(&mut self) {
        self.0 = Some(Instant::now());
    }

    /// Spend the invitation if it is still live. An expired one is discarded —
    /// the board has moved on and the next announcement is a board loop away.
    fn take(&mut self) -> bool {
        match self.0.take() {
            Some(at) => at.elapsed() < READY_LIFETIME,
            None => false,
        }
    }

    fn clear(&mut self) {
        self.0 = None;
    }
}

impl SerialLink {
    pub fn open(path: &str, baud: u32) -> Result<Self> {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_millis(5))
            .open()
            .with_context(|| format!("could not open {path} at {baud} baud"))?;

        let mut link = Self {
            port,
            decoder: Decoder::default(),
            hello: Hello {
                protocol_version: 0,
                firmware_version: 0,
                led_count: 0,
                max_bands: 0,
            },
            path: path.to_string(),
            baud,
            scratch: Vec::with_capacity(protocol::MAX_PAYLOAD + protocol::OVERHEAD),
            read_buf: [0; 512],
            ready: Readiness::default(),
        };

        link.hello = link.handshake()?;
        if link.hello.protocol_version != PROTOCOL_VERSION {
            return Err(anyhow!(
                "firmware speaks protocol v{} but this build speaks v{PROTOCOL_VERSION} — reflash the sketch in firmware/",
                link.hello.protocol_version
            ));
        }
        Ok(link)
    }

    /// List candidate ports, for error messages and `--list-ports`.
    pub fn available() -> Vec<String> {
        serialport::available_ports()
            .map(|ports| ports.into_iter().map(|p| p.port_name).collect())
            .unwrap_or_default()
    }

    pub fn hello(&self) -> &Hello {
        &self.hello
    }

    /// Send HELLO until the board answers, tolerating the bootloader reset.
    fn handshake(&mut self) -> Result<Hello> {
        let deadline = Instant::now() + RESET_GRACE;
        let mut last_probe = Instant::now() - Duration::from_secs(1);

        // Anything buffered predates the reset and is meaningless.
        let _ = self.port.clear(serialport::ClearBuffer::All);
        self.decoder.clear();

        while Instant::now() < deadline {
            if last_probe.elapsed() >= Duration::from_millis(250) {
                protocol::encode_hello(&mut self.scratch);
                let _ = self.port.write_all(&self.scratch);
                let _ = self.port.flush();
                last_probe = Instant::now();
            }

            if self.drain()? {
                return Ok(self.hello);
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        Err(anyhow!(
            "no reply from {} after {:.0}s. Check the board is running the sketch from firmware/, \
             that no other program holds the port, and that the baud rate matches ({} here).",
            self.path,
            RESET_GRACE.as_secs_f32(),
            self.baud
        ))
    }

    /// Read whatever is available into the decoder.
    fn pump(&mut self) -> Result<()> {
        match self.port.read(&mut self.read_buf) {
            Ok(0) => Ok(()),
            Ok(n) => {
                self.decoder.push(&self.read_buf[..n]);
                Ok(())
            }
            // The short read timeout is the normal idle path, not a fault.
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(()),
            Err(e) => Err(e).context("serial read failed"),
        }
    }

    /// Fold everything the decoder has into state. Reports whether a HELLO
    /// arrived, which is the one event a caller has to react to.
    fn consume_events(&mut self) -> bool {
        let mut hello = false;
        while let Some(event) = self.decoder.next_event() {
            match event {
                Event::Ready => self.ready.offered(),
                Event::Hello(h) => {
                    self.hello = h;
                    hello = true;
                }
                Event::BadCrc | Event::Unknown(_) => {}
            }
        }
        hello
    }

    /// Read the port empty, not merely once.
    ///
    /// Draining to the end is what keeps READY meaningful: only the newest one
    /// describes the board *now*, and stopping at one bufferful would leave
    /// older bytes to be mistaken for it on the next pass.
    fn drain(&mut self) -> Result<bool> {
        // Nothing reads this port while the source is silent — the loop never
        // reaches `send` — so the driver's input queue can be holding minutes
        // of readiness by the time audio comes back. Decoding that pile is
        // pointless and treating the end of it as fresh is wrong, so it goes in
        // the bin and we wait for the next announcement, one board loop away.
        if self.port.bytes_to_read().unwrap_or(0) as usize > STALE_INPUT {
            let _ = self.port.clear(serialport::ClearBuffer::Input);
            self.decoder.clear();
            self.ready.clear();
            return Ok(false);
        }

        let mut hello = false;
        for _ in 0..MAX_DRAIN_READS {
            self.pump()?;
            hello |= self.consume_events();
            if self.port.bytes_to_read().unwrap_or(0) == 0 {
                break;
            }
        }
        Ok(hello)
    }

    /// Wait for the board to say it can receive, and spend that invitation.
    ///
    /// Returns false on timeout, which the caller counts as a dropped frame.
    /// Dropping is the right answer: the alternative is writing into a deaf
    /// UART, which costs the next frame as well as this one.
    fn await_ready(&mut self, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain()?;
            if self.ready.take() {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    fn write_frame(&mut self) -> Result<()> {
        self.port.write_all(&self.scratch).context("serial write failed")?;
        self.port.flush().context("serial flush failed")?;
        Ok(())
    }
}

impl Link for SerialLink {
    fn describe(&self) -> String {
        format!(
            "{} @ {} baud — firmware v{}, {} LEDs, up to {} bands",
            self.path, self.baud, self.hello.firmware_version, self.hello.led_count, self.hello.max_bands
        )
    }

    /// What the board said it can take, in the handshake it has already
    /// completed by the time this can be called.
    ///
    /// A zero is treated as "did not say" rather than as "none": the sketch has
    /// always reported this, but reading a broken value as a limit of nothing
    /// would blank the strip on the strength of one bad byte.
    fn max_points(&self) -> usize {
        match self.hello.max_bands as usize {
            0 => protocol::max_bands(),
            n => n.min(protocol::max_bands()),
        }
    }

    fn send(&mut self, bands: &[[u8; 3]]) -> Result<bool> {
        if !self.await_ready(READY_TIMEOUT)? {
            return Ok(false);
        }
        protocol::encode_band_rgb(bands, &mut self.scratch).map_err(|e| anyhow!(e))?;
        self.write_frame()?;
        Ok(true)
    }

    fn send_test(&mut self, pattern: TestPattern) -> Result<()> {
        self.await_ready(READY_TIMEOUT)?;
        protocol::encode_test(pattern, &mut self.scratch);
        self.write_frame()
    }

    fn blackout(&mut self) -> Result<()> {
        self.await_ready(READY_TIMEOUT)?;
        protocol::encode_blackout(&mut self.scratch);
        self.write_frame()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_invitation_is_spent_once() {
        let mut ready = Readiness::default();
        ready.offered();
        assert!(ready.take());
        assert!(!ready.take(), "the same READY was spent twice");
    }

    /// The regression this file exists for: a board that announces itself all
    /// through a silent stretch must not leave a bank of frames to be fired off
    /// the moment the music comes back. One READY on the wire is one frame.
    #[test]
    fn readiness_does_not_bank_up_while_nobody_is_sending() {
        let mut ready = Readiness::default();
        for _ in 0..500 {
            ready.offered();
        }
        assert!(ready.take(), "the most recent invitation still stands");
        assert!(!ready.take(), "the other 499 were banked");
    }

    #[test]
    fn an_invitation_older_than_the_boards_listening_window_is_not_spent() {
        let mut ready = Readiness(Some(Instant::now() - READY_LIFETIME - Duration::from_millis(1)));
        assert!(!ready.take(), "the board stopped listening 25 ms after it asked");
        assert!(!ready.take(), "and the expired token was not left behind");
    }

    #[test]
    fn nothing_is_spendable_before_the_board_speaks() {
        assert!(!Readiness::default().take());
    }
}
