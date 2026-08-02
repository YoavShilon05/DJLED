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

pub struct SerialLink {
    port: Box<dyn SerialPort>,
    decoder: Decoder,
    hello: Hello,
    path: String,
    baud: u32,
    scratch: Vec<u8>,
    read_buf: [u8; 512],
    /// READY tokens received but not yet spent. The MCU can signal readiness
    /// while we are still computing, and dropping those would halve throughput.
    credits: u32,
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
            credits: 0,
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

            self.pump()?;
            while let Some(event) = self.decoder.next_event() {
                match event {
                    Event::Hello(h) => return Ok(h),
                    // Expected while the sketch is already running and looping.
                    Event::Ready => self.credits += 1,
                    Event::BadCrc | Event::Unknown(_) => {}
                }
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

    /// Consume one READY credit, waiting for one if none is banked.
    fn await_ready(&mut self, timeout: Duration) -> Result<bool> {
        if self.credits > 0 {
            self.credits -= 1;
            return Ok(true);
        }

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.pump()?;
            let mut got = false;
            while let Some(event) = self.decoder.next_event() {
                match event {
                    Event::Ready => {
                        if got {
                            self.credits += 1;
                        } else {
                            got = true;
                        }
                    }
                    Event::Hello(h) => self.hello = h,
                    Event::BadCrc | Event::Unknown(_) => {}
                }
            }
            if got {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        Ok(false)
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
