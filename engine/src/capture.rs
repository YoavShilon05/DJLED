//! WASAPI loopback capture — the audio the PC is already playing.
//!
//! No cable, no Stereo Mix driver, no microphone. cpal enables loopback
//! transparently when an *input* stream is built on an *output* device (it sets
//! `AUDCLNT_STREAMFLAGS_LOOPBACK` for any device whose data flow is `eRender`),
//! so capture is bit-exact and needs no extra hardware.
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
//! The audio callback does the minimum possible — downmix to mono and push into
//! a lock-free SPSC ring. No allocation, no locking, no logging. Everything else
//! happens on the thread draining the ring.

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

pub struct LoopbackCapture {
    /// Held to keep the stream alive; dropping it stops capture.
    _stream: cpal::Stream,
    consumer: HeapCons<f32>,
    sample_rate: f64,
    device_name: String,
    channels: usize,
}

impl LoopbackCapture {
    /// Open the default output device in loopback mode.
    ///
    /// `buffer_secs` sizes the ring between the audio callback and the analysis
    /// thread. It only needs to cover scheduling jitter; a quarter second is
    /// generous and still bounds how far behind the display can fall.
    pub fn new(buffer_secs: f32) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no default output device — is anything configured for playback?")?;
        let device_name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "<unknown>".into());

        // Loopback delivers the render mix format, so the output config is the
        // one to ask for; the input config list describes the microphone side.
        let supported = device
            .default_output_config()
            .with_context(|| format!("no default output config for '{device_name}'"))?;

        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let sample_rate = config.sample_rate as f64;

        let capacity = ((sample_rate * buffer_secs as f64) as usize).max(4096);
        let (producer, consumer) = HeapRb::<f32>::new(capacity).split();

        let stream = match sample_format {
            SampleFormat::F32 => build::<f32>(&device, &config, channels, producer),
            SampleFormat::I16 => build::<i16>(&device, &config, channels, producer),
            SampleFormat::U16 => build::<u16>(&device, &config, channels, producer),
            SampleFormat::I32 => build::<i32>(&device, &config, channels, producer),
            other => anyhow::bail!("unsupported loopback sample format {other:?}"),
        }
        .with_context(|| format!("failed to open loopback stream on '{device_name}'"))?;

        stream.play().context("failed to start the loopback stream")?;

        Ok(Self {
            _stream: stream,
            consumer,
            sample_rate,
            device_name,
            channels,
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

    /// Drain available mono samples into `out`, returning how many were written.
    /// Never blocks; returns 0 when nothing is queued.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        self.consumer.pop_slice(out)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    mut producer: HeapProd<f32>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + Sample,
    f32: FromSample<T>,
{
    let inv = 1.0 / channels as f32;
    let mut mono = Vec::new();

    device.build_input_stream::<T, _, _>(
        *config,
        move |data, _| {
            // Downmix in place into a reusable buffer. This allocates only on
            // the first callback, and on any later callback that is larger than
            // every previous one — in practice, once.
            mono.clear();
            mono.reserve(data.len() / channels);
            for frame in data.chunks_exact(channels) {
                let sum: f32 = frame.iter().map(|&s| f32::from_sample(s)).sum();
                mono.push(sum * inv);
            }

            // A full ring means the analysis thread has stalled. Dropping the
            // newest samples is the right failure mode: the alternative is
            // blocking the audio callback, which would glitch playback itself.
            producer.push_slice(&mono);
        },
        |err| eprintln!("loopback stream error: {err}"),
        None,
    )
}
