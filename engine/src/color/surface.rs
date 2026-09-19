//! The 2D keyframe colour surface.
//!
//! Colour is authored as a field over the unit square: **x** is position along
//! the strip (bass on the left, treble on the right) and **y** is that band's
//! intensity. Keyframes are placed anywhere in that square, and any point
//! between them is interpolated.
//!
//! # Choice of interpolator
//!
//! Scattered 2D interpolation has three obvious candidates, and the constraints
//! here pick one cleanly:
//!
//! - **Radial basis functions** are smooth but need a K×K solve, go
//!   ill-conditioned when two keyframes land on top of each other (which a user
//!   dragging points *will* do), and overshoot outside the convex hull —
//!   producing out-of-gamut colours that then need real gamut mapping.
//! - **Inverse-distance weighting** never overshoots but has the classic
//!   bullseye artefact: a flat plateau at every control point with abrupt
//!   transitions between them. It looks bad as a colour field.
//! - **Normalised Gaussian weighting** (what this uses) is smooth everywhere,
//!   needs no matrix solve, cannot be made singular by coincident keyframes, and
//!   is a convex combination — so the result stays inside the hull of the
//!   authored colours in Oklab, and lands at worst a rounding error outside
//!   displayable RGB (see [`super::oklab::LinearRgb::gamut_excursion`]).
//!
//! The tradeoff is that it approximates rather than strictly interpolates: the
//! colour exactly at a keyframe is mostly, not purely, that keyframe's colour.
//! For a colour field that reads as an improvement, since it guarantees no
//! discontinuities anywhere.
//!
//! Evaluation is O(keyframes) — a handful of `exp` calls — so there is no lookup
//! table. Rendering a full 256×128 preview costs well under a millisecond, and
//! skipping the bake removes any chance of the cache going stale after an edit.
//!
//! # Opacity
//!
//! Keyframes carry opacity as well as colour, so the field is four channels
//! rather than three. The two are not interpolated the same way: opacity uses
//! the plain Gaussian weights, colour uses those weights scaled by opacity. See
//! [`ColorSurface::sample`] for why, and [`super::oklab`] for what opacity means
//! once it reaches the strip.
//!
//! # Area of effect
//!
//! A keyframe may also carry a [`Keyframe::radius`], past which it contributes
//! nothing at all. That is a different question from [`SurfaceConfig::sigma`],
//! which is how two keyframes that both reach a point share it; sigma is one
//! number for the whole field, so narrowing it to confine one keyframe sharpens
//! every other one at the same time.
//!
//! The consequence is that the field need not be covered. Where nothing
//! reaches — including a field with no keyframes at all — the sample is
//! transparent black, so an area of effect composes with the stack the way a
//! hole in it should: the layer below shows through.
//!
//! # Colour cycle
//!
//! With [`ColorSurface::cycling`] on, the position axis is a *circle* rather
//! than a segment: the distance from a keyframe to a point is measured the
//! short way round, so the two ends of the strip are neighbours. An area of
//! effect that runs off one end comes back on the other, and a keyframe
//! animated from x = 0 to x = 1 arrives where it started instead of jumping
//! back — which is the whole reason it exists, since a chase is a colour
//! crossing the wall over and over and a loop has to close for that to read.
//!
//! Only the x axis wraps. The y axis is level, and level has no far side: a
//! quiet band is not adjacent to a loud one.
//!
//! It is one flag for the whole field rather than per keyframe, because it is a
//! statement about the *axis* — half a field on a circle and half on a segment
//! is not a shape anything downstream could draw.

use serde::{Deserialize, Serialize};

use super::oklab::{srgb_hex_to_linear, srgba_hex_to_linear, LinearRgb, Oklab};
use super::timeline::{Timeline, TimelineKey};

/// One authored control point. `color` is an sRGB hex string so saved presets
/// stay readable and hand-editable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Keyframe {
    /// Position along the strip, 0 = lowest band, 1 = highest.
    pub x: f32,
    /// Band intensity, 0 = silent, 1 = full.
    pub y: f32,
    /// sRGB hex, either `"#ff2000"` or `"#ff2000cc"` with an opacity byte.
    ///
    /// Opacity lives in the colour rather than in a field of its own, so a
    /// preset stays one readable value per keyframe and matches what CSS,
    /// every colour picker and the editor all already call a colour. Six digits
    /// means fully opaque, which is what keeps presets written before opacity
    /// existed loading unchanged.
    pub color: String,
    /// How far this keyframe reaches, as a distance in the unit square.
    /// `None` — and an absent field — means everywhere, which is what every
    /// keyframe did before this existed.
    ///
    /// This is *not* [`SurfaceConfig::sigma`]. Sigma is how two keyframes that
    /// both reach a point argue over it; this is whether a keyframe reaches the
    /// point at all. A keyframe confined to the bass end still blends normally
    /// with its neighbours there, and simply is not in the conversation at the
    /// treble end — which is the thing sigma cannot say, because shrinking
    /// sigma to confine one keyframe sharpens every other one too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
}

impl Keyframe {
    /// A keyframe that reaches the whole field.
    pub fn new(x: f32, y: f32, color: &str) -> Self {
        Self { x, y, color: color.to_string(), radius: None }
    }

    /// A keyframe confined to `radius` of where it sits.
    pub fn within(x: f32, y: f32, color: &str, radius: f32) -> Self {
        Self { radius: Some(radius), ..Self::new(x, y, color) }
    }
}

/// Smallest area of effect that still means something. Below this a keyframe
/// reaches a region narrower than one LED at any plausible strip length, so
/// there is nothing to gain by letting it go to zero and a division to lose.
pub const MIN_RADIUS: f32 = 1e-3;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SurfaceConfig {
    /// May be empty. A field with nothing in it is a layer that paints nothing,
    /// which is a legitimate thing to author and the state a layer passes
    /// through while its keyframes are being replaced one at a time.
    pub keyframes: Vec<Keyframe>,
    /// Blend radius. Smaller gives each keyframe a tighter region and crisper
    /// transitions; larger blurs them together. Around a quarter of the typical
    /// spacing between keyframes reads well.
    pub sigma: f32,
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self {
            // A classic EQ palette: warm at the bottom, green through the mids,
            // cool at the top, with the y=0 row near black so quiet bands go
            // dark rather than merely dim.
            keyframes: vec![
                Keyframe::new(0.0, 0.0, "#1a0000"),
                Keyframe::new(0.0, 1.0, "#ff2000"),
                Keyframe::new(0.5, 0.0, "#001a08"),
                Keyframe::new(0.5, 1.0, "#40ff60"),
                Keyframe::new(1.0, 0.0, "#000820"),
                Keyframe::new(1.0, 1.0, "#40c0ff"),
            ],
            sigma: 0.25,
        }
    }
}

impl SurfaceConfig {
    /// The same field with every keyframe moved to the top row.
    ///
    /// A layer listening to nothing has no intensity axis — there is no level
    /// to be the surface's y — so its field is one dimensional: a colour along
    /// the strip and nothing else. Such a layer is sampled at a flat full
    /// level, so projecting is what makes that literally true rather than
    /// merely close: every keyframe is in the conversation at the one row that
    /// is ever read, and the colour the editor draws on a handle is the colour
    /// that reaches the wall. Sampling a field authored in two dimensions along
    /// its top edge instead would show colours on the graph that the strip
    /// never produces.
    ///
    /// A copy rather than an edit of the stored config, because the authored y
    /// is not wrong — it is simply not being read. A layer switched to no
    /// source and back is the two dimensional field it was.
    pub fn flattened(&self) -> Self {
        Self {
            keyframes: self
                .keyframes
                .iter()
                .map(|k| Keyframe { y: 1.0, ..k.clone() })
                .collect(),
            sigma: self.sigma,
        }
    }
}

/// Largest area of effect worth distinguishing from no limit at all.
///
/// The diagonal of the unit square is √2, so a keyframe reaching this far
/// touches every point of the field from any corner. It exists because
/// [`Keyframe::radius`] is an `Option` and an interpolation cannot be halfway
/// to `None`: across a span where one end is confined and the other is not,
/// "everywhere" stands in as this distance, the radius grows smoothly into it,
/// and reaching it turns the limit off again. The editor's radius slider tops
/// out at the same number, so the two ends of that interpolation are both
/// authorable.
pub const UNCONFINED: f32 = 1.45;

#[derive(Clone, Copy)]
struct Compiled {
    x: f32,
    y: f32,
    color: Oklab,
    /// Precomputed `1 / r` for a confined keyframe, `None` for one that reaches
    /// everywhere.
    reach: Option<f32>,
}

/// One authored keyframe with its colour parsed, before a radius has been
/// turned into the reciprocal [`Compiled`] samples with.
///
/// Kept separate because interpolation happens in the authored units: lerping
/// `1 / r` would make an area of effect open fast and close slowly for no
/// reason anybody asked for.
#[derive(Clone, Copy)]
struct Authored {
    x: f32,
    y: f32,
    color: Oklab,
    radius: Option<f32>,
}

impl Authored {
    fn compile(self) -> Compiled {
        Compiled {
            x: self.x,
            y: self.y,
            color: self.color,
            // A radius of zero would divide by zero and then multiply a zero
            // distance by the infinity it produced, which is a NaN on the
            // strip. Floored instead, so a keyframe dragged to nothing simply
            // reaches nothing.
            reach: self.radius.map(|r| 1.0 / r.max(MIN_RADIUS)),
        }
    }

    /// The same keyframe with its opacity scaled — how a keyframe authored at
    /// one end of a span and absent from the other crosses it.
    fn faded(self, by: f32) -> Self {
        Self { color: Oklab::with_alpha(self.color.l, self.color.a, self.color.b, self.color.alpha * by), ..self }
    }
}

/// The whole field at one authored moment of a loop.
struct Moment {
    /// Where in the loop, 0..1.
    at: f32,
    /// The blend radius at this moment. Interpolated as sigma rather than as
    /// the falloff it becomes, for the same reason a radius is.
    sigma: f32,
    /// Index-matched across every moment, padded to the longest. `None` where
    /// this keyframe is not authored here — see [`ColorSurface::seek`].
    points: Vec<Option<Authored>>,
}

pub struct ColorSurface {
    /// The field as it stands right now, which is the whole of it for a surface
    /// that does not animate.
    points: Vec<Compiled>,
    /// Whether the position axis wraps — see the module docs. Off is what every
    /// field did before this existed, and what an unset flag on the wire means.
    cycle: bool,
    /// Precomputed `1 / (2σ²)`.
    falloff: f32,
    /// Empty unless this surface animates. Sorted by [`Moment::at`].
    moments: Vec<Moment>,
    /// Loop length in seconds. Only read when `moments` has something in it.
    length: f32,
}

impl ColorSurface {
    /// Compile a config for sampling. The only way this fails is a colour that
    /// is not hex; a field with no keyframes at all is legal and samples as
    /// transparent black everywhere. See [`ColorSurface::sample`].
    pub fn new(cfg: &SurfaceConfig) -> Result<Self, String> {
        let points: Vec<Compiled> =
            authored(cfg)?.into_iter().map(Authored::compile).collect();

        Ok(Self {
            points,
            falloff: falloff_of(cfg.sigma),
            moments: Vec::new(),
            length: 0.0,
            cycle: false,
        })
    }

    /// The field a layer paints with: its timeline where it has one, and its
    /// still surface where it does not.
    ///
    /// The two are not alternatives the caller picks between, because the keys
    /// are authoritative wherever there are any — [`crate::show::Layer::surface`]
    /// is only the view of them a peer that does not know about timelines gets,
    /// and reading it in preference to a key would mean an editor and an engine
    /// that both understand timelines still argued about which field was live.
    pub fn for_layer(surface: &SurfaceConfig, timeline: &Timeline) -> Result<Self, String> {
        match timeline.start() {
            // A timeline switched off, or one with a single key, is a still
            // field — the one the loop would have started from.
            Some(start) if !timeline.animates() => Self::new(start),
            Some(_) => Self::animated(timeline),
            None => Self::new(surface),
        }
    }

    /// Compile a timeline for sampling. Fails on the same thing
    /// [`ColorSurface::new`] does and nothing else: a colour that is not hex.
    ///
    /// Every key is padded to the longest one's keyframe count, so the blend in
    /// [`ColorSurface::seek`] is an index walk with no bookkeeping. The keys
    /// are sorted here rather than trusted, because a hand-edited preset is
    /// under no obligation to be in order and an out-of-order key would
    /// otherwise play the loop backwards through it.
    pub fn animated(timeline: &Timeline) -> Result<Self, String> {
        let mut keys: Vec<&TimelineKey> = timeline.keys.iter().collect();
        keys.sort_by(|a, b| a.at.total_cmp(&b.at));

        let width = keys.iter().map(|k| k.surface.keyframes.len()).max().unwrap_or(0);

        let moments = keys
            .iter()
            .map(|k| {
                let mut points: Vec<Option<Authored>> =
                    authored(&k.surface)?.into_iter().map(Some).collect();
                points.resize(width, None);
                Ok(Moment {
                    at: if k.at.is_finite() { k.at.clamp(0.0, 1.0) } else { 0.0 },
                    sigma: k.surface.sigma,
                    points,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        // Valid before anything has sought: a renderer built mid-frame paints
        // the start of the loop rather than an empty field.
        let start = &moments[0];
        let points = start.points.iter().flatten().map(|p| p.compile()).collect();
        let falloff = falloff_of(start.sigma);

        Ok(Self { points, falloff, moments, length: timeline.length(), cycle: false })
    }

    /// The same surface with the position axis joined end to end, or not.
    ///
    /// A builder rather than an argument to every constructor: it changes how
    /// the field is *read*, not what was authored, so nothing about compiling a
    /// config or a timeline depends on it. That is also why it survives
    /// [`ColorSurface::seek`] — the flag belongs to the surface, and the keys
    /// it walks know nothing about it.
    pub fn cycling(mut self, on: bool) -> Self {
        self.cycle = on;
        self
    }

    /// Whether this surface moves on its own. A still one ignores
    /// [`ColorSurface::seek`] entirely.
    pub fn animates(&self) -> bool {
        self.moments.len() >= 2
    }

    /// Move the field to where the loop has reached at `seconds` — seconds
    /// since the Unix epoch, which is what makes the engine and the editor
    /// agree on the instant without it crossing the wire. See
    /// [`crate::color::timeline`].
    ///
    /// Cheap enough to call once per frame per layer: it walks the two
    /// bracketing keys once and writes into a buffer it already owns. Nothing
    /// is parsed and nothing is allocated after the first call.
    ///
    /// # Between two keys
    ///
    /// Everything authored interpolates: position, colour, opacity, area of
    /// effect and the blend radius. The one case that is not a plain lerp is a
    /// keyframe authored at one end of the span and not the other, which fades
    /// its opacity out across the span instead of appearing or vanishing
    /// between two frames. The editor never produces that — it adds and removes
    /// a colour on every key at once — but a hand-edited preset can, and a pop
    /// on the wall is a worse answer than a fade.
    pub fn seek(&mut self, seconds: f64) {
        if !self.animates() {
            return;
        }

        let n = self.moments.len();
        let length = self.length.max(1e-3) as f64;
        let mut phase = (seconds % length) / length;
        if phase < 0.0 {
            phase += 1.0;
        }
        let phase = phase as f32;

        // The bracketing pair, wrapping past the last key back to the first —
        // which is the whole of what makes this a loop rather than a ramp. A
        // phase before the first key is still inside that wrap.
        let (a, b) = if phase < self.moments[0].at {
            (n - 1, 0)
        } else {
            let mut i = 0;
            while i + 1 < n && self.moments[i + 1].at <= phase {
                i += 1;
            }
            (i, (i + 1) % n)
        };

        // Both spans are measured around the loop, so the one that crosses the
        // wrap comes out positive rather than as a negative span that would
        // play the segment backwards.
        let span = wrapped(self.moments[b].at - self.moments[a].at);
        let travelled = wrapped(phase - self.moments[a].at);
        let t = if span > 1e-6 { (travelled / span).clamp(0.0, 1.0) } else { 0.0 };

        let (from, to) = (&self.moments[a], &self.moments[b]);
        self.falloff = falloff_of(lerp(from.sigma, to.sigma, t));

        // Cleared and refilled rather than resized: the capacity is already
        // there after the first frame, so this allocates nothing.
        self.points.clear();
        for (start, end) in from.points.iter().zip(&to.points) {
            let blended = match (start, end) {
                (Some(p), Some(q)) => Some(mix(*p, *q, t)),
                (Some(p), None) => Some(p.faded(1.0 - t)),
                (None, Some(q)) => Some(q.faded(t)),
                (None, None) => None,
            };
            if let Some(point) = blended {
                self.points.push(point.compile());
            }
        }
    }

    /// Colour and opacity at a point in the unit square. Inputs outside 0..1 are
    /// clamped.
    ///
    /// Opacity interpolates on the same Gaussian weights as colour, so it is
    /// smooth for the same reasons — a keyframe faded out leaves a soft hole in
    /// the field rather than a hard-edged one.
    ///
    /// Colour, though, is weighted by `w · α` rather than `w`. See
    /// [`Oklab::blend`], which this is the inlined form of: the accumulation is
    /// spelled out here because it also has to track the nearest keyframe, and
    /// this runs once per strip position per frame.
    ///
    /// # The baseline
    ///
    /// Where no keyframe reaches — because there are none, or because every one
    /// of them is confined somewhere else — the answer is transparent black.
    /// Not *opaque* black: an unreached position has to let a layer underneath
    /// through, exactly like a position no LED sector reaches. That makes
    /// authoring nothing and authoring a hole the same statement, which is the
    /// only way an area of effect can be composed with a stack.
    pub fn sample(&self, x: f32, y: f32) -> Oklab {
        let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));

        // `weight` normalises opacity; `cover` normalises colour. `present` is
        // the third quantity an area of effect needs and neither of those two
        // can supply — see below.
        let (mut weight, mut cover) = (0.0f32, 0.0f32);
        let (mut l, mut a, mut b) = (0.0f32, 0.0f32, 0.0f32);
        let mut present = 0.0f32;
        let mut nearest: Option<(f32, usize)> = None;

        for (i, p) in self.points.iter().enumerate() {
            let dx = if self.cycle { short_way_round(x - p.x) } else { x - p.x };
            let dy = y - p.y;
            let d2 = dx * dx + dy * dy;

            let taper = match p.reach {
                // Only unconfined keyframes are candidates for the fallback
                // below — a confined one is not allowed to colour a point it
                // does not reach, which is the whole feature.
                None => {
                    if nearest.is_none_or(|(best, _)| d2 < best) {
                        nearest = Some((d2, i));
                    }
                    1.0
                }
                Some(inv_r) => {
                    let t = d2.sqrt() * inv_r;
                    if t >= 1.0 {
                        continue;
                    }
                    edge_taper(t)
                }
            };

            present = present.max(taper);

            let w = (-d2 * self.falloff).exp() * taper;
            let wa = w * p.color.alpha;
            weight += w;
            cover += wa;
            l += wa * p.color.l;
            a += wa * p.color.a;
            b += wa * p.color.b;
        }

        // With a small sigma and a point far from every keyframe, all weights
        // can underflow to zero. Falling back to the nearest keyframe keeps the
        // surface defined everywhere instead of returning black — but only the
        // unconfined ones can stand in, and where there are none the baseline
        // above is the answer.
        if weight <= f32::MIN_POSITIVE {
            return match nearest {
                Some((_, i)) => self.points[i].color,
                None => Oklab::with_alpha(0.0, 0.0, 0.0, 0.0),
            };
        }

        // Normalisation is what makes this a convex combination, and it is also
        // what would silently undo the taper: with one keyframe in reach it
        // appears in both `cover` and `weight` and cancels exactly, so opacity
        // would hold the authored value right to the edge and then fall off a
        // cliff into the baseline. `present` is the taper *before*
        // normalisation — how much of anything reaches here at all — and it is
        // the maximum rather than the sum, so two overlapping areas of effect
        // are covered where either one covers, not covered twice.
        let alpha = present * cover / weight;

        // Every keyframe within reach is fully transparent, so there is no
        // colour to average — only the absence of one.
        if cover <= f32::MIN_POSITIVE {
            return Oklab::with_alpha(0.0, 0.0, 0.0, alpha);
        }

        Oklab::with_alpha(l / cover, a / cover, b / cover, alpha)
    }
}

/// Every keyframe of a field, with its colour parsed and nothing else done to
/// it. The one place a config can be rejected, and the shared half of compiling
/// a still surface and compiling a timeline key.
fn authored(cfg: &SurfaceConfig) -> Result<Vec<Authored>, String> {
    cfg.keyframes
        .iter()
        .map(|k| {
            Ok(Authored {
                x: k.x,
                y: k.y,
                color: parse_color(&k.color)?.to_oklab(),
                radius: k.radius,
            })
        })
        .collect()
}

/// `1 / (2σ²)`, with sigma floored so a field authored at zero blend is sharp
/// rather than a division by zero.
fn falloff_of(sigma: f32) -> f32 {
    let sigma = sigma.max(1e-3);
    1.0 / (2.0 * sigma * sigma)
}

/// A difference of two phases, measured forwards around the loop.
fn wrapped(delta: f32) -> f32 {
    if delta >= 0.0 { delta } else { delta + 1.0 }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A separation along the position axis, measured the short way round a strip
/// joined end to end. Never more than half the field, and correct for a
/// keyframe authored outside 0..1 as well as inside it.
///
/// Only the square of this is ever read, so the tie at exactly half a turn —
/// where Rust rounds away from zero and JavaScript rounds up — is the same
/// distance either way and the port stays exact.
fn short_way_round(delta: f32) -> f32 {
    delta - delta.round()
}

/// One keyframe part of the way from where it was authored at one key to where
/// it is authored at the next.
///
/// Colour is interpolated in Oklab, which is the whole reason colours live in
/// Oklab here: a red crossing to a blue passes through the purples rather than
/// through the muddy grey a linear-RGB lerp would give.
///
/// The radius is the one value with no obvious midpoint, because "everywhere"
/// is not a distance. [`UNCONFINED`] stands in for it, so an area of effect
/// opening up grows smoothly to the size of the field and only then stops
/// being a limit at all.
fn mix(p: Authored, q: Authored, t: f32) -> Authored {
    let radius = match (p.radius, q.radius) {
        (None, None) => None,
        (from, to) => {
            let r = lerp(from.unwrap_or(UNCONFINED), to.unwrap_or(UNCONFINED), t);
            if r >= UNCONFINED { None } else { Some(r) }
        }
    };
    Authored {
        x: lerp(p.x, q.x, t),
        y: lerp(p.y, q.y, t),
        color: Oklab::with_alpha(
            lerp(p.color.l, q.color.l, t),
            lerp(p.color.a, q.color.a, t),
            lerp(p.color.b, q.color.b, t),
            lerp(p.color.alpha, q.color.alpha, t),
        ),
        radius,
    }
}

/// The outer fraction of a radius spent fading out. Across the rest of it a
/// keyframe is simply present, at the opacity it was authored with.
///
/// Zero — a hard-edged disc — is the obvious reading of "area of effect" and is
/// wrong on a wall: every other edge in this field is smooth, so the one hard
/// line reads as a fault in the strip rather than as a decision. One is wrong
/// the other way, because then full opacity is reached only at the exact centre
/// and an opaque keyframe never looks opaque.
const EDGE: f32 = 0.35;

/// Presence at `t`, the distance from a keyframe as a fraction of its radius.
/// Flat at 1 across the interior, then smoothstep to 0, with zero slope at both
/// ends of the fade so neither joint shows.
fn edge_taper(t: f32) -> f32 {
    let u = ((1.0 - t) / EDGE).min(1.0);
    u * u * (3.0 - 2.0 * u)
}

/// Parse `#rrggbb` or `#rrggbbaa`, with or without the hash.
///
/// Six digits is fully opaque. That is what lets a preset saved before opacity
/// existed load with exactly the appearance it had.
fn parse_color(s: &str) -> Result<LinearRgb, String> {
    let t = s.trim().trim_start_matches('#');
    let value =
        |t: &str| u32::from_str_radix(t, 16).map_err(|_| format!("colour '{s}' is not valid hex"));
    match t.len() {
        6 => Ok(srgb_hex_to_linear(value(t)?)),
        8 => Ok(srgba_hex_to_linear(value(t)?)),
        _ => Err(format!(
            "colour '{s}' must be 6 hex digits, or 8 with an opacity byte, optionally prefixed with '#'"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::oklab::srgb_hex_to_linear;

    fn default_surface() -> ColorSurface {
        ColorSurface::new(&SurfaceConfig::default()).unwrap()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn parses_hex_with_and_without_hash() {
        let expected = srgb_hex_to_linear(0xFF2000);
        for s in ["#ff2000", "ff2000"] {
            let got = parse_color(s).unwrap();
            assert!(close(got.r, expected.r) && close(got.alpha, 1.0), "{s} gave {got:?}");
        }
        assert!(parse_color("#ff20").is_err());
        assert!(parse_color("#gggggg").is_err());
    }

    /// Eight digits carry opacity, six mean opaque. The second half is the
    /// compatibility guarantee: presets predate the channel and must not shift.
    #[test]
    fn parses_the_opacity_byte() {
        let opaque = parse_color("#ff2000").unwrap();
        let full = parse_color("#ff2000ff").unwrap();
        assert!(close(full.r, opaque.r) && close(full.alpha, 1.0));

        assert!(close(parse_color("#ff200000").unwrap().alpha, 0.0));
        assert!(close(parse_color("ff200080").unwrap().alpha, 128.0 / 255.0));

        // The colour half must not depend on the opacity half.
        let faded = parse_color("#ff200040").unwrap();
        assert!(close(faded.r, opaque.r) && close(faded.g, opaque.g) && close(faded.b, opaque.b));

        assert!(parse_color("#ff2000f").is_err());
        assert!(parse_color("#ff2000gg").is_err());
    }

    /// Sampling at a keyframe should land close to that keyframe's colour. Not
    /// exactly — this interpolator approximates — but close enough that the
    /// authored palette is clearly what shows up.
    #[test]
    fn sampling_at_a_keyframe_resembles_it() {
        let cfg = SurfaceConfig::default();
        let surface = default_surface();

        for k in &cfg.keyframes {
            let expected = parse_color(&k.color).unwrap().to_oklab();
            let got = surface.sample(k.x, k.y);
            let dist = ((got.l - expected.l).powi(2)
                + (got.a - expected.a).powi(2)
                + (got.b - expected.b).powi(2))
            .sqrt();
            assert!(dist < 0.12, "at ({}, {}) got {got:?}, wanted {expected:?}", k.x, k.y);
        }
    }

    /// Quiet bands must be dark, or the whole strip glows at idle.
    #[test]
    fn zero_intensity_is_dark_everywhere() {
        let surface = default_surface();
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            let l = surface.sample(x, 0.0).l;
            assert!(l < 0.25, "x={x} at zero intensity had lightness {l}");
        }
    }

    /// Brightness must rise with intensity at every position on the strip.
    #[test]
    fn lightness_increases_with_intensity() {
        let surface = default_surface();
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            let mut previous = f32::NEG_INFINITY;
            for j in 0..=10 {
                let l = surface.sample(x, j as f32 / 10.0).l;
                assert!(l >= previous - 1e-4, "x={x} lightness dipped at y={}", j as f32 / 10.0);
                previous = l;
            }
        }
    }

    /// Every sample must be displayable to within far less than one 8-bit step,
    /// which is what lets the renderer clamp instead of gamut-mapping.
    ///
    /// Note the invariant is "negligibly outside", not "exactly inside". The
    /// blend is convex in Oklab, but the cubic conversion to RGB does not
    /// preserve convexity, so tiny excursions are expected. This pins how tiny.
    #[test]
    fn every_sample_is_within_a_rounding_error_of_gamut() {
        let surface = default_surface();
        let mut worst = 0.0f32;
        for i in 0..=64 {
            for j in 0..=64 {
                let (x, y) = (i as f32 / 64.0, j as f32 / 64.0);
                let excursion = surface.sample(x, y).to_linear_rgb().gamut_excursion();
                worst = worst.max(excursion);
            }
        }
        assert!(
            worst <= 0.5 / 255.0,
            "worst excursion {worst} exceeds half an 8-bit step ({})",
            0.5 / 255.0
        );
    }

    /// The field must be continuous — a visible seam on the wall would come from
    /// a discontinuity here.
    #[test]
    fn surface_is_continuous() {
        let surface = default_surface();
        let step = 1.0 / 256.0;
        for i in 0..256 {
            let x = i as f32 / 256.0;
            for j in 0..256 {
                let y = j as f32 / 256.0;
                let a = surface.sample(x, y);
                let b = surface.sample(x + step, y);
                assert!(
                    (a.l - b.l).abs() < 0.05 && (a.a - b.a).abs() < 0.05 && (a.b - b.b).abs() < 0.05,
                    "jump between ({x}, {y}) and ({}, {y})",
                    x + step
                );
            }
        }
    }

    #[test]
    fn rejects_malformed_colours() {
        assert!(ColorSurface::new(&SurfaceConfig {
            keyframes: vec![Keyframe::new(0.0, 0.0, "nope")],
            sigma: 0.25,
        })
        .is_err());
    }

    /// A layer with no colours authored at all paints nothing — and *nothing* is
    /// transparent, not black. Opaque black would blank everything under it,
    /// which is the opposite of what an empty field should do to a stack.
    #[test]
    fn an_empty_field_is_transparent_black_everywhere() {
        let surface = ColorSurface::new(&SurfaceConfig { keyframes: vec![], sigma: 0.25 }).unwrap();
        for i in 0..=8 {
            for j in 0..=8 {
                let c = surface.sample(i as f32 / 8.0, j as f32 / 8.0);
                assert!(close(c.alpha, 0.0), "({i}, {j}) sampled at opacity {}", c.alpha);
                assert!(close(c.l, 0.0) && close(c.a, 0.0) && close(c.b, 0.0), "produced {c:?}");
            }
        }
    }

    /// The point of the radius: past it the keyframe is not merely faint, it is
    /// absent. With nothing else in the field that means the baseline.
    #[test]
    fn a_confined_keyframe_reaches_nothing_past_its_radius() {
        let surface = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![Keyframe::within(0.25, 0.5, "#ff2000", 0.2)],
            sigma: 0.25,
        })
        .unwrap();

        assert!(surface.sample(0.25, 0.5).alpha > 0.9, "the keyframe faded at its own position");
        for x in [0.5, 0.75, 1.0] {
            let c = surface.sample(x, 0.5);
            assert!(close(c.alpha, 0.0), "x={x} is {:.3} of the way outside 0.2 away", c.alpha);
        }
    }

    /// Two confined keyframes with a gap between them: the gap is the baseline,
    /// not a blend of the two. This is the dead space the feature creates, and
    /// it has to be transparent so the layer below fills it.
    #[test]
    fn dead_space_between_confined_keyframes_is_transparent() {
        let surface = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![
                Keyframe::within(0.0, 0.5, "#ff2000", 0.2),
                Keyframe::within(1.0, 0.5, "#40c0ff", 0.2),
            ],
            sigma: 0.25,
        })
        .unwrap();

        assert!(surface.sample(0.0, 0.5).alpha > 0.9);
        assert!(surface.sample(1.0, 0.5).alpha > 0.9);
        assert!(close(surface.sample(0.5, 0.5).alpha, 0.0), "the gap was painted");
    }

    /// The edge of an area of effect must be a fade, not a step — a step here is
    /// a hard line across the wall.
    #[test]
    fn a_confined_keyframe_fades_out_rather_than_stopping() {
        let surface = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![Keyframe::within(0.5, 0.5, "#ff2000", 0.3)],
            sigma: 0.25,
        })
        .unwrap();

        let step = 1.0 / 2048.0;
        let mut previous = surface.sample(0.5, 0.5).alpha;
        let mut x = 0.5;
        while x < 1.0 {
            x += step;
            let alpha = surface.sample(x, 0.5).alpha;
            assert!(alpha <= previous + 1e-4, "opacity rose again at x={x}");
            assert!((alpha - previous).abs() < 0.02, "opacity stepped at x={x}");
            previous = alpha;
        }
        assert!(close(previous, 0.0), "it never actually reached the baseline");
    }

    /// An unconfined keyframe standing beside a confined one must still cover
    /// the whole field, including the dead space the confined one leaves.
    #[test]
    fn an_unconfined_keyframe_still_covers_everything() {
        let surface = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![
                Keyframe::within(0.0, 0.5, "#ff2000", 0.15),
                Keyframe::new(1.0, 0.5, "#40c0ff"),
            ],
            sigma: 0.25,
        })
        .unwrap();

        for i in 0..=16 {
            let alpha = surface.sample(i as f32 / 16.0, 0.5).alpha;
            assert!(alpha > 0.9, "x={i} fell to {alpha} with an unconfined keyframe present");
        }
    }

    // -- Colour cycle ----------------------------------------------------

    /// The claim the flag is for: an area of effect that runs off one end of
    /// the strip comes back on the other, rather than being clipped there.
    #[test]
    fn a_cycling_area_of_effect_wraps_around_the_strip() {
        let cfg = SurfaceConfig {
            keyframes: vec![Keyframe::within(0.95, 0.5, "#ff2000", 0.2)],
            sigma: 0.25,
        };

        let plain = ColorSurface::new(&cfg).unwrap();
        assert!(close(plain.sample(0.02, 0.5).alpha, 0.0), "it reached the far end uncycled");

        let cycling = ColorSurface::new(&cfg).unwrap().cycling(true);
        assert!(
            cycling.sample(0.02, 0.5).alpha > 0.5,
            "0.07 away around the loop, and it did not arrive"
        );
    }

    /// Only the position axis is a circle. Level has no far side — a silent
    /// band is not next to a loud one — and wrapping y would light the bottom
    /// of the field whenever the top of it was lit.
    #[test]
    fn only_the_position_axis_wraps() {
        let surface = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![Keyframe::within(0.5, 0.95, "#ff2000", 0.2)],
            sigma: 0.25,
        })
        .unwrap()
        .cycling(true);

        assert!(close(surface.sample(0.5, 0.02).alpha, 0.0), "the level axis wrapped");
    }

    /// Joining the axis makes the two ends of the strip one point, so every
    /// field paints them identically. A closed-form property rather than a
    /// captured value, and the whole of what makes a colour cross the seam
    /// without a step.
    #[test]
    fn the_ends_of_a_cycling_field_meet() {
        let surface = default_surface().cycling(true);
        for i in 0..=8 {
            let y = i as f32 / 8.0;
            let (a, b) = (surface.sample(0.0, y), surface.sample(1.0, y));
            assert!(
                close(a.l, b.l) && close(a.a, b.a) && close(a.b, b.b) && close(a.alpha, b.alpha),
                "the ends disagreed at y={y}: {a:?} against {b:?}"
            );
        }

        // And without the flag they are as far apart as the field is wide, or
        // the assertion above would hold whatever cycling did.
        let plain = default_surface();
        assert!(!close(plain.sample(0.0, 1.0).l, plain.sample(1.0, 1.0).l));
    }

    /// Nothing is ever more than half the field away, whichever side of the
    /// seam either point is on — including a keyframe authored outside 0..1,
    /// which a hand-edited preset is free to hold.
    #[test]
    fn nothing_is_further_than_half_the_strip() {
        for i in -20..=20 {
            let d = i as f32 / 8.0;
            assert!(short_way_round(d).abs() <= 0.5 + 1e-6, "{d} came out {}", short_way_round(d));
        }
        assert!(close(short_way_round(0.9), -0.1));
        assert!(close(short_way_round(-0.9), 0.1));
    }

    /// A radius nobody set must change nothing. The default palette is the
    /// strongest form of that: it predates the field entirely.
    #[test]
    fn an_absent_radius_reaches_everywhere() {
        let bounded_to_everything = ColorSurface::new(&SurfaceConfig {
            // Far past the √2 diagonal of the unit square, so every point is
            // deep inside the plateau and a radius this large is the same thing
            // as no radius at all.
            keyframes: vec![Keyframe::within(0.5, 0.5, "#ff2000", 100.0)],
            sigma: 0.25,
        })
        .unwrap();
        assert!(bounded_to_everything.sample(1.0, 1.0).alpha > 0.9);

        let plain = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![Keyframe::new(0.5, 0.5, "#ff2000")],
            sigma: 0.25,
        })
        .unwrap();
        for i in 0..=8 {
            assert!(close(plain.sample(i as f32 / 8.0, 0.5).alpha, 1.0));
        }
    }

    /// A radius dragged to zero must reach nothing, rather than dividing by it
    /// and putting a NaN on the wire.
    #[test]
    fn a_zero_radius_is_harmless() {
        let surface = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![Keyframe::within(0.5, 0.5, "#ff2000", 0.0)],
            sigma: 0.25,
        })
        .unwrap();

        for i in 0..=8 {
            for j in 0..=8 {
                let c = surface.sample(i as f32 / 8.0, j as f32 / 8.0);
                assert!(c.l.is_finite() && c.alpha.is_finite(), "produced {c:?}");
            }
        }
    }

    /// The radius has to survive the wire, and an absent one has to stay absent
    /// — that is what keeps every preset written before this loading unchanged.
    #[test]
    fn the_radius_round_trips_and_stays_optional() {
        let cfg = SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.0, 0.0, "#ff2000"),
                Keyframe::within(1.0, 1.0, "#40c0ff", 0.3),
            ],
            sigma: 0.25,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("\"radius\":null"), "an unconfined keyframe wrote a radius: {json}");

        let back: SurfaceConfig = serde_json::from_str(&json).unwrap();
        assert!(back.keyframes[0].radius.is_none());
        assert!(close(back.keyframes[1].radius.unwrap(), 0.3));

        // And a config from before the field existed.
        let old: SurfaceConfig =
            serde_json::from_str(r##"{"keyframes":[{"x":0,"y":1,"color":"#ff2000"}],"sigma":0.25}"##)
                .unwrap();
        assert!(old.keyframes[0].radius.is_none());
    }

    /// Coincident keyframes are something a user dragging points will produce.
    /// This interpolator must simply average them, not blow up.
    #[test]
    fn coincident_keyframes_are_harmless() {
        let cfg = SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.5, 0.5, "#ff0000"),
                Keyframe::new(0.5, 0.5, "#0000ff"),
            ],
            sigma: 0.25,
        };
        let surface = ColorSurface::new(&cfg).unwrap();
        let c = surface.sample(0.5, 0.5);
        assert!(c.l.is_finite() && c.a.is_finite() && c.b.is_finite(), "produced {c:?}");
    }

    /// The shipped palette predates opacity and must still be fully opaque, or
    /// adding the channel would have quietly changed what everyone's strip does.
    #[test]
    fn the_default_palette_is_opaque_everywhere() {
        let surface = default_surface();
        for i in 0..=16 {
            for j in 0..=16 {
                let alpha = surface.sample(i as f32 / 16.0, j as f32 / 16.0).alpha;
                assert!(close(alpha, 1.0), "({i}, {j}) sampled at opacity {alpha}");
            }
        }
    }

    /// Opacity has to be a field like colour is: smooth between keyframes, and
    /// landing near what was authored at each of them.
    #[test]
    fn opacity_interpolates_across_the_field() {
        let cfg = SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.0, 0.5, "#ff2000ff"),
                Keyframe::new(1.0, 0.5, "#ff200000"),
            ],
            sigma: 0.25,
        };
        let surface = ColorSurface::new(&cfg).unwrap();

        assert!(surface.sample(0.0, 0.5).alpha > 0.9, "the opaque end faded");
        assert!(surface.sample(1.0, 0.5).alpha < 0.1, "the transparent end did not");

        let mut previous = f32::INFINITY;
        for i in 0..=20 {
            let alpha = surface.sample(i as f32 / 20.0, 0.5).alpha;
            assert!((0.0..=1.0).contains(&alpha), "opacity left 0..1 at x={i}: {alpha}");
            assert!(alpha <= previous + 1e-4, "opacity rose again at x={i}");
            previous = alpha;
        }
    }

    /// The bug alpha-weighted blending exists to prevent: an invisible keyframe
    /// must not tint the colours around it. A user fading a green keyframe out
    /// expects the green to leave, not to linger as a wash over its neighbours.
    #[test]
    fn a_transparent_keyframe_lends_no_colour() {
        let with_ghost = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.0, 0.5, "#ff2000"),
                Keyframe::new(0.4, 0.5, "#00ff0000"),
            ],
            sigma: 0.25,
        })
        .unwrap();
        let without = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![Keyframe::new(0.0, 0.5, "#ff2000")],
            sigma: 0.25,
        })
        .unwrap();

        for i in 0..=10 {
            let x = i as f32 / 10.0;
            let (ghost, plain) = (with_ghost.sample(x, 0.5), without.sample(x, 0.5));
            assert!(
                close(ghost.l, plain.l) && close(ghost.a, plain.a) && close(ghost.b, plain.b),
                "the invisible keyframe changed the colour at x={x}: {ghost:?} vs {plain:?}"
            );
        }

        // It must still make the field *fade*, though — it is transparent, not
        // absent, and pulling opacity down is exactly what it is there to do.
        assert!(with_ghost.sample(0.4, 0.5).alpha < 0.6, "the field did not fade toward it");
    }

    /// A field with nothing visible in it must stay finite rather than dividing
    /// by an opacity of zero.
    #[test]
    fn a_fully_transparent_field_is_harmless() {
        let surface = ColorSurface::new(&SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.0, 0.0, "#ff200000"),
                Keyframe::new(1.0, 1.0, "#40c0ff00"),
            ],
            sigma: 0.25,
        })
        .unwrap();

        for i in 0..=8 {
            let c = surface.sample(i as f32 / 8.0, 0.5);
            assert!(c.l.is_finite() && c.a.is_finite() && c.b.is_finite(), "produced {c:?}");
            assert!(close(c.alpha, 0.0), "opacity should be zero, got {}", c.alpha);
        }
    }

    // -- Timelines -------------------------------------------------------
    //
    // The field as a function of time. `seek` takes seconds since the epoch, so
    // every case below picks a length and a phase and multiplies.
    //
    // Two probe fields run through these, and the choice of each is the
    // interpolator's doing. `dot` has a single unconfined keyframe, so the
    // normalised weighting makes its colour the colour of the *whole* field —
    // which is what turns "did the colour travel" into one sample. `spot`
    // confines its keyframe instead, so the field is covered near it and
    // transparent away from it, and opacity reads out *where* it is. Sweeping
    // for a brightest point would answer neither: one keyframe, normalised, is
    // the same lightness everywhere.

    /// A field that is one colour everywhere. See above.
    fn dot(color: &str) -> SurfaceConfig {
        SurfaceConfig { keyframes: vec![Keyframe::new(0.5, 0.5, color)], sigma: 0.25 }
    }

    /// A field covered within 0.3 of `x` and transparent past it.
    fn spot(x: f32) -> SurfaceConfig {
        SurfaceConfig { keyframes: vec![Keyframe::within(x, 0.5, "#ffffff", 0.3)], sigma: 0.25 }
    }

    fn lab(hex: &str) -> Oklab {
        parse_color(hex).unwrap().to_oklab()
    }

    fn is_color(got: Oklab, hex: &str) -> bool {
        let want = lab(hex);
        close(got.l, want.l) && close(got.a, want.a) && close(got.b, want.b)
    }

    /// Red to blue and back, over four seconds.
    fn fading() -> Timeline {
        Timeline {
            enabled: true,
            length: 4.0,
            keys: vec![
                TimelineKey::new(0.0, dot("#ff0000")),
                TimelineKey::new(0.5, dot("#0000ff")),
            ],
        }
    }

    /// A covered area travelling from one end of the strip to the other.
    fn sliding() -> Timeline {
        Timeline {
            enabled: true,
            length: 4.0,
            keys: vec![TimelineKey::new(0.0, spot(0.0)), TimelineKey::new(0.5, spot(1.0))],
        }
    }

    /// A surface with no timeline must ignore the clock completely, or every
    /// show written before this existed would start drifting.
    #[test]
    fn a_still_surface_does_not_move() {
        let mut surface = default_surface();
        assert!(!surface.animates());
        let before = surface.sample(0.3, 0.7);
        surface.seek(1_750_000_000.0);
        assert_eq!(before, surface.sample(0.3, 0.7));
    }

    /// Landing on a key shows that key's field. Anything else and the authored
    /// moments would be the only ones the loop never actually reaches.
    #[test]
    fn a_key_is_reached_exactly_at_its_own_phase() {
        let mut surface = ColorSurface::animated(&fading()).unwrap();
        assert!(surface.animates());

        surface.seek(0.0);
        assert!(is_color(surface.sample(0.5, 0.5), "#ff0000"), "phase 0 is not the first key");

        // Half of a four second loop.
        surface.seek(2.0);
        assert!(is_color(surface.sample(0.5, 0.5), "#0000ff"), "phase 0.5 is not the second key");
    }

    /// The point of interpolating rather than switching: part way between two
    /// keys is part way between their fields. Through Oklab, so red crossing to
    /// blue passes through the purples rather than through the muddy grey a
    /// linear RGB lerp would give.
    #[test]
    fn between_two_keys_the_colour_is_part_way_there() {
        let mut surface = ColorSurface::animated(&fading()).unwrap();
        // A quarter of the loop is halfway between the keys at 0 and 0.5.
        surface.seek(1.0);

        let got = surface.sample(0.5, 0.5);
        let (red, blue) = (lab("#ff0000"), lab("#0000ff"));
        assert!(close(got.l, (red.l + blue.l) / 2.0), "l: {}", got.l);
        assert!(close(got.a, (red.a + blue.a) / 2.0), "a: {}", got.a);
        assert!(close(got.b, (red.b + blue.b) / 2.0), "b: {}", got.b);
    }

    /// Positions travel as well as colours, which is the difference between a
    /// palette crossfade and something moving along the wall.
    #[test]
    fn a_keyframe_travels_along_the_strip() {
        let mut surface = ColorSurface::animated(&sliding()).unwrap();

        surface.seek(0.0);
        assert!(surface.sample(0.05, 0.5).alpha > 0.9, "the bass end is not covered at phase 0");
        assert!(close(surface.sample(0.95, 0.5).alpha, 0.0), "the treble end already is");

        // Halfway between the two keys, so halfway along the strip: both ends
        // are now out of reach and the middle is covered.
        surface.seek(1.0);
        assert!(surface.sample(0.5, 0.5).alpha > 0.9, "the middle is not covered halfway");
        assert!(close(surface.sample(0.05, 0.5).alpha, 0.0), "it did not leave the bass end");

        surface.seek(2.0);
        assert!(surface.sample(0.95, 0.5).alpha > 0.9, "it did not arrive at the treble end");
    }

    /// The last key interpolates back to the first rather than holding until
    /// the loop restarts, which is what makes this a loop and not a ramp.
    #[test]
    fn the_loop_closes_from_the_last_key_back_to_the_first() {
        let mut surface = ColorSurface::animated(&sliding()).unwrap();

        // Three quarters of the loop: halfway back from the key at 0.5 to the
        // one at 0, going forwards around the wrap.
        surface.seek(3.0);
        assert!(surface.sample(0.5, 0.5).alpha > 0.9, "the wrap did not interpolate");
        assert!(close(surface.sample(0.95, 0.5).alpha, 0.0), "it never left the treble end");

        // And a whole loop later is the start again.
        surface.seek(4.0);
        assert!(surface.sample(0.05, 0.5).alpha > 0.9, "the loop did not come back round");
    }

    /// A key out of order is sorted rather than played backwards. A preset can
    /// be hand-edited, and an unsorted one would otherwise run the loop through
    /// its keys in whatever order somebody happened to type them.
    #[test]
    fn keys_are_sorted_rather_than_trusted() {
        let mut surface = ColorSurface::animated(&Timeline {
            enabled: true,
            length: 4.0,
            keys: vec![
                TimelineKey::new(0.5, dot("#0000ff")),
                TimelineKey::new(0.0, dot("#ff0000")),
            ],
        })
        .unwrap();

        surface.seek(0.0);
        assert!(is_color(surface.sample(0.5, 0.5), "#ff0000"));
        surface.seek(2.0);
        assert!(is_color(surface.sample(0.5, 0.5), "#0000ff"));
    }

    /// The blend radius is authored per key, so it has to travel too — a field
    /// that softens over the loop is as much a look as one that moves.
    ///
    /// Read out through opacity: an opaque keyframe at one end of the field and
    /// a transparent one at the other hand over across it, and how quickly they
    /// do that *is* sigma.
    #[test]
    fn the_blend_radius_interpolates_with_the_field() {
        let handover = |sigma| SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.0, 0.5, "#ffffffff"),
                Keyframe::new(1.0, 0.5, "#ffffff00"),
            ],
            sigma,
        };
        let mut surface = ColorSurface::animated(&Timeline {
            enabled: true,
            length: 2.0,
            keys: vec![
                TimelineKey::new(0.0, handover(0.05)),
                TimelineKey::new(0.5, handover(0.45)),
            ],
        })
        .unwrap();

        surface.seek(0.0);
        let sharp = surface.sample(0.8, 0.5).alpha;
        surface.seek(1.0);
        let soft = surface.sample(0.8, 0.5).alpha;
        assert!(soft > sharp + 0.1, "sigma did not open up: {sharp} then {soft}");
    }

    /// An area of effect is authored per key like everything else, so a loop can
    /// open and close one.
    #[test]
    fn an_area_of_effect_opens_and_closes_over_the_loop() {
        let reaching = |radius| SurfaceConfig {
            keyframes: vec![Keyframe::within(0.0, 0.5, "#ffffff", radius)],
            sigma: 0.25,
        };
        let mut surface = ColorSurface::animated(&Timeline {
            enabled: true,
            length: 2.0,
            keys: vec![TimelineKey::new(0.0, reaching(0.1)), TimelineKey::new(0.5, reaching(0.9))],
        })
        .unwrap();

        // A point well outside the tight radius and well inside the wide one.
        surface.seek(0.0);
        assert!(close(surface.sample(0.5, 0.5).alpha, 0.0), "the tight key already reached 0.5");
        surface.seek(1.0);
        assert!(surface.sample(0.5, 0.5).alpha > 0.5, "the wide key did not reach 0.5");
    }

    /// Going the other way: an area of effect can be released into reaching
    /// everywhere, because a radius cannot be halfway to `None` and
    /// [`UNCONFINED`] is the distance that stands in for it.
    #[test]
    fn an_area_of_effect_can_open_all_the_way_to_unconfined() {
        let confined = SurfaceConfig {
            keyframes: vec![Keyframe::within(0.0, 0.5, "#ffffff", 0.1)],
            sigma: 0.25,
        };
        let open =
            SurfaceConfig { keyframes: vec![Keyframe::new(0.0, 0.5, "#ffffff")], sigma: 0.25 };
        let mut surface = ColorSurface::animated(&Timeline {
            enabled: true,
            length: 2.0,
            keys: vec![TimelineKey::new(0.0, confined), TimelineKey::new(0.5, open)],
        })
        .unwrap();

        surface.seek(0.0);
        assert!(close(surface.sample(1.0, 0.5).alpha, 0.0), "the confined key reached the far end");
        surface.seek(1.0);
        assert!(surface.sample(1.0, 0.5).alpha > 0.9, "the far end was not covered");
    }

    /// A keyframe the editor never produces but a hand-edited preset can: one
    /// authored at one key and missing from the next. It has to fade, because
    /// appearing or vanishing between two frames is a visible step on the wall
    /// and a parse error would lose the whole show.
    #[test]
    fn a_keyframe_missing_from_one_key_fades_rather_than_popping() {
        let both = SurfaceConfig {
            keyframes: vec![
                Keyframe::new(0.0, 0.5, "#ffffff"),
                Keyframe::within(1.0, 0.5, "#ffffff", 0.2),
            ],
            sigma: 0.15,
        };
        let one =
            SurfaceConfig { keyframes: vec![Keyframe::new(0.0, 0.5, "#ffffff")], sigma: 0.15 };

        let mut surface = ColorSurface::animated(&Timeline {
            enabled: true,
            length: 2.0,
            keys: vec![TimelineKey::new(0.0, both), TimelineKey::new(0.5, one)],
        })
        .unwrap();

        surface.seek(0.0);
        let present = surface.sample(1.0, 0.5).alpha;
        surface.seek(0.5);
        let halfway = surface.sample(1.0, 0.5).alpha;
        surface.seek(1.0);
        let gone = surface.sample(1.0, 0.5).alpha;

        assert!(present > 0.5, "the keyframe was not there to begin with: {present}");
        assert!(gone < 0.05, "it should have gone entirely, got {gone}");
        assert!(
            halfway > 0.05 && halfway < present,
            "it stepped instead of fading: {present} then {halfway} then {gone}"
        );
    }

    /// The keys are the field wherever there are any. A layer's own surface is
    /// the view a peer that does not understand timelines gets, and preferring
    /// it would leave two editors arguing about which field is live.
    #[test]
    fn the_keys_win_over_the_layers_own_surface() {
        let mut surface = ColorSurface::for_layer(&dot("#00ff00"), &fading()).unwrap();
        assert!(surface.animates());
        surface.seek(0.0);
        assert!(is_color(surface.sample(0.5, 0.5), "#ff0000"), "the stored surface was painted");

        // With no keys at all the surface is the whole of it, which is every
        // show written before timelines existed.
        let still = ColorSurface::for_layer(&dot("#00ff00"), &Timeline::default()).unwrap();
        assert!(!still.animates());
        assert!(is_color(still.sample(0.5, 0.5), "#00ff00"));
    }

    /// Switching a timeline off holds its first key rather than falling back to
    /// the layer's stored surface, which is only ever a mirror of that key and
    /// may be a stale one.
    #[test]
    fn a_timeline_switched_off_holds_its_first_key() {
        let off = Timeline { enabled: false, ..fading() };
        let mut surface = ColorSurface::for_layer(&dot("#00ff00"), &off).unwrap();
        assert!(!surface.animates());
        assert!(is_color(surface.sample(0.5, 0.5), "#ff0000"), "not the first key's field");

        surface.seek(2.0);
        assert!(is_color(surface.sample(0.5, 0.5), "#ff0000"), "a switched-off timeline moved");
    }

    /// The flag belongs to the surface, not to the keys it walks — so a loop
    /// that has been sought is still cycling afterwards. Getting this wrong
    /// would cycle on the first frame and stop on the second, which reads as
    /// the wall tearing once per loop rather than as a setting doing nothing.
    #[test]
    fn cycling_survives_a_seek() {
        let mut surface = ColorSurface::animated(&sliding()).unwrap().cycling(true);
        for phase in [0.0, 0.3, 0.75, 0.9] {
            surface.seek(phase * 4.0);
            let (a, b) = (surface.sample(0.0, 0.5), surface.sample(1.0, 0.5));
            assert!(close(a.alpha, b.alpha), "the seam opened at phase {phase}");
        }
    }

    /// Seeking runs once per layer per frame, so it must not allocate after the
    /// first call. Capacity is the observable proxy for that.
    #[test]
    fn seeking_reuses_its_buffer() {
        let mut surface = ColorSurface::animated(&sliding()).unwrap();
        surface.seek(0.0);
        let capacity = surface.points.capacity();
        for i in 0..200 {
            surface.seek(i as f64 * 0.017);
        }
        assert_eq!(surface.points.capacity(), capacity, "the point buffer was reallocated");
    }
}
