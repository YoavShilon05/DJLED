//! Colour: from a band level to the bytes on the wire.
//!
//! - [`oklab`] — the perceptual space interpolation happens in, the conversions
//!   either side of it, and what opacity means once colours are composited.
//! - [`surface`] — the 2D keyframe field over (strip position × intensity).
//! - [`intensity`] — level to brightness: threshold, clamp and the curve.
//! - [`strip`] — where each frequency lands on the wall: LED sectors, reverse
//!   and mirror.
//! - [`render`] — sampling the field at those points and quantising to bytes.

pub mod intensity;
pub mod oklab;
pub mod render;
pub mod strip;
pub mod surface;

pub use intensity::{IntensityConfig, IntensityCurve};
pub use oklab::{LinearRgb, Oklab};
pub use render::{Geometry, RenderConfig, Renderer};
pub use strip::{LayoutConfig, LedKeyframe, StripMap};
pub use surface::{ColorSurface, Keyframe, SurfaceConfig};
