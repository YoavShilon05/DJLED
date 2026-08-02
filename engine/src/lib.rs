//! DJLED engine — audio capture, spectrum analysis and colour mapping for an
//! audio-reactive LED wall.
//!
//! All signal processing happens here on the PC; the Arduino downstream is a
//! dumb pixel expander. See the project plan for the rationale.

pub mod capture;
pub mod color;
pub mod dsp;
pub mod engine;
pub mod link;
pub mod show;
pub mod ui;

pub use engine::{Engine, EngineConfig};
pub use show::ShowConfig;
