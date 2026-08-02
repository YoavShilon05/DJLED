//! Colour: from a band level to the bytes on the wire.
//!
//! - [`oklab`] — the perceptual space interpolation happens in, and the
//!   conversions either side of it.
//! - [`surface`] — the 2D keyframe field over (strip position × intensity).
//! - [`render`] — sampling that field per band and quantising to LED bytes.

pub mod oklab;
pub mod render;
pub mod surface;

pub use oklab::{LinearRgb, Oklab};
pub use render::{RenderConfig, Renderer};
pub use surface::{ColorSurface, Keyframe, SurfaceConfig};
