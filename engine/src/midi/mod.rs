//! MIDI input — a keyboard, or a DAW sending notes down a virtual cable.
//!
//! # Two ways in, and only one of them is a cable
//!
//! **A controller** is a MIDI input port in its own right: plug a keyboard in
//! and it appears, no setup at all.
//!
//! **A DAW** is the interesting case. FL Studio's MIDI Out plugin sends to a
//! MIDI *output* port, and an application can only listen on a MIDI *input*
//! port — so something has to join the two, and Windows ships nothing that
//! does. The answer is a virtual MIDI cable driver: create a port in
//! **loopMIDI**, enable it under FL's Options → MIDI settings → Output and give
//! it a port number, then point the MIDI Out plugin at that number. It shows up
//! here as an ordinary input and no hardware is involved anywhere.
//!
//! The engine cannot create that port itself. A virtual MIDI port on Windows is
//! a kernel-mode driver, which means a signed driver package — not something a
//! user-space process can conjure. So the loopMIDI step is manual, once, and
//! [`open_hint`] says so at the moment it matters.
//!
//! # Threading
//!
//! The same shape as audio capture: the driver's callback does nothing but
//! parse three bytes and push them into a lock-free SPSC ring, and everything
//! else happens on the thread draining it. WinMM invokes that callback from its
//! own high-priority thread, so blocking or allocating there would cost real
//! timing accuracy.

pub mod notes;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use midir::{Ignore, MidiInput, MidiInputConnection};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapRb};

use crate::source::{DeviceInfo, Source, SourceKind};

pub use notes::{note_hz, note_name, MidiConfig, NoteEngine};

/// MIDI channels, which is a fixed property of the protocol rather than of any
/// device — a port always carries all sixteen.
pub const CHANNELS: usize = 16;

/// Name this process presents to the MIDI system.
const CLIENT: &str = "DJLED";

/// Messages buffered between the driver callback and the engine. Three bytes
/// each, so this is generous: a fast trill is tens of messages a second, and
/// the ring is drained every couple of milliseconds.
const RING: usize = 2048;

/// Port ids are prefixed so they can never be confused with a cpal device id,
/// which is what makes one `Source` type able to name both.
const ID_PREFIX: &str = "midi:";

/// One channel voice message, flattened.
///
/// Running status is not a concern: WinMM delivers each short message complete,
/// with its status byte, rather than as a raw byte stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
}

impl Message {
    pub const NOTE_OFF: u8 = 0x80;
    pub const NOTE_ON: u8 = 0x90;
    pub const CONTROL_CHANGE: u8 = 0xB0;

    pub const CC_SUSTAIN: u8 = 64;
    pub const CC_ALL_SOUND_OFF: u8 = 120;
    pub const CC_ALL_NOTES_OFF: u8 = 123;

    /// 0-based, as everything downstream counts channels.
    pub fn channel(self) -> u8 {
        self.status & 0x0F
    }

    /// The message type, with the channel masked off.
    pub fn kind(self) -> u8 {
        self.status & 0xF0
    }

    pub fn is_note_on(self) -> bool {
        self.kind() == Self::NOTE_ON && self.data2 > 0
    }
}

/// Every MIDI input that can be listened to right now.
///
/// Cheap — no port is opened — so unlike audio enumeration this could run on a
/// timer. It does not, only because there is nothing else that would want it to.
pub fn ports() -> Vec<DeviceInfo> {
    let Ok(input) = MidiInput::new(CLIENT) else { return Vec::new() };

    let mut out: Vec<DeviceInfo> = input
        .ports()
        .iter()
        .filter_map(|port| {
            Some(DeviceInfo {
                id: format!("{ID_PREFIX}{}", port.id()),
                name: input.port_name(port).ok()?,
                kind: SourceKind::Midi,
                is_default: false,
                // There is no sample rate in MIDI; `None` is what tells the
                // editor to show a dash rather than "0.0 kHz". The channel
                // count is not a property of the port at all — every MIDI
                // cable carries all sixteen.
                sample_rate: None,
                channels: Some(CHANNELS),
            })
        })
        .collect();

    out.sort_by_key(|d| d.name.to_lowercase());
    out
}

/// The port a selection would open, by name, or `None` if it names nothing that
/// exists.
///
/// Exists for one situation, which is not a corner case: a MIDI port admits one
/// listener at a time, so a selection that resolves to the port already open
/// must be recognised *before* anything tries to open it again and is refused by
/// its own process. Names rather than ids, because "the first MIDI input" and
/// that port's id are two selections that mean the same port.
pub fn resolve_name(source: &Source) -> Option<String> {
    let all = ports();
    match &source.id {
        Some(id) => all.into_iter().find(|d| &d.id == id).map(|d| d.name),
        None => all.into_iter().next().map(|d| d.name),
    }
}

/// An open MIDI input, draining into a ring.
pub struct MidiListener {
    /// Held to keep the port open; dropping it closes the connection.
    _connection: MidiInputConnection<()>,
    consumer: HeapCons<Message>,
    /// The selection as it was actually resolved. A request for "the first
    /// port" keeps `id: None` rather than being pinned to whichever port it
    /// happened to find, so the selection survives the port list changing.
    source: Source,
    port_name: String,
    dropped: Arc<AtomicU64>,
}

impl MidiListener {
    /// Open a port. `source.id` names one; `None` takes the first available,
    /// which is the only sensible default given MIDI has no notion of a system
    /// default device.
    pub fn open(source: &Source) -> Result<Self> {
        let mut input = MidiInput::new(CLIENT).context("could not open the MIDI system")?;
        // Sysex, timing clock and active sensing are all noise here, and
        // filtering them in the driver keeps them off the ring entirely.
        input.ignore(Ignore::All);

        let available = input.ports();
        if available.is_empty() {
            anyhow::bail!(no_ports_hint());
        }

        let port = match &source.id {
            Some(id) => {
                let raw = id.strip_prefix(ID_PREFIX).unwrap_or(id);
                input
                    .find_port_by_id(raw)
                    // Ids are stable but not eternal — a port recreated under
                    // the same name is a different id, and matching the name is
                    // what the user meant.
                    .or_else(|| {
                        available
                            .iter()
                            .find(|p| input.port_name(p).is_ok_and(|n| n == raw))
                            .cloned()
                    })
                    .with_context(|| {
                        format!("MIDI port '{raw}' is not available — unplugged, or closed?")
                    })?
            }
            None => available[0].clone(),
        };

        let port_name = input.port_name(&port).unwrap_or_else(|_| "<unknown>".into());

        let (mut producer, consumer) = HeapRb::<Message>::new(RING).split();
        let dropped = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&dropped);

        let connection = input
            .connect(
                &port,
                CLIENT,
                move |_timestamp, bytes, _| {
                    if let Some(msg) = parse(bytes) {
                        // A full ring means the engine has stalled for seconds.
                        // Counting is all that can be done here; blocking the
                        // driver's callback would be worse than losing a note.
                        if producer.try_push(msg).is_err() {
                            counter.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                },
                (),
            )
            .map_err(|e| anyhow!("{}: {e}", open_hint(&port_name)))?;

        Ok(Self {
            _connection: connection,
            consumer,
            source: Source { id: source.id.clone(), kind: SourceKind::Midi, channel: source.channel },
            port_name,
            dropped,
        })
    }

    pub fn port_name(&self) -> &str {
        &self.port_name
    }

    /// The selection as resolved, not as requested.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Adopt a selection that resolves to this same port — a channel change, or
    /// naming by id what was opened as "the first MIDI input". The connection is
    /// untouched; only what is reported about it moves.
    pub fn reselect(&mut self, source: &Source) {
        self.source =
            Source { id: source.id.clone(), kind: SourceKind::Midi, channel: source.channel };
    }

    /// Messages lost to a full ring. Nonzero means the engine stalled, not that
    /// anything is wrong with the port.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Drain queued messages into `out`, returning how many were written. Never
    /// blocks; returns 0 when nothing has arrived.
    pub fn read(&mut self, out: &mut [Message]) -> usize {
        self.consumer.pop_slice(out)
    }

    pub fn describe(&self) -> String {
        let channel = match self.source.channel {
            Some(c) => format!("channel {}", c + 1),
            None => "all channels".into(),
        };
        format!("{} [midi] ({channel})", self.port_name)
    }
}

/// Keep channel voice messages, drop everything else.
///
/// System messages (0xF0 and up) carry no channel and nothing here reacts to
/// them; most are already filtered by `Ignore::All`, and this is the backstop.
fn parse(bytes: &[u8]) -> Option<Message> {
    let &status = bytes.first()?;
    if !(0x80..0xF0).contains(&status) {
        return None;
    }
    Some(Message {
        status,
        data1: bytes.get(1).copied().unwrap_or(0) & 0x7F,
        data2: bytes.get(2).copied().unwrap_or(0) & 0x7F,
    })
}

/// The message for the case that is not a failure at all — nothing is plugged
/// in and nothing has been set up. This is where most people meet MIDI here, so
/// it is worth the paragraph.
pub fn no_ports_hint() -> String {
    "no MIDI inputs found. A keyboard appears here as soon as it is plugged in. \
     To capture FL Studio's MIDI Out plugin instead, install loopMIDI, create a \
     port in it, then enable that port under FL's Options -> MIDI settings -> \
     Output and set the MIDI Out plugin to its port number — Windows has no \
     built-in way to join a MIDI output to a MIDI input"
        .into()
}

/// The failure worth explaining, because it is the one that actually happens: a
/// MIDI input port on Windows admits one listener at a time.
fn open_hint(name: &str) -> String {
    format!(
        "could not open MIDI port '{name}'. Windows gives a MIDI input to one \
         application at a time — another one, or a second copy of this engine, \
         may already hold it"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_voice_messages_are_split_into_kind_and_channel() {
        let m = Message { status: 0x93, data1: 60, data2: 100 };
        assert_eq!(m.kind(), Message::NOTE_ON);
        assert_eq!(m.channel(), 3);
        assert!(m.is_note_on());

        let m = Message { status: 0x90, data1: 60, data2: 0 };
        assert!(!m.is_note_on(), "velocity 0 is a note-off");
    }

    #[test]
    fn parsing_keeps_voice_messages_and_drops_the_rest() {
        assert_eq!(
            parse(&[0x90, 60, 100]),
            Some(Message { status: 0x90, data1: 60, data2: 100 })
        );
        // Two-byte messages (program change, channel pressure) still parse.
        assert_eq!(parse(&[0xC0, 5]), Some(Message { status: 0xC0, data1: 5, data2: 0 }));

        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&[0xF8]), None, "timing clock is not a voice message");
        assert_eq!(parse(&[0xF0, 0x7E, 0xF7]), None, "sysex is not a voice message");
        assert_eq!(parse(&[60, 100]), None, "a data byte cannot start a message");
    }

    /// Data bytes are seven bits. A driver that ever handed over a high bit
    /// would otherwise index a note array out of range.
    #[test]
    fn data_bytes_are_masked_to_seven_bits() {
        let m = parse(&[0x90, 0xFF, 0xFF]).unwrap();
        assert_eq!(m.data1, 127);
        assert_eq!(m.data2, 127);
    }

    /// Enumeration runs against whatever is on the machine, so it can only
    /// assert invariants — but "every entry is selectable" is the one that
    /// matters, since the editor sends these ids straight back.
    #[test]
    fn enumerated_ports_are_well_formed() {
        for port in ports() {
            assert!(port.id.starts_with(ID_PREFIX), "'{}' is not tagged as MIDI", port.id);
            assert_eq!(port.kind, SourceKind::Midi);
            assert!(port.sample_rate.is_none());
        }
    }
}
