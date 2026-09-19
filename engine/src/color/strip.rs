//! Where on the strip each frequency lands.
//!
//! # Why this does not need a firmware change
//!
//! The protocol carries a fixed number of colours per frame and the firmware
//! spreads them across the strip — 149 bytes regardless of whether there are
//! 150 LEDs or 600. Those colours were previously one-per-band. They are now
//! one per *strip position*, and the firmware cannot tell the difference: it
//! interpolates N colours across the run either way.
//!
//! That is what makes LED sectors, reverse and mirror implementable at all. The
//! alternative — per-LED frames — is 1800 bytes at 600 LEDs and caps the
//! display at 27 fps, and an ATmega cannot evaluate a sector table and an Oklab
//! surface inside an interrupt-disabled strip write.
//!
//! The cost is spatial resolution: sector boundaries and the mirror fold are
//! smoothed over `leds / points` LEDs, twelve at 600 LEDs and 48 bands. Raise
//! `--bands` to sharpen them.

use serde::{Deserialize, Serialize};

use super::intensity::IntensityConfig;

/// No channel owns this point.
///
/// MIDI's channels are 0..15, so anything outside that says "nobody" — which is
/// what every audio level is, and what a MIDI point with nothing sounding on it
/// is too. It matters that the two are the same answer: a channel tint applies
/// where a note is, and an unlit point has no note to be that channel's.
pub const NO_CHANNEL: u8 = 0xFF;

/// The editor's frequency axis. Fixed rather than derived from the band plan so
/// that a keyframe authored at 250 Hz lands on 250 Hz whatever the plan does.
pub const F_MIN: f32 = 20.0;
pub const F_MAX: f32 = 20_000.0;

pub fn hz_to_norm(hz: f32) -> f32 {
    ((hz / F_MIN).ln() / (F_MAX / F_MIN).ln()).clamp(0.0, 1.0)
}

pub fn norm_to_hz(n: f32) -> f32 {
    F_MIN * ((F_MAX / F_MIN).ln() * n.clamp(0.0, 1.0)).exp()
}

/// A frequency pinned to an LED index. Consecutive keyframes define a sector.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LedKeyframe {
    pub led: usize,
    pub hz: f32,
}

impl Default for LedKeyframe {
    fn default() -> Self {
        Self { led: 0, hz: F_MIN }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LayoutConfig {
    /// Empty means the whole axis spread evenly across the whole strip.
    pub led_keyframes: Vec<LedKeyframe>,
    /// Play the strip back to front.
    pub reverse: bool,
    /// Fold the whole range into each half.
    pub mirror: bool,
}

impl LayoutConfig {
    pub fn spanning(led_count: usize) -> Self {
        Self {
            led_keyframes: vec![
                LedKeyframe { led: 0, hz: F_MIN },
                LedKeyframe { led: led_count.saturating_sub(1), hz: F_MAX },
            ],
            reverse: false,
            mirror: false,
        }
    }
}

/// Which logical LED an output position shows.
///
/// Both transforms are spatial: they rearrange where colour lands on the wall
/// and change nothing about the analysis, which is why they apply at the very
/// end and are invisible to the editor's graph.
///
/// Mirror folds the whole range into each half, so both ends of the strip show
/// the low end and the centre shows the high end. Reverse then flips what that
/// lookup returns, which is what puts bass in the middle when both are on.
pub fn source_position(u: f32, mirror: bool, reverse: bool) -> f32 {
    let mut u = u.clamp(0.0, 1.0);
    if mirror {
        u = if u < 0.5 { u * 2.0 } else { (1.0 - u) * 2.0 };
    }
    if reverse {
        u = 1.0 - u;
    }
    u
}

/// The frequency an LED displays, or `None` if it falls outside every sector.
///
/// Sectors are ordered by index, not by frequency — a sector may run downwards
/// in frequency if that is how it was authored. Interpolation is linear on the
/// *log* axis, which is what makes "LED 0 at 20 Hz, LED 50 at 2 kHz" spread
/// evenly across the first fifty rather than piling seven octaves into the last
/// few.
pub fn led_frequency(sorted: &[LedKeyframe], led: f32) -> Option<f32> {
    let first = sorted.first()?;
    let last = sorted.last()?;
    if led < first.led as f32 || led > last.led as f32 {
        return None;
    }
    if sorted.len() == 1 {
        return Some(first.hz);
    }

    for pair in sorted.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if led > b.led as f32 {
            continue;
        }
        let span = b.led as f32 - a.led as f32;
        let t = if span <= 0.0 { 0.0 } else { (led - a.led as f32) / span };
        return Some(norm_to_hz(hz_to_norm(a.hz) + t * (hz_to_norm(b.hz) - hz_to_norm(a.hz))));
    }
    Some(last.hz)
}

/// The analyser's level at an arbitrary frequency, interpolated on the log axis.
pub fn level_at(levels: &[f32], centers: &[f32], hz: f32) -> f32 {
    let n = levels.len().min(centers.len());
    if n == 0 {
        return 0.0;
    }
    if hz <= centers[0] {
        return levels[0];
    }
    if hz >= centers[n - 1] {
        return levels[n - 1];
    }
    for i in 1..n {
        if hz > centers[i] {
            continue;
        }
        let (lo, hi) = (hz_to_norm(centers[i - 1]), hz_to_norm(centers[i]));
        let t = if hi > lo { (hz_to_norm(hz) - lo) / (hi - lo) } else { 0.0 };
        return levels[i - 1] + t * (levels[i] - levels[i - 1]);
    }
    levels[n - 1]
}

/// The loudest level between two frequencies, or `None` when no band centre
/// falls between them. `centers` must be ascending, which both band plans and
/// the note grid are.
///
/// This is the other half of [`level_at`], and which one is correct depends on
/// which of the two grids is finer. Reading a level *between* centres is
/// interpolation, and that is right whenever the frame has at least as many
/// control points as the analyser has bands — the audio path, always. It is
/// wrong the moment the analyser is the finer of the two, because then a
/// control point stands for a span of the grid rather than a position in it,
/// and sampling one position inside that span slides off whatever peak is in
/// there.
///
/// MIDI is exactly that case and is the reason this exists: the note grid is
/// one point per semitone — 88 of them for a piano — and the wire carries 64.
/// A note is about a semitone wide, so the sample lands beside its peak and
/// reports a velocity nobody played; at 64 points the worst-placed note loses a
/// quarter of its level. On a colour surface that is not a slightly dimmer
/// note, it is a different colour.
///
/// Taking the loudest rather than averaging, for the same reason
/// [`crate::midi::notes::NoteEngine`] does when it spreads a note over its
/// neighbours: two notes a semitone apart are two notes, not one of them at
/// half strength.
pub fn peak_level_between(levels: &[f32], centers: &[f32], a: f32, b: f32) -> Option<f32> {
    peak_between(levels, centers, a, b).map(|(_, level)| level)
}

/// [`peak_level_between`], and *which* grid point the peak came from.
///
/// The index is what carries a MIDI note's channel through to the colour: the
/// level and the channel have to be read off the same note, or a point would be
/// coloured by one hand and lit by another.
pub fn peak_between(
    levels: &[f32],
    centers: &[f32],
    a: f32,
    b: f32,
) -> Option<(usize, f32)> {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let n = levels.len().min(centers.len());
    let centers = &centers[..n];

    let first = centers.partition_point(|&c| c < lo);
    let last = centers.partition_point(|&c| c <= hi);
    if first >= last {
        return None;
    }
    levels[first..last]
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(i, &level)| (first + i, level))
}

/// The grid point nearest a frequency, on the log axis the grid is even on.
///
/// The other half of [`peak_between`], for the case where the frame is the
/// finer of the two grids and a level is interpolated rather than aggregated.
/// A colour cannot be interpolated between two channels — they are identities,
/// not quantities — so the nearest note is the one that owns the point.
pub fn nearest_index(centers: &[f32], hz: f32) -> Option<usize> {
    if centers.is_empty() {
        return None;
    }
    let target = hz_to_norm(hz);
    let mut best = 0;
    let mut best_d = f32::INFINITY;
    for (i, &c) in centers.iter().enumerate() {
        let d = (hz_to_norm(c) - target).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    Some(best)
}

/// What one wire colour is sampled from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlPoint {
    /// Surface x — position on the frequency axis, 0..1.
    pub x: f32,
    /// Surface y — the band's level, 0..1.
    pub y: f32,
    /// Brightness from the intensity stage, applied in linear light.
    pub gain: f32,
    /// Whether this layer's sectors address this position at all.
    ///
    /// Distinct from `gain == 0`, and the distinction only exists because of
    /// the stack. A band that is *quiet* has been painted with the colour its
    /// field holds at zero intensity — black, in the stock palette — and that
    /// colour covers whatever is beneath it. A position no sector reaches has
    /// not been painted at all, so the layer below shows through untouched.
    ///
    /// Over the unlit strip the two are indistinguishable, which is why a
    /// one-layer show behaves exactly as it did before this field existed.
    pub covered: bool,
    /// The MIDI channel whose note this point is showing, or [`NO_CHANNEL`].
    ///
    /// Always `NO_CHANNEL` for audio, which has no channels to report: a band
    /// is a range of frequency and nobody played it. See
    /// [`StripMap::map_with`].
    pub channel: u8,
}

impl Default for ControlPoint {
    fn default() -> Self {
        // Not derived, because a derived `channel` would be zero — which is
        // channel 1, a real channel with a real colour, rather than nobody.
        Self { x: 0.0, y: 0.0, gain: 0.0, covered: false, channel: NO_CHANNEL }
    }
}

/// Turns band levels into the control points the wire carries.
pub struct StripMap {
    layout: LayoutConfig,
    intensity: IntensityConfig,
    /// Sorted by LED index; sectors are defined by index order.
    sorted: Vec<LedKeyframe>,
    centers: Vec<f32>,
    led_count: usize,
    points: Vec<ControlPoint>,
    /// Whether the level grid is finer than the frame, which decides how a
    /// level is read off it — see [`peak_level_between`].
    ///
    /// One flag for the whole map rather than a decision per point: both grids
    /// are near-uniform on the log axis, so either the analyser is the finer of
    /// the two everywhere or it is nowhere. A sector that stretches a narrow
    /// range over many LEDs is the exception, and it falls back on its own —
    /// a span with no centre in it has nothing to take a peak of.
    dense: bool,
}

impl StripMap {
    pub fn new(
        layout: LayoutConfig,
        intensity: IntensityConfig,
        centers: &[f32],
        point_count: usize,
        led_count: usize,
    ) -> Self {
        let mut map = Self {
            layout: LayoutConfig::default(),
            intensity,
            sorted: Vec::new(),
            centers: centers.to_vec(),
            led_count,
            points: vec![ControlPoint::default(); point_count],
            dense: centers.len() > point_count,
        };
        map.set_layout(layout);
        map
    }

    pub fn set_layout(&mut self, layout: LayoutConfig) {
        // An empty or single keyframe list cannot define a sector, so it falls
        // back to the whole axis rather than blanking the strip.
        let mut sorted = layout.led_keyframes.clone();
        sorted.sort_by_key(|k| k.led);
        if sorted.len() < 2 {
            sorted = LayoutConfig::spanning(self.led_count).led_keyframes;
        }
        self.sorted = sorted;
        self.layout = layout;
    }

    pub fn set_intensity(&mut self, intensity: IntensityConfig) {
        self.intensity = intensity;
    }

    pub fn layout(&self) -> &LayoutConfig {
        &self.layout
    }

    pub fn intensity(&self) -> &IntensityConfig {
        &self.intensity
    }

    /// Turn a frame of levels into control points. See [`Self::map_with`] for
    /// the same thing with a channel per level beside it.
    pub fn map(&mut self, levels: &[f32]) -> &[ControlPoint] {
        self.map_with(levels, &[])
    }

    /// The same, carrying each level's MIDI channel through to the point that
    /// ends up showing it.
    ///
    /// `channels` is either empty — audio, or a MIDI frame nobody asked to
    /// colour — or the same length as `levels`. It is read through *exactly*
    /// the index the level came from, which is the whole reason it is a second
    /// slice rather than something derived from frequency afterwards: the two
    /// halves of one note must not be looked up separately.
    pub fn map_with(&mut self, levels: &[f32], channels: &[u8]) -> &[ControlPoint] {
        let last_point = self.points.len().saturating_sub(1).max(1) as f32;
        let last_led = self.led_count.saturating_sub(1).max(1) as f32;

        for i in 0..self.points.len() {
            let u = source_position(i as f32 / last_point, self.layout.mirror, self.layout.reverse);
            let point = match led_frequency(&self.sorted, u * last_led) {
                // Outside every sector: the LED is not addressed by this layer,
                // so it is dark over an unlit strip and transparent over a
                // layer below. See [`ControlPoint::covered`].
                None => ControlPoint {
                    x: u,
                    y: 0.0,
                    gain: 0.0,
                    covered: false,
                    channel: NO_CHANNEL,
                },
                Some(hz) => {
                    // A frame coarser than the grid has to aggregate rather
                    // than sample, or it slides off the peaks. See
                    // [`peak_between`].
                    let peak = self.span_peak(levels, i, last_point, last_led);
                    let level = peak
                        .map(|(_, level)| level)
                        .unwrap_or_else(|| level_at(levels, &self.centers, hz))
                        .clamp(0.0, 1.0);
                    // Whichever grid point that level came from is the one
                    // whose channel this point is showing. Where the level was
                    // interpolated there is no such point, so the nearest one
                    // answers — a colour cannot be halfway between two
                    // channels, because channels are identities rather than
                    // quantities.
                    let channel = if channels.is_empty() {
                        NO_CHANNEL
                    } else {
                        peak.map(|(index, _)| index)
                            .or_else(|| nearest_index(&self.centers, hz))
                            .and_then(|index| channels.get(index).copied())
                            .unwrap_or(NO_CHANNEL)
                    };
                    ControlPoint {
                        x: hz_to_norm(hz),
                        y: level,
                        gain: self.intensity.brightness(level),
                        covered: true,
                        channel,
                    }
                }
            };
            self.points[i] = point;
        }
        &self.points
    }

    /// The loudest level across the span of the axis one control point stands
    /// for, or `None` when there is nothing to aggregate — the frame is not the
    /// coarser grid, or this particular span happens to hold no band centre.
    ///
    /// The span runs between the midpoints to the neighbouring control points,
    /// taken through the same sector, mirror and reverse mapping as the point
    /// itself, so it is the span on the *axis* rather than on the strip. At a
    /// mirror fold the two edges land on the same side and the span collapses,
    /// which falls back by itself.
    fn span_peak(
        &self,
        levels: &[f32],
        i: usize,
        last_point: f32,
        last_led: f32,
    ) -> Option<(usize, f32)> {
        if !self.dense {
            return None;
        }
        let edge = |offset: f32| {
            let u = source_position(
                (i as f32 + offset) / last_point,
                self.layout.mirror,
                self.layout.reverse,
            );
            led_frequency(&self.sorted, u * last_led)
        };
        peak_between(levels, &self.centers, edge(-0.5)?, edge(0.5)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyframes(pairs: &[(usize, f32)]) -> Vec<LedKeyframe> {
        pairs.iter().map(|&(led, hz)| LedKeyframe { led, hz }).collect()
    }

    #[test]
    fn frequency_axis_round_trips() {
        for hz in [20.0, 100.0, 1000.0, 20_000.0] {
            assert!((norm_to_hz(hz_to_norm(hz)) - hz).abs() / hz < 1e-4);
        }
        assert_eq!(hz_to_norm(F_MIN), 0.0);
        assert_eq!(hz_to_norm(F_MAX), 1.0);
    }

    /// The example from the spec: LED 0 at 20 Hz and LED 50 at 2 kHz means the
    /// first fifty LEDs cover 20–2000 Hz, spread evenly in log frequency.
    #[test]
    fn sectors_spread_evenly_on_the_log_axis() {
        let k = keyframes(&[(0, 20.0), (50, 2000.0), (149, 20_000.0)]);
        assert!((led_frequency(&k, 0.0).unwrap() - 20.0).abs() < 0.01);
        assert!((led_frequency(&k, 50.0).unwrap() - 2000.0).abs() < 1.0);
        assert!((led_frequency(&k, 149.0).unwrap() - 20_000.0).abs() < 1.0);

        // Halfway along the first sector is the geometric mean, not the mean.
        let mid = led_frequency(&k, 25.0).unwrap();
        assert!((mid - (20.0f32 * 2000.0).sqrt()).abs() < 1.0, "midpoint was {mid}");
    }

    #[test]
    fn leds_outside_every_sector_are_dark() {
        let k = keyframes(&[(20, 100.0), (80, 5000.0)]);
        assert!(led_frequency(&k, 0.0).is_none());
        assert!(led_frequency(&k, 19.0).is_none());
        assert!(led_frequency(&k, 20.0).is_some());
        assert!(led_frequency(&k, 80.0).is_some());
        assert!(led_frequency(&k, 81.0).is_none());
    }

    // The two cases the feature was specified by.
    #[test]
    fn mirror_puts_the_low_end_at_both_tips() {
        assert_eq!(source_position(0.0, true, false), 0.0);
        assert_eq!(source_position(1.0, true, false), 0.0);
        assert_eq!(source_position(0.5, true, false), 1.0);
    }

    #[test]
    fn mirror_with_reverse_puts_the_low_end_in_the_middle() {
        assert_eq!(source_position(0.0, true, true), 1.0);
        assert_eq!(source_position(1.0, true, true), 1.0);
        assert_eq!(source_position(0.5, true, true), 0.0);
    }

    #[test]
    fn mirror_is_symmetric_about_the_centre() {
        for i in 0..=100 {
            let u = i as f32 / 100.0;
            let a = source_position(u, true, false);
            let b = source_position(1.0 - u, true, false);
            assert!((a - b).abs() < 1e-6, "asymmetric at {u}: {a} vs {b}");
        }
    }

    #[test]
    fn reverse_alone_flips_end_to_end() {
        assert_eq!(source_position(0.0, false, true), 1.0);
        assert_eq!(source_position(1.0, false, true), 0.0);
    }

    #[test]
    fn level_lookup_interpolates_between_centres() {
        let centers = [100.0, 1000.0, 10_000.0];
        let levels = [0.0, 1.0, 0.0];
        assert_eq!(level_at(&levels, &centers, 100.0), 0.0);
        assert_eq!(level_at(&levels, &centers, 1000.0), 1.0);
        // Geometric midpoint of the first span is halfway up on the log axis.
        let mid = level_at(&levels, &centers, (100.0f32 * 1000.0).sqrt());
        assert!((mid - 0.5).abs() < 1e-3, "log interpolation gave {mid}");

        // Outside the measured range the ends hold rather than extrapolate.
        assert_eq!(level_at(&levels, &centers, 20.0), 0.0);
        assert_eq!(level_at(&levels, &centers, 20_000.0), 0.0);
    }

    #[test]
    fn the_peak_lookup_takes_the_loudest_in_the_span_and_nothing_outside_it() {
        let centers = [100.0, 200.0, 400.0, 800.0];
        let levels = [0.2, 0.9, 0.3, 1.0];
        assert_eq!(peak_level_between(&levels, &centers, 150.0, 500.0), Some(0.9));
        assert_eq!(peak_level_between(&levels, &centers, 90.0, 810.0), Some(1.0));

        // The bounds are inclusive, so a centre sitting exactly on an edge is
        // in — adjacent control points share their edges and a note landing on
        // one must reach both rather than neither.
        assert_eq!(peak_level_between(&levels, &centers, 200.0, 200.0), Some(0.9));

        // Nothing in the span is not the same as silence in it: the caller has
        // to know it got no answer, so it can interpolate instead.
        assert_eq!(peak_level_between(&levels, &centers, 210.0, 390.0), None);
        assert_eq!(peak_level_between(&[], &[], 0.0, 1.0), None);
    }

    fn centers(n: usize) -> Vec<f32> {
        (0..n).map(|i| norm_to_hz(i as f32 / (n - 1) as f32)).collect()
    }

    /// The bug this whole aggregation exists for.
    ///
    /// A note is one grid point wide and the wire is narrower than the grid, so
    /// point-sampling lands beside the peak and reports a quieter band than was
    /// played. The surface's y *is* that level, so the note comes out the wrong
    /// colour rather than merely dimmer — and at spread 0 the sample can miss
    /// it altogether.
    #[test]
    fn a_grid_finer_than_the_frame_keeps_every_peak() {
        let c = centers(88);
        for spike in 0..88 {
            let mut levels = vec![0.0; 88];
            levels[spike] = 1.0;

            for points in [85, 64] {
                let mut map = StripMap::new(
                    LayoutConfig::spanning(150),
                    IntensityConfig::pass_through(),
                    &c,
                    points,
                    150,
                );
                let peak = map.map(&levels).iter().map(|p| p.y).fold(0.0f32, f32::max);
                assert_eq!(peak, 1.0, "grid point {spike} of 88 lost its peak at {points} points");
            }
        }
    }

    /// And only when the frame really is the coarser grid. Interpolation is
    /// what the audio path has always done and what the colour reference is
    /// written against, so a frame with room for every band must still get it.
    #[test]
    fn a_frame_at_least_as_fine_as_the_grid_still_interpolates() {
        let c = centers(3);
        let levels = [0.0, 1.0, 0.0];
        let mut map =
            StripMap::new(LayoutConfig::spanning(150), IntensityConfig::pass_through(), &c, 9, 150);
        let points = map.map(&levels).to_vec();

        // Quarter of the way along is halfway between the first two centres,
        // which interpolation puts at 0.5 and an aggregate would round up to 1.
        assert!((points[2].y - 0.5).abs() < 0.02, "point 2 was at {}", points[2].y);
        assert_eq!(points[0].y, 0.0);
        assert_eq!(points[4].y, 1.0);
    }

    #[test]
    fn default_layout_spans_the_whole_axis_in_order() {
        let c = centers(48);
        let mut map = StripMap::new(
            LayoutConfig::spanning(150),
            IntensityConfig::default(),
            &c,
            48,
            150,
        );
        let points = map.map(&[1.0; 48]).to_vec();
        assert!(points[0].x < 0.01, "first point was at {}", points[0].x);
        assert!(points[47].x > 0.99, "last point was at {}", points[47].x);
        assert!(points.windows(2).all(|w| w[1].x >= w[0].x), "axis was not monotonic");
    }

    #[test]
    fn mirrored_output_is_symmetric() {
        let c = centers(48);
        let layout = LayoutConfig { mirror: true, ..LayoutConfig::spanning(150) };
        let mut map = StripMap::new(layout, IntensityConfig::default(), &c, 48, 150);

        let levels: Vec<f32> = (0..48).map(|i| i as f32 / 47.0).collect();
        let points = map.map(&levels).to_vec();
        for i in 0..48 {
            let (a, b) = (points[i], points[47 - i]);
            assert!((a.x - b.x).abs() < 1e-5, "point {i} broke symmetry");
            assert!((a.gain - b.gain).abs() < 1e-5);
        }
    }

    /// The whole point of the threshold: nothing under it reaches the strip.
    #[test]
    fn quiet_bands_are_cut_by_the_threshold() {
        let c = centers(48);
        let intensity = IntensityConfig { threshold: -40.0, clamp: -10.0, curve: Default::default() };
        let mut map = StripMap::new(LayoutConfig::spanning(150), intensity, &c, 48, 150);

        let points = map.map(&[0.1; 48]).to_vec(); // -72 dB, well under
        assert!(points.iter().all(|p| p.gain == 0.0), "threshold let signal through");

        let points = map.map(&[1.0; 48]).to_vec();
        assert!(points.iter().all(|p| p.gain == 1.0), "clamp did not saturate");
    }

    #[test]
    fn silence_produces_no_light_under_any_layout() {
        let c = centers(48);
        for mirror in [false, true] {
            for reverse in [false, true] {
                let layout = LayoutConfig { mirror, reverse, ..LayoutConfig::spanning(150) };
                let mut map = StripMap::new(layout, IntensityConfig::default(), &c, 48, 150);
                let points = map.map(&[0.0; 48]).to_vec();
                assert!(
                    points.iter().all(|p| p.gain == 0.0),
                    "mirror={mirror} reverse={reverse} lit up on silence"
                );
            }
        }
    }

    #[test]
    fn a_degenerate_keyframe_list_still_lights_the_strip() {
        let c = centers(48);
        for list in [vec![], keyframes(&[(0, 1000.0)])] {
            let layout = LayoutConfig { led_keyframes: list, reverse: false, mirror: false };
            let mut map = StripMap::new(layout, IntensityConfig::default(), &c, 48, 150);
            let points = map.map(&[1.0; 48]).to_vec();
            assert!(points.iter().any(|p| p.gain > 0.0), "fallback layout went dark");
        }
    }

    #[test]
    fn parses_the_editor_wire_format() {
        let json = r#"{"ledKeyframes":[{"id":"a","led":0,"hz":20},{"id":"b","led":149,"hz":20000}],"reverse":true,"mirror":false}"#;
        let layout: LayoutConfig = serde_json::from_str(json).unwrap();
        assert_eq!(layout.led_keyframes.len(), 2);
        assert_eq!(layout.led_keyframes[1].led, 149);
        assert!(layout.reverse && !layout.mirror);
    }
}
