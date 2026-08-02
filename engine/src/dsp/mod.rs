//! Real-time spectrum analysis for the LED wall.
//!
//! The chain, in order, with the reason each stage exists:
//!
//! 1. [`dcblock`] — removes DC offset, the classic cause of phantom sub-bass bars.
//! 2. [`window`] — Blackman-Harris. Stops neighbouring bars bleeding together.
//! 3. [`bands`] — Bark-like layout, and picks an FFT size per band.
//! 4. [`mrstft`] — runs the tier ladder, produces per-band magnitudes.
//! 5. [`fastpath`] — gated parallel path, built on [`biquad`], so bass attacks
//!    are not stuck behind a 171 ms window.
//! 6. [`post`] — floor tracking, dB mapping, AGC, ballistics, smoothing.

pub mod bands;
pub mod biquad;
pub mod dcblock;
pub mod eq;
pub mod fastpath;
pub mod mrstft;
pub mod post;
pub mod window;
