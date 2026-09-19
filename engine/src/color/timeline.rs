//! A layer's colour field as a function of time.
//!
//! The colour surface answers "what colour is this position, at this level".
//! A timeline adds a third question — "*when*" — by authoring several whole
//! fields and interpolating between them. Each one is a [`TimelineKey`]: a
//! moment in the loop and the field the layer has reached by then. Keyframe
//! positions, colours, opacities, areas of effect and the blend radius all move
//! between one key and the next, so a show can sweep a colour along the wall,
//! open and close an area of effect, or fade one palette into another without
//! anything upstream of the surface knowing time exists.
//!
//! # Why a stack of whole fields, and not a track per property
//!
//! The obvious alternative is the one every animation package uses: a track per
//! animatable value, each with its own keys. It was rejected because the thing
//! being authored here is a *graph*. The editor draws one colour field at a
//! time and every gesture on it — drag a keyframe, pick a colour, widen an area
//! of effect — edits that field as a whole. A key that *is* the graph at a
//! moment therefore needs no second editor and no second mental model: select a
//! key, and the plot is showing it.
//!
//! The cost is that a value which does not change is still stored once per key.
//! At a handful of keyframes and a handful of keys that is a few hundred bytes,
//! against a preset format that stays readable and an editor that needs no
//! curve sheet.
//!
//! # Identity is position in the list
//!
//! Interpolating two fields means knowing which keyframe in one corresponds to
//! which in the other, and the wire format has no id to match on — a
//! [`super::surface::Keyframe`] is deliberately just a point and a colour, so a
//! preset stays hand-readable. Index is that correspondence: the editor adds and
//! removes a colour on *every* key at once, precisely so the nth keyframe means
//! the same thing in all of them.
//!
//! A hand-edited preset can still hold keys of different lengths, so that is
//! given the only sane meaning rather than being rejected: a keyframe authored
//! at one end of a span and not the other fades out across it. Appearing or
//! vanishing between two frames would be a visible pop on the wall, and a
//! parse error would lose the whole edit.
//!
//! # The clock
//!
//! The phase is a function of the wall clock — seconds since the Unix epoch,
//! modulo the loop length — and not of when the show started. Three things
//! follow, and all three are the reason:
//!
//! - The engine and the editor agree on it without it ever crossing the wire.
//!   Both read the same machine's clock, so the Preview strip is showing the
//!   same instant of the loop the wall is.
//! - Switching preset, reconnecting the editor or restarting the engine does
//!   not restart every layer's loop. A show that is up on the wall mid-set does
//!   not lurch when something unrelated is touched.
//! - Two layers given the same length stay in step for as long as they exist,
//!   with no transport to keep them there.

use serde::{Deserialize, Serialize};

use super::surface::SurfaceConfig;

/// Shortest loop worth running. Below this the field moves faster than the
/// strip is written and the result is noise rather than motion.
pub const MIN_LENGTH: f32 = 0.05;

/// Longest. Ten minutes is past any set's worth of one loop, and the cap is
/// here so a hostile or mistyped value cannot park a layer on one key forever
/// in a way that looks like the timeline is broken.
pub const MAX_LENGTH: f32 = 600.0;

/// What a layer's field has become by one moment of the loop.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineKey {
    /// The editor's handle on this key. Never interpreted here; carried so a
    /// selection survives a reorder, exactly as [`crate::show::Layer::id`] is.
    #[serde(default)]
    pub id: String,
    /// Where in the loop this field is reached, as a fraction of the length.
    ///
    /// Normalised rather than in seconds so that changing the length stretches
    /// the whole animation instead of clipping the end off it — which is what
    /// someone matching a loop to a tempo means by making it shorter.
    #[serde(default)]
    pub at: f32,
    #[serde(default)]
    pub surface: SurfaceConfig,
}

impl TimelineKey {
    pub fn new(at: f32, surface: SurfaceConfig) -> Self {
        Self { id: String::new(), at, surface }
    }
}

/// A layer's timeline. Empty of keys for a layer that does not animate, which
/// is what every show written before this existed parses as.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Timeline {
    /// Off holds the field at the first key rather than dropping the keys, so
    /// an animation can be auditioned against its own starting state and put
    /// back without losing anything.
    pub enabled: bool,
    /// Loop length in seconds. Per layer and independent: a slow colour wash
    /// under a fast strobe is two lengths, and nothing about one constrains the
    /// other.
    pub length: f32,
    /// In authored order, which the editor keeps sorted by [`TimelineKey::at`].
    /// Compilation sorts again rather than trusting it — a hand-edited preset
    /// is not obliged to be tidy, and an out-of-order key would otherwise play
    /// backwards.
    pub keys: Vec<TimelineKey>,
}

impl Default for Timeline {
    fn default() -> Self {
        // Eight seconds is long enough to read as a sweep rather than a flash
        // at any sensible number of keys, and the length is the first thing
        // anyone changes anyway. No keys, so the default animates nothing.
        Self { enabled: true, length: 8.0, keys: Vec::new() }
    }
}

impl Timeline {
    /// Loop length in seconds, made safe to divide by.
    pub fn length(&self) -> f32 {
        if self.length.is_finite() { self.length.clamp(MIN_LENGTH, MAX_LENGTH) } else { 8.0 }
    }

    /// Whether this timeline actually moves. One key is a still field, and so
    /// is a timeline switched off.
    pub fn animates(&self) -> bool {
        self.enabled && self.keys.len() >= 2
    }

    /// The key the loop starts from — the earliest, not merely the first in the
    /// list.
    ///
    /// This is also the field a layer shows when its timeline is switched off,
    /// and the one [`crate::show::Layer::surface`] mirrors for the benefit of a
    /// peer that does not know about timelines at all.
    pub fn start(&self) -> Option<&SurfaceConfig> {
        self.keys
            .iter()
            .filter(|k| k.at.is_finite())
            .min_by(|a, b| a.at.total_cmp(&b.at))
            .map(|k| &k.surface)
    }

    /// Where in the loop a moment in time falls, 0..1.
    ///
    /// Takes seconds since the epoch as `f64` and does the modulo there: at
    /// nearly two billion seconds an `f32` cannot resolve a frame, so
    /// converting first would quantise the whole animation to a standstill.
    pub fn phase(&self, seconds: f64) -> f32 {
        let length = self.length() as f64;
        let mut phase = (seconds % length) / length;
        if phase < 0.0 {
            phase += 1.0;
        }
        phase as f32
    }

    /// The same timeline with every key's field projected onto the top row.
    ///
    /// A layer listening to nothing is sampled at a flat full level, so only
    /// one row of its field is ever read — see [`SurfaceConfig::flattened`],
    /// which this is the whole-timeline form of. A still colour that animates is
    /// exactly the combination that needs it: no signal, and a field that still
    /// has to move.
    pub fn flattened(&self) -> Self {
        Self {
            enabled: self.enabled,
            length: self.length,
            keys: self
                .keys
                .iter()
                .map(|k| TimelineKey {
                    id: k.id.clone(),
                    at: k.at,
                    surface: k.surface.flattened(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::surface::Keyframe;

    fn key(at: f32, color: &str) -> TimelineKey {
        TimelineKey::new(
            at,
            SurfaceConfig { keyframes: vec![Keyframe::new(0.5, 0.5, color)], sigma: 0.25 },
        )
    }

    #[test]
    fn a_timeline_with_fewer_than_two_keys_does_not_animate() {
        assert!(!Timeline::default().animates());
        assert!(!Timeline { keys: vec![key(0.0, "#ff0000")], ..Default::default() }.animates());
        assert!(
            Timeline { keys: vec![key(0.0, "#ff0000"), key(0.5, "#0000ff")], ..Default::default() }
                .animates()
        );
    }

    /// Switching it off holds the first field rather than discarding the rest,
    /// which is what makes auditioning an animation against its own starting
    /// state a toggle instead of an undo.
    #[test]
    fn switching_it_off_leaves_the_keys_alone() {
        let tl = Timeline {
            enabled: false,
            keys: vec![key(0.0, "#ff0000"), key(0.5, "#0000ff")],
            ..Default::default()
        };
        assert!(!tl.animates());
        assert_eq!(tl.keys.len(), 2);
        assert_eq!(tl.start().unwrap().keyframes[0].color, "#ff0000");
    }

    /// The loop starts at the earliest key, not at whichever one was written
    /// first — a hand-edited preset is under no obligation to be in order.
    #[test]
    fn the_loop_starts_at_the_earliest_key() {
        let tl = Timeline {
            keys: vec![key(0.75, "#0000ff"), key(0.1, "#ff0000")],
            ..Default::default()
        };
        assert_eq!(tl.start().unwrap().keyframes[0].color, "#ff0000");
    }

    #[test]
    fn hostile_lengths_are_clamped_not_trusted() {
        assert_eq!(Timeline { length: 0.0, ..Default::default() }.length(), MIN_LENGTH);
        assert_eq!(Timeline { length: -4.0, ..Default::default() }.length(), MIN_LENGTH);
        assert_eq!(Timeline { length: 1e9, ..Default::default() }.length(), MAX_LENGTH);
        assert_eq!(Timeline { length: f32::NAN, ..Default::default() }.length(), 8.0);
    }

    /// The phase has to keep resolving a frame at nearly two billion seconds
    /// since the epoch, which is the whole reason the modulo happens in `f64`.
    #[test]
    fn the_phase_still_moves_at_a_real_wall_clock_time() {
        let tl = Timeline { length: 4.0, ..Default::default() };
        let now = 1_750_000_000.0_f64;
        let a = tl.phase(now);
        let b = tl.phase(now + 0.05);
        assert!((b - a - 0.0125).abs() < 1e-4, "phase barely moved: {a} then {b}");
    }

    #[test]
    fn the_phase_wraps_and_is_never_negative() {
        let tl = Timeline { length: 2.0, ..Default::default() };
        assert!((tl.phase(0.0) - 0.0).abs() < 1e-6);
        assert!((tl.phase(1.0) - 0.5).abs() < 1e-6);
        assert!((tl.phase(2.0) - 0.0).abs() < 1e-6);
        assert!((tl.phase(4.5) - 0.25).abs() < 1e-6);
        assert!(tl.phase(-0.5) >= 0.0 && tl.phase(-0.5) <= 1.0);
    }

    /// A still layer that animates is the case this exists for: no signal to
    /// read a level from, and a field that still has to move.
    #[test]
    fn flattening_projects_every_key() {
        let tl = Timeline {
            keys: vec![
                TimelineKey::new(
                    0.0,
                    SurfaceConfig {
                        keyframes: vec![Keyframe::new(0.2, 0.0, "#ff0000")],
                        sigma: 0.25,
                    },
                ),
                TimelineKey::new(
                    0.5,
                    SurfaceConfig {
                        keyframes: vec![Keyframe::new(0.8, 0.3, "#0000ff")],
                        sigma: 0.25,
                    },
                ),
            ],
            ..Default::default()
        };
        let flat = tl.flattened();
        assert!(flat.keys.iter().all(|k| k.surface.keyframes.iter().all(|f| f.y == 1.0)));
        // The x positions are untouched: it is the level axis that does not
        // exist, not the strip.
        assert_eq!(flat.keys[1].surface.keyframes[0].x, 0.8);
    }

    #[test]
    fn survives_a_round_trip() {
        let tl = Timeline {
            enabled: true,
            length: 3.5,
            keys: vec![key(0.0, "#ff0000"), key(0.5, "#0000ff")],
        };
        let json = serde_json::to_string(&tl).unwrap();
        assert!(json.contains("\"length\":3.5"), "{json}");
        let back: Timeline = serde_json::from_str(&json).unwrap();
        assert_eq!(back.keys.len(), 2);
        assert_eq!(back.keys[1].at, 0.5);
    }

    /// An editor that predates a field, or a preset written by hand, must not
    /// fail the whole message.
    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let tl: Timeline = serde_json::from_str("{}").unwrap();
        assert!(tl.keys.is_empty());
        assert!(tl.enabled);

        let tl: Timeline = serde_json::from_str(r#"{"length":2,"keys":[{"at":0},{"at":1}]}"#)
            .unwrap();
        assert_eq!(tl.length(), 2.0);
        // A key with no field of its own takes the default palette, for the
        // same reason every other missing field does: a key that cannot be
        // parsed would drop the whole show, and a key that paints the stock
        // colours is visibly wrong in a way somebody can fix.
        assert_eq!(tl.keys[0].surface.keyframes.len(), SurfaceConfig::default().keyframes.len());
    }
}
