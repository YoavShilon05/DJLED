//! What can be listened to, and the live thing listening to it.
//!
//! # One selection, three kinds
//!
//! Loopback, an input, and now a MIDI port are all *a source*: something that
//! produces levels over the editor's frequency axis. Keeping them in one
//! [`Source`] type rather than one per kind is what lets the dropdown, the
//! command-line lookup and the wire format stay single — the editor sends the
//! same message to move from a microphone to a keyboard as it does to move
//! between two microphones.
//!
//! [`Source::channel`] carries its weight in both worlds without meaning two
//! different things: it is "which channel of the many, or all of them", whether
//! those are the inputs of an interface or the sixteen channels of a MIDI
//! cable.
//!
//! # Why the analysis chain lives here too
//!
//! [`LiveSource`] holds the open device *and* the analyser that goes with it,
//! because the two are inseparable — an audio device without an STFT produces
//! nothing, and a MIDI port without note state produces nothing. What comes out
//! is the same pair in both cases: levels, and the frequencies they sit at.
//! Everything downstream reads that pair and never asks which kind produced it.
//!
//! The one thing a source switch can change that a config edit cannot is the
//! *shape* of that pair — 48 bands become 88 semitones — which is why opening a
//! source is the only operation here that makes the caller rebuild its
//! renderer.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::capture::Capture;
use crate::engine::{Engine, EngineConfig};
use crate::midi::{self, MidiListener, Message, NoteEngine};
use crate::show::ShowConfig;

/// How often the note engine advances its envelopes. There is no sample clock
/// in MIDI, so this is a wall-clock choice: 125 Hz is comfortably above the
/// 60 fps the strip is driven at, and the run loop already wakes more often
/// than this while idle.
const MIDI_FRAME: Duration = Duration::from_millis(8);

/// Longest step the note envelopes will take in one go. Without a cap, a
/// scheduling stall or a laptop resuming from sleep would collapse every
/// release into a single frame and snap the strip to black.
const MAX_MIDI_STEP: f32 = 0.1;

/// Which kind of endpoint is being listened to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    /// A render endpoint captured in loopback: the mix Windows is already
    /// sending to the speakers.
    #[default]
    Loopback,
    /// A capture endpoint: microphone, line input, interface inputs.
    Input,
    /// A MIDI input port: a controller, or a virtual cable carrying a DAW's
    /// note output.
    Midi,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Loopback => "loopback",
            SourceKind::Input => "input",
            SourceKind::Midi => "midi",
        }
    }

    pub fn is_audio(self) -> bool {
        !matches!(self, SourceKind::Midi)
    }
}

/// What the analyser should listen to.
///
/// Every field defaults, so an editor that predates one still produces a valid
/// selection rather than a parse error that would drop the whole command.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    /// Device or port id, stable across runs and reboots. `None` follows
    /// whatever counts as the default for `kind` — for audio, whatever Windows
    /// currently calls the default, which is what most people want for PC
    /// audio; for MIDI, the first port there is, since MIDI has no notion of a
    /// default device.
    pub id: Option<String>,
    /// Which default to follow when `id` is `None`. Ignored otherwise, because
    /// a named endpoint already determines how it has to be opened.
    pub kind: SourceKind,
    /// Channel to listen to, 0-based. `None` takes them all.
    ///
    /// For audio that is the channel to analyse, and averaging is what `None`
    /// means: a guitar in input 1 of a stereo interface exists on one channel
    /// only, and mixing it with a silent neighbour costs 6 dB and adds that
    /// neighbour's noise floor. For MIDI it is the channel to accept, and
    /// `None` merges all sixteen — which is what a keyboard wants, while a DAW
    /// sending several parts down one cable usually wants one.
    pub channel: Option<usize>,
}

impl Source {
    /// The system default output, captured in loopback — the historical
    /// behaviour, and the default at startup.
    pub fn default_output() -> Self {
        Self { id: None, kind: SourceKind::Loopback, channel: None }
    }

    /// The system default recording device.
    pub fn default_input() -> Self {
        Self { id: None, kind: SourceKind::Input, channel: None }
    }

    /// The first MIDI input port there is.
    pub fn default_midi() -> Self {
        Self { id: None, kind: SourceKind::Midi, channel: None }
    }

    /// Look a source up the way a human would name it on the command line: a
    /// full id, or any case-insensitive fragment of a device name, optionally
    /// narrowed to one kind.
    ///
    /// Ambiguity is an error listing the candidates rather than a guess — and
    /// it is not a corner case. An interface presents its playback and capture
    /// halves under one name, so "UR22" genuinely means two different things,
    /// which is exactly the distinction the caller is here to make.
    pub fn find(spec: &str, kind: Option<SourceKind>) -> Result<Self> {
        let available: Vec<DeviceInfo> =
            devices().into_iter().filter(|d| kind.is_none_or(|k| d.kind == k)).collect();

        if let Some(exact) = available.iter().find(|d| d.id == spec) {
            return Ok(Self { id: Some(exact.id.clone()), kind: exact.kind, channel: None });
        }

        let needle = spec.to_lowercase();
        let matches: Vec<&DeviceInfo> =
            available.iter().filter(|d| d.name.to_lowercase().contains(&needle)).collect();

        match matches.as_slice() {
            [one] => Ok(Self { id: Some(one.id.clone()), kind: one.kind, channel: None }),
            [] => {
                let scope = match kind {
                    Some(k) => format!(" {} device", k.label()),
                    None => " device".into(),
                };
                anyhow::bail!(
                    "no{scope} matches '{spec}'. Run with --list-devices to see them all"
                )
            }
            many => {
                let names: Vec<String> =
                    many.iter().map(|d| format!("{} ({})", d.name, d.kind.label())).collect();
                let hint = if many.iter().any(|d| d.kind == SourceKind::Input) {
                    ". Add --input to mean the capture side, or pass the id from --list-devices"
                } else {
                    ". Pass the id from --list-devices to be exact"
                };
                anyhow::bail!("'{spec}' matches {}: {}{hint}", many.len(), names.join(", "))
            }
        }
    }
}

/// One selectable endpoint, as the editor's dropdown sees it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    /// True for the endpoint Windows currently defaults to for this kind.
    /// Never true for MIDI, which has no such notion.
    pub is_default: bool,
    /// From the endpoint's mix format. `None` for MIDI, which has no sample
    /// rate at all, and for an audio endpoint that refused to be opened for a
    /// look — typically because something else holds it. That is worth showing
    /// rather than hiding the device, since the reason is usually exactly what
    /// the user is trying to work around.
    pub sample_rate: Option<f64>,
    pub channels: Option<usize>,
}

/// Everything that can be listened to right now.
///
/// Ordered the way the editor lists it: loopback, then inputs, then MIDI, with
/// the system default first within each group and the rest by name. Audio
/// enumeration costs one mix format query per endpoint, so this is for startup
/// and explicit refreshes, not for the frame loop.
pub fn devices() -> Vec<DeviceInfo> {
    let mut out = crate::capture::endpoints();
    out.extend(midi::ports());
    out.sort_by(|a, b| {
        (a.kind as u8, !a.is_default, a.name.to_lowercase()).cmp(&(
            b.kind as u8,
            !b.is_default,
            b.name.to_lowercase(),
        ))
    });
    out
}

/// An open source and the analyser that turns it into levels.
///
/// The variants differ in size by about half a kilobyte — the note engine
/// carries an envelope per MIDI note — which is not worth a box. Exactly one of
/// these exists, it is moved only when the source changes, and boxing would put
/// a pointer chase on `levels` and `centers`, which are read every frame.
#[allow(clippy::large_enum_variant)]
pub enum LiveSource {
    Audio(AudioChain),
    Midi(MidiChain),
}

pub struct AudioChain {
    capture: Capture,
    engine: Engine,
    cfg: EngineConfig,
    buf: Vec<f32>,
    /// How much of `buf` the last poll filled. Kept only so `--probe` can
    /// report what actually arrived from the device, which is a different
    /// question from what came out of the analyser: a stream delivering half
    /// the samples it should still produces frames.
    filled: usize,
}

pub struct MidiChain {
    listener: MidiListener,
    notes: NoteEngine,
    buf: Vec<Message>,
    last: Instant,
}

impl LiveSource {
    /// Open a source and build its analyser.
    ///
    /// `base` supplies everything about the audio analyser that is not in the
    /// show — the band count, chiefly — and is ignored for MIDI, which has no
    /// band plan to configure.
    pub fn open(
        source: &Source,
        base: &EngineConfig,
        show: &ShowConfig,
        buffer_secs: f32,
    ) -> Result<Self> {
        if source.kind == SourceKind::Midi {
            let listener = MidiListener::open(source)?;
            let mut notes = NoteEngine::new(&show.midi, source.channel, show.decay());
            notes.set_eq(&show.eq);
            return Ok(Self::Midi(MidiChain {
                listener,
                notes,
                buf: vec![Message::default(); 256],
                last: Instant::now(),
            }));
        }

        let capture = Capture::open(source, buffer_secs)?;
        let cfg = EngineConfig { hop: show.hop(), ..base.clone() };
        let mut engine = Engine::new(&cfg, capture.sample_rate());
        engine.set_eq(&show.eq);
        engine.set_decay(show.decay());
        Ok(Self::Audio(AudioChain { capture, engine, cfg, buf: vec![0.0; 8192], filled: 0 }))
    }

    /// Bar brightness, 0..1, one per point of [`Self::centers`].
    pub fn levels(&self) -> &[f32] {
        match self {
            Self::Audio(a) => a.engine.levels(),
            Self::Midi(m) => m.notes.levels(),
        }
    }

    /// Where those levels sit on the editor's frequency axis.
    ///
    /// For audio these are band centre frequencies. For MIDI they are the
    /// stretched positions of the notes — see [`crate::midi::notes`].
    pub fn centers(&self) -> &[f32] {
        match self {
            Self::Audio(a) => a.engine.centers(),
            Self::Midi(m) => m.notes.centers(),
        }
    }

    /// Colours this source would put on the wire, before any link says how many
    /// it will take.
    ///
    /// One per band for audio, one per semitone for MIDI. The caller narrows
    /// this to what its destination accepts — see [`crate::link::Link::max_points`]
    /// — because the renderer resamples by frequency, so sending fewer points
    /// costs spatial resolution on the strip and nothing else, while sending
    /// more costs the entire frame.
    pub fn points(&self) -> usize {
        match self {
            Self::Audio(a) => a.engine.band_count(),
            Self::Midi(m) => m.notes.centers().len(),
        }
    }

    /// The selection as resolved, not as requested.
    pub fn source(&self) -> &Source {
        match self {
            Self::Audio(a) => a.capture.source(),
            Self::Midi(m) => m.listener.source(),
        }
    }

    pub fn kind(&self) -> SourceKind {
        match self {
            Self::Audio(a) => a.capture.kind(),
            Self::Midi(_) => SourceKind::Midi,
        }
    }

    pub fn device_name(&self) -> &str {
        match self {
            Self::Audio(a) => a.capture.device_name(),
            Self::Midi(m) => m.listener.port_name(),
        }
    }

    /// Channels the source offers, which is what bounds
    /// [`Source::channel`].
    pub fn channels(&self) -> usize {
        match self {
            Self::Audio(a) => a.capture.channels(),
            Self::Midi(_) => midi::CHANNELS,
        }
    }

    /// Zero for MIDI, which has no sample rate. The editor shows a dash rather
    /// than pretending otherwise.
    pub fn sample_rate(&self) -> f64 {
        match self {
            Self::Audio(a) => a.capture.sample_rate(),
            Self::Midi(_) => 0.0,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Audio(a) => a.capture.describe(),
            Self::Midi(m) => m.listener.describe(),
        }
    }

    /// Notes seen outside the configured range, or `None` for an audio source.
    pub fn out_of_range(&self) -> Option<u32> {
        match self {
            Self::Audio(_) => None,
            Self::Midi(m) => Some(m.notes.out_of_range()),
        }
    }

    /// Take the reason the source died, if it has. Reported once: a dead stream
    /// stays dead, and repeating it every frame would bury everything else.
    pub fn take_fault(&self) -> Option<String> {
        match self {
            Self::Audio(a) => a.capture.take_fault(),
            // A MIDI port has no error callback to fail through. Losing one is
            // silent by nature — the port simply stops delivering — and there
            // is nothing to poll that would say so.
            Self::Midi(_) => None,
        }
    }

    /// Take in whatever has arrived and advance the analyser.
    ///
    /// Returns true when a new frame is ready and [`Self::levels`] has changed.
    /// False means there was nothing to do yet, not that anything is wrong: a
    /// loopback endpoint delivers nothing at all while it is idle, and MIDI
    /// only steps its envelopes on its own frame boundary.
    pub fn poll(&mut self) -> bool {
        match self {
            Self::Audio(a) => {
                a.filled = a.capture.read(&mut a.buf);
                a.filled > 0 && a.engine.push(&a.buf[..a.filled])
            }
            Self::Midi(m) => {
                // Messages are drained every pass, not only on frame
                // boundaries, so a note-on is never held back behind the
                // envelope clock.
                loop {
                    let n = m.listener.read(&mut m.buf);
                    for i in 0..n {
                        m.notes.handle(m.buf[i]);
                    }
                    if n < m.buf.len() {
                        break;
                    }
                }

                let elapsed = m.last.elapsed();
                if elapsed < MIDI_FRAME {
                    return false;
                }
                m.last = Instant::now();
                m.notes.advance(elapsed.as_secs_f32().min(MAX_MIDI_STEP));
                true
            }
        }
    }

    /// Push a configuration into the stages it touches.
    ///
    /// Everything is compared against what is already in force, because the
    /// expensive cases must not fire on every keyframe drag: resampling the EQ
    /// walks every band, and a hop change rebuilds the analyser outright —
    /// which resets the floor tracker and the ballistics, so it is worth a
    /// visible hiccup only when the user actually asked for it.
    ///
    /// Returns true when the level grid changed shape, which is what tells the
    /// caller to rebuild its renderer.
    pub fn apply(&mut self, next: &ShowConfig, current: &ShowConfig, force: bool) -> bool {
        match self {
            Self::Audio(a) => {
                if force || next.hop() != current.hop() {
                    a.cfg.hop = next.hop();
                    a.engine = Engine::new(&a.cfg, a.capture.sample_rate());
                    // A rebuilt analyser has no EQ or ballistics, so both are
                    // reinstalled regardless of whether they were what changed.
                    a.engine.set_eq(&next.eq);
                    a.engine.set_decay(next.decay());
                } else {
                    if !eq_matches(&next.eq, &current.eq) {
                        a.engine.set_eq(&next.eq);
                    }
                    if next.decay() != current.decay() {
                        a.engine.set_decay(next.decay());
                    }
                }
                // The band plan comes from the sample rate and the band count,
                // neither of which is in the show, so the grid never moves here.
                false
            }
            Self::Midi(m) => {
                let moved = force || next.midi != current.midi;
                let moved = moved && m.notes.set_config(&next.midi);
                if force || !eq_matches(&next.eq, &current.eq) || moved {
                    m.notes.set_eq(&next.eq);
                }
                if force || next.decay() != current.decay() {
                    m.notes.set_decay(next.decay());
                }
                moved
            }
        }
    }

    /// Start again from silence. For a new source, where carrying the old
    /// floor tracker, ballistics or held notes across would spend the first
    /// second unwinding a level that no longer exists.
    pub fn reset(&mut self) {
        match self {
            Self::Audio(a) => a.engine.reset(),
            Self::Midi(m) => {
                m.notes.reset();
                m.last = Instant::now();
            }
        }
    }

    /// Adopt a selection that resolves to what is already open, without
    /// reopening anything. Returns false when the request genuinely needs a new
    /// source.
    ///
    /// This is not an optimisation. Windows gives a MIDI input port to one
    /// application at a time, and that application is this one — so reopening
    /// the port to change which channel is being *filtered for* would be
    /// refused by the handle this process is holding, and the user would be told
    /// something else has the port. An audio channel is chosen when the stream
    /// is built and genuinely does need a new one, so this only ever applies to
    /// MIDI.
    pub fn retune(&mut self, source: &Source) -> bool {
        let Self::Midi(m) = self else { return false };
        if source.kind != SourceKind::Midi {
            return false;
        }
        // By name, not by id: "the first MIDI input" and that port's id are two
        // selections that mean the same port, and moving between them must not
        // count as a switch.
        if midi::resolve_name(source).as_deref() != Some(m.listener.port_name()) {
            return false;
        }

        m.listener.reselect(source);
        m.notes.set_channel(source.channel);
        true
    }

    /// The samples the last [`Self::poll`] took in, for the diagnostic that
    /// needs to distinguish "the device is not delivering" from "the analyser
    /// has nothing to show". Always empty for MIDI, which has no samples.
    pub fn last_block(&self) -> &[f32] {
        match self {
            Self::Audio(a) => &a.buf[..a.filled],
            Self::Midi(_) => &[],
        }
    }

    /// The audio analyser, for the diagnostics that only apply to one.
    pub fn engine(&self) -> Option<&Engine> {
        match self {
            Self::Audio(a) => Some(&a.engine),
            Self::Midi(_) => None,
        }
    }
}

/// Structural comparison; `EqBand` is not `PartialEq` because it holds floats
/// and an exact-equality derive on those would be a trap elsewhere.
fn eq_matches(a: &[crate::dsp::eq::EqBand], b: &[crate::dsp::eq::EqBand]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.kind == y.kind && x.hz == y.hz && x.gain == y.gain && x.q == y.q
        })
}

/// Whether an id names a MIDI port rather than an audio endpoint. Lives here
/// so the one place that knows ids are opaque strings is the one that made
/// them opaque.
pub(crate) fn is_midi_id(id: &str) -> bool {
    id.starts_with("midi:")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    /// The wire format the editor sends. Every field is optional, and an empty
    /// object has to mean the historical behaviour — default output, mixed down
    /// — or a rolled-back editor would silently change what is captured.
    #[test]
    fn a_source_parses_from_the_ui_wire_format() {
        let s: Source = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Source::default_output());

        let s: Source = serde_json::from_str(r#"{"kind":"input"}"#).unwrap();
        assert_eq!(s, Source::default_input());

        let s: Source = serde_json::from_str(r#"{"kind":"midi"}"#).unwrap();
        assert_eq!(s, Source::default_midi());

        let s: Source =
            serde_json::from_str(r#"{"id":"wasapi:{0.0.1.00000000}","kind":"input","channel":0}"#)
                .unwrap();
        assert_eq!(s.id.as_deref(), Some("wasapi:{0.0.1.00000000}"));
        assert_eq!(s.kind, SourceKind::Input);
        assert_eq!(s.channel, Some(0));
    }

    #[test]
    fn a_source_survives_a_round_trip() {
        for original in [
            Source { id: Some("wasapi:x".into()), kind: SourceKind::Input, channel: Some(1) },
            Source { id: Some("midi:x".into()), kind: SourceKind::Midi, channel: Some(15) },
        ] {
            let json = serde_json::to_string(&original).unwrap();
            assert_eq!(serde_json::from_str::<Source>(&json).unwrap(), original);
        }
    }

    /// Device ids contain braces, dots and colons; they travel as opaque
    /// strings and must come back byte-identical or the lookup fails.
    #[test]
    fn device_ids_survive_json() {
        let id = "wasapi:{0.0.0.00000000}.{a1b2c3d4-0000-0000-0000-000000000000}";
        let s = Source { id: Some(id.into()), ..Source::default_output() };
        let back: Source = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.id.as_deref(), Some(id));
    }

    /// A MIDI id must never be mistaken for a cpal one, or a MIDI selection
    /// would be handed to the audio host to choke on.
    #[test]
    fn midi_ids_are_distinguishable_from_audio_ids() {
        for device in devices() {
            assert_eq!(
                is_midi_id(&device.id),
                device.kind == SourceKind::Midi,
                "'{}' is tagged wrongly for a {:?}",
                device.id,
                device.kind
            );
        }
    }

    /// Enumeration runs against whatever hardware the machine has, so it can
    /// only assert invariants — but "every entry is selectable" is the one that
    /// matters, since the editor sends these ids straight back.
    #[test]
    fn enumerated_devices_are_well_formed() {
        for device in devices() {
            assert!(!device.id.is_empty(), "'{}' has no id to select it by", device.name);
            if device.kind.is_audio() {
                assert!(
                    cpal::DeviceId::from_str(&device.id).is_ok(),
                    "id '{}' cannot be parsed back",
                    device.id
                );
            }
            assert!(device.sample_rate.is_none_or(|r| r > 0.0));
            assert!(device.channels.is_none_or(|c| c > 0));
        }
    }

    /// At most one default per direction, or the editor's dropdown would show
    /// two entries both claiming to be the one in use.
    #[test]
    fn at_most_one_default_per_kind() {
        for kind in [SourceKind::Loopback, SourceKind::Input, SourceKind::Midi] {
            let defaults = devices().iter().filter(|d| d.kind == kind && d.is_default).count();
            assert!(defaults <= 1, "{defaults} devices claim to be the default {kind:?}");
        }
    }

    #[test]
    fn an_unknown_name_is_an_error_not_a_guess() {
        let err = Source::find("no such device anywhere", None).unwrap_err().to_string();
        assert!(err.contains("--list-devices"), "unhelpful error: {err}");
    }

    /// The interesting lookup: an interface names its playback and capture
    /// halves identically, so the fragment that matches both must resolve once
    /// the direction is given. Skipped where the machine has no such pair.
    #[test]
    fn a_direction_resolves_a_name_shared_by_both_halves() {
        let all = devices();
        let Some(shared) = all
            .iter()
            .find(|d| d.kind == SourceKind::Loopback)
            .filter(|d| all.iter().any(|o| o.kind == SourceKind::Input && o.name == d.name))
        else {
            return;
        };

        assert!(
            Source::find(&shared.name, None).is_err(),
            "'{}' names two endpoints and should not resolve without a direction",
            shared.name
        );

        let found = Source::find(&shared.name, Some(SourceKind::Input)).unwrap();
        assert_eq!(found.kind, SourceKind::Input);
        assert!(all.iter().any(|d| Some(&d.id) == found.id.as_ref() && d.kind == SourceKind::Input));
    }
}
 