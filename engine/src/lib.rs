//! DJLED engine — capture, analysis and colour mapping for an audio-reactive
//! LED wall.
//!
//! Two kinds of source drive it: audio, analysed into frequency bands, and
//! MIDI, whose notes land on the same frequency axis with velocity for level.
//! Everything downstream of [`source::LiveSource`] reads levels over that axis
//! and never asks which kind produced them.
//!
//! All signal processing happens here on the PC; the Arduino downstream is a
//! dumb pixel expander. See the project plan for the rationale.

pub mod capture;
pub mod color;
pub mod dsp;
pub mod engine;
pub mod link;
pub mod midi;
pub mod show;
pub mod source;
pub mod ui;

pub use engine::{Engine, EngineConfig};
pub use midi::MidiConfig;
pub use show::ShowConfig;
pub use source::{LiveSource, Source, SourceKind};
