//! Audio capture — either what the PC is playing, or what an interface hears.
//!
//! # Two ways in
//!
//! **Loopback** taps a *render* endpoint. No cable, no Stereo Mix driver, no
//! microphone: cpal enables it transparently when an input stream is built on an
//! output device (it sets `AUDCLNT_STREAMFLAGS_LOOPBACK` for any device whose
//! data flow is `eRender`), so capture is bit-exact and needs no extra hardware.
//! This is the right source for music the PC is playing.
//!
//! **Input** taps a *capture* endpoint — a microphone, a line input, the inputs
//! of an audio interface. This is the right source for an instrument, and it is
//! the only one that works when a DAW owns the interface directly: an ASIO
//! client bypasses the render endpoint entirely, so there is nothing on the
//! loopback side to hear.
//!
//! A WASAPI endpoint is one or the other, never both — an interface shows up as
//! a separate render endpoint and capture endpoint. So naming a device is enough
//! to determine how it has to be opened; [`SourceKind`] only picks *which*
//! default to follow when no device is named.
//!
//! # Latency
//!
//! cpal documents that the callback period is always the device period from
//! `GetDevicePeriod()` regardless of the requested buffer size — the request
//! only affects ring-buffer depth. That pins this stage at the shared-mode
//! default of roughly 10 ms. Reaching the ~3 ms that `IAudioClient3`'s
//! `InitializeSharedAudioStream` allows would mean dropping to the raw `wasapi`
//! crate. Not worth it yet: 10 ms is a third of the treble path's total budget
//! and well under the LED link's own frame time.
//!
//! # Threading
//!
//! The audio callback does the minimum possible — reduce to mono and push into a
//! lock-free SPSC ring. No allocation, no locking, no logging. Everything else
//! happens on the thread draining the ring. The one lock in this module is in
//! the *error* callback, which fires at most once per stream failure.

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use crate::source::{is_midi_id, DeviceInfo, Source, SourceKind};

/// Audio endpoints only. MIDI ports are enumerated by [`crate::midi::ports`],
/// and [`crate::source::devices`] is what joins the two lists into the one the
/// editor's dropdown shows.
///
/// Ordered the way the editor lists it: loopback before input, the system
/// default first within each group, then by name. Enumeration costs one mix
/// format query per endpoint, so this is for startup and explicit refreshes,
/// not for the frame loop.
pub fn endpoints() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let default_output = host.default_output_device().and_then(|d| d.id().ok());
    let default_input = host.default_input_device().and_then(|d| d.id().ok());

    let Ok(found) = host.devices() else { return Vec::new() };

    let mut out: Vec<DeviceInfo> = found
        .filter_map(|device| {
            let id = device.id().ok()?;
            let kind = direction(&device)?;
            let config = default_config(&device, kind).ok();
            let default = match kind {
                SourceKind::Loopback => &default_output,
                SourceKind::Input => &default_input,
                // Unreachable: `direction` only ever names an audio endpoint.
                SourceKind::Midi => &None,
            };

            Some(DeviceInfo {
                is_default: default.as_ref() == Some(&id),
                id: id.to_string(),
                name: describe(&device),
                kind,
                sample_rate: config.as_ref().map(|c| c.sample_rate() as f64),
                channels: config.as_ref().map(|c| c.channels() as usize),
            })
        })
        .collect();

    out.sort_by(|a, b| {
        (a.kind as u8, !a.is_default, a.name.to_lowercase()).cmp(&(
            b.kind as u8,
            !b.is_default,
            b.name.to_lowercase(),
        ))
    });
    out
}

pub struct Capture {
    /// Held to keep the stream alive; dropping it stops capture.
    _stream: cpal::Stream,
    consumer: HeapCons<f32>,
    /// The selection as it was actually resolved: the direction settled, and the
    /// channel dropped if the device turned out not to have it. What the UI
    /// shows must be what is running.
    ///
    /// A request for a default keeps `id: None` rather than being pinned to the
    /// endpoint it happened to resolve to, because "follow the default" is the
    /// selection — swapping headphones should not silently strand it.
    source: Source,
    device_name: String,
    kind: SourceKind,
    sample_rate: f64,
    channels: usize,
    fault: Arc<Mutex<Option<String>>>,
}

impl Capture {
    /// Open a source. `buffer_secs` sizes the ring between the audio callback
    /// and the analysis thread; it only needs to cover scheduling jitter, and a
    /// quarter second is generous while still bounding how far behind the
    /// display can fall.
    pub fn open(source: &Source, buffer_secs: f32) -> Result<Self> {
        let host = cpal::default_host();
        let (device, kind) = resolve(&host, source)?;
        let device_name = describe(&device);

        // Loopback delivers the render mix format, so an output endpoint is
        // asked for its *output* config; its input config list describes a
        // microphone side that does not exist.
        let supported = default_config(&device, kind).with_context(|| {
            format!("no default {} config for '{device_name}'", kind.label())
        })?;

        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let sample_rate = config.sample_rate as f64;

        // A channel that no longer exists (the selection outlived the device it
        // was made for) falls back to the mix rather than refusing to open —
        // silence is a worse answer than a slightly wrong one, and `source`
        // reports what actually happened.
        let channel = source.channel.filter(|&c| c < channels);

        let capacity = ((sample_rate * buffer_secs as f64) as usize).max(4096);
        let (producer, consumer) = HeapRb::<f32>::new(capacity).split();

        let fault = Arc::new(Mutex::new(None));
        let stream = match sample_format {
            SampleFormat::F32 => build::<f32>(&device, &config, channels, channel, producer, &fault),
            SampleFormat::I16 => build::<i16>(&device, &config, channels, channel, producer, &fault),
            SampleFormat::U16 => build::<u16>(&device, &config, channels, channel, producer, &fault),
            SampleFormat::I32 => build::<i32>(&device, &config, channels, channel, producer, &fault),
            other => anyhow::bail!("unsupported sample format {other:?}"),
        }
        .with_context(|| open_hint(&device_name, kind))?;

        stream
            .play()
            .with_context(|| format!("failed to start the stream on '{device_name}'"))?;

        Ok(Self {
            _stream: stream,
            consumer,
            source: Source { id: source.id.clone(), kind, channel },
            device_name,
            kind,
            sample_rate,
            channels,
            fault,
        })
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn kind(&self) -> SourceKind {
        self.kind
    }

    /// The selection as resolved, not as requested.
    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn describe(&self) -> String {
        let channel = match self.source.channel {
            Some(c) => format!(", ch {}", c + 1),
            None => String::new(),
        };
        format!(
            "{} [{}] ({} ch{channel} @ {:.0} Hz)",
            self.device_name,
            self.kind.label(),
            self.channels,
            self.sample_rate
        )
    }

    /// Take the reason the stream died, if it has. Reported once: a dead stream
    /// stays dead, and repeating it every frame would bury everything else.
    ///
    /// Worth polling. A stream can be killed out from under the process — a DAW
    /// taking the interface exclusively, or the device being unplugged — and
    /// without this the only symptom is bars that quietly stop moving.
    pub fn take_fault(&self) -> Option<String> {
        self.fault.lock().ok().and_then(|mut f| f.take())
    }

    /// Drain available mono samples into `out`, returning how many were written.
    /// Never blocks; returns 0 when nothing is queued.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        self.consumer.pop_slice(out)
    }
}

/// Which way an endpoint faces, or `None` for one that does neither and
/// therefore cannot be captured at all.
fn direction(device: &cpal::Device) -> Option<SourceKind> {
    if device.supports_output() {
        Some(SourceKind::Loopback)
    } else if device.supports_input() {
        Some(SourceKind::Input)
    } else {
        None
    }
}

fn default_config(
    device: &cpal::Device,
    kind: SourceKind,
) -> Result<cpal::SupportedStreamConfig, cpal::Error> {
    match kind {
        SourceKind::Loopback => device.default_output_config(),
        // A MIDI selection never reaches here — `Capture::open` rejects it
        // first — and asking cpal for an input config is the harmless answer
        // if that ever stops being true.
        SourceKind::Input | SourceKind::Midi => device.default_input_config(),
    }
}

fn describe(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "<unknown>".into())
}

fn resolve(host: &cpal::Host, source: &Source) -> Result<(cpal::Device, SourceKind)> {
    // The one selection this module cannot serve. Caught here rather than left
    // to cpal, which would report an unparseable device id and send whoever
    // reads it looking in the wrong place entirely.
    if source.kind == SourceKind::Midi || source.id.as_deref().is_some_and(is_midi_id) {
        anyhow::bail!("'{}' is a MIDI port, not an audio device", source.id.as_deref().unwrap_or("midi"));
    }

    let Some(id) = &source.id else {
        return match source.kind {
            SourceKind::Loopback => host
                .default_output_device()
                .map(|d| (d, SourceKind::Loopback))
                .context("no default output device — is anything configured for playback?"),
            _ => host
                .default_input_device()
                .map(|d| (d, SourceKind::Input))
                .context("no default recording device — is anything configured for input?"),
        };
    };

    let parsed = cpal::DeviceId::from_str(id)
        .with_context(|| format!("'{id}' is not a device id"))?;
    let device = host
        .device_by_id(&parsed)
        .with_context(|| format!("device '{id}' is not available — unplugged, or disabled?"))?;
    let kind = direction(&device)
        .with_context(|| format!("device '{id}' offers neither capture nor playback"))?;
    Ok((device, kind))
}

/// The failure worth explaining, because it is the one that actually happens:
/// an endpoint the process cannot open is nearly always one something else has
/// taken.
fn open_hint(name: &str, kind: SourceKind) -> String {
    match kind {
        SourceKind::Loopback => format!(
            "failed to open '{name}' for loopback. If a DAW is driving this interface over \
             ASIO it owns the device outright and there is no render mix to tap — pick the \
             interface's *input* instead"
        ),
        // MIDI never reaches here; `resolve` refuses it by name first.
        _ => format!(
            "failed to open '{name}' for input. Another application may hold it exclusively, \
             or microphone access may be blocked in Windows privacy settings"
        ),
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    channel: Option<usize>,
    mut producer: HeapProd<f32>,
    fault: &Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + Sample,
    f32: FromSample<T>,
{
    let inv = 1.0 / channels as f32;
    let mut mono = Vec::new();
    let fault = Arc::clone(fault);

    device.build_input_stream::<T, _, _>(
        *config,
        move |data, _| {
            // Reduce to mono in a reusable buffer. This allocates only on the
            // first callback, and on any later callback larger than every
            // previous one — in practice, once.
            mono.clear();
            mono.reserve(data.len() / channels);

            // Branch once per callback, not once per frame.
            match channel {
                Some(ch) => {
                    for frame in data.chunks_exact(channels) {
                        mono.push(f32::from_sample(frame[ch]));
                    }
                }
                None => {
                    for frame in data.chunks_exact(channels) {
                        let sum: f32 = frame.iter().map(|&s| f32::from_sample(s)).sum();
                        mono.push(sum * inv);
                    }
                }
            }

            // A full ring means the analysis thread has stalled. Dropping the
            // newest samples is the right failure mode: the alternative is
            // blocking the audio callback, which would glitch playback itself.
            producer.push_slice(&mono);
        },
        // Not the data callback: this fires once, when the stream is already
        // finished, so a lock and an allocation here cost nothing real. Printing
        // instead would tear the terminal display.
        move |err| {
            if let Ok(mut slot) = fault.lock() {
                slot.get_or_insert_with(|| err.to_string());
            }
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Enumeration runs against whatever hardware the machine has, so it can
    /// only assert invariants — but "every endpoint faces one way" is the one
    /// that matters here, since the direction is what decides how it is opened.
    /// The wider checks live in [`crate::source`], with the types.
    #[test]
    fn every_endpoint_is_audio_and_faces_one_way() {
        for device in endpoints() {
            assert!(device.kind.is_audio(), "'{}' is not an audio endpoint", device.name);
            assert!(!device.id.is_empty(), "'{}' has no id to select it by", device.name);
        }
    }

    /// A MIDI selection must be turned away with a message that names the real
    /// problem, not left to fail as an unparseable device id.
    #[test]
    fn a_midi_selection_is_refused_by_name() {
        let source = Source { id: Some("midi:x".into()), kind: SourceKind::Midi, channel: None };
        let err = match Capture::open(&source, 0.25) {
            Ok(_) => panic!("a MIDI port must not open as an audio device"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("MIDI"), "unhelpful error: {err}");
    }
}
