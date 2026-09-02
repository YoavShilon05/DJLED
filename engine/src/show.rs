//! Everything the editor can change, in one struct.
//!
//! One message rather than a command per control. The editor already holds the
//! whole configuration as a single object, and sending it whole means the two
//! sides cannot drift into disagreeing about which half of an edit landed.
//! Applying it is cheap: each stage compares against what it already has and
//! only the parts that actually changed are rebuilt.
//!
//! Every field carries `#[serde(default)]`, so an editor that predates a field
//! — or one that has been rolled back — still produces a valid config rather
//! than a parse error that would drop the whole edit.

use serde::{Deserialize, Serialize};

use crate::color::intensity::{CurveKind, IntensityConfig, IntensityCurve};
use crate::color::strip::{LayoutConfig, LedKeyframe};
use crate::color::SurfaceConfig;
use crate::dsp::eq::EqBand;
use crate::dsp::mrstft::DEFAULT_HOP;
use crate::dsp::post::REFERENCE_DECAY;
use crate::midi::MidiConfig;

/// Hop sizes the editor offers. Anything else is clamped into this range —
/// a hop below 64 buries the machine in transforms and one above 8192 is
/// slower than the longest analysis window, which makes it pointless.
pub const MIN_HOP: usize = 64;
pub const MAX_HOP: usize = 8192;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShowConfig {
    pub surface: SurfaceConfig,
    pub eq: Vec<EqBand>,
    pub led_keyframes: Vec<LedKeyframe>,
    pub reverse: bool,
    pub mirror: bool,
    /// dB on the editor's display axis; see [`crate::color::intensity`].
    pub threshold: f32,
    pub clamp: f32,
    pub curve: IntensityCurve,
    /// Fraction of the previous frame a band keeps.
    pub decay: f32,
    /// New samples between analysis frames.
    pub sample_length: usize,
    /// How MIDI notes land on the axis. Carried in the show rather than beside
    /// the source selection because it is authored, not discovered — the note
    /// range and spread are as much a part of a look as the colours are, and
    /// they should travel with a saved show and survive switching to audio and
    /// back.
    pub midi: MidiConfig,
}

impl Default for ShowConfig {
    fn default() -> Self {
        Self {
            surface: SurfaceConfig::default(),
            eq: Vec::new(),
            led_keyframes: Vec::new(),
            reverse: false,
            mirror: false,
            threshold: -62.0,
            clamp: -6.0,
            curve: IntensityCurve { kind: CurveKind::Linear, ..Default::default() },
            decay: REFERENCE_DECAY,
            sample_length: DEFAULT_HOP,
            midi: MidiConfig::default(),
        }
    }
}

impl ShowConfig {
    /// The default with LED sectors spanning a specific strip, which is what
    /// the engine advertises before an editor has ever connected.
    pub fn spanning(led_count: usize) -> Self {
        Self { led_keyframes: LayoutConfig::spanning(led_count).led_keyframes, ..Self::default() }
    }

    pub fn layout(&self) -> LayoutConfig {
        LayoutConfig {
            led_keyframes: self.led_keyframes.clone(),
            reverse: self.reverse,
            mirror: self.mirror,
        }
    }

    pub fn intensity(&self) -> IntensityConfig {
        IntensityConfig { threshold: self.threshold, clamp: self.clamp, curve: self.curve }
    }

    /// Hop in samples, clamped to something the analyser can actually run.
    pub fn hop(&self) -> usize {
        self.sample_length.clamp(MIN_HOP, MAX_HOP)
    }

    pub fn decay(&self) -> f32 {
        if self.decay.is_finite() { self.decay.clamp(0.0, 0.999) } else { REFERENCE_DECAY }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the editor actually sends, including the `id` fields it keeps
    /// for its own bookkeeping and the engine has no use for.
    #[test]
    fn parses_a_full_editor_payload() {
        let json = r##"{
            "surface": { "keyframes": [{"x":0,"y":1,"color":"#ff2000"}], "sigma": 0.25 },
            "eq": [{"id":"eq-1","type":"peak","hz":2000,"gain":6,"q":1.4}],
            "ledKeyframes": [{"id":"a","led":0,"hz":20},{"id":"b","led":149,"hz":20000}],
            "reverse": true,
            "mirror": true,
            "threshold": -55.5,
            "clamp": -12,
            "curve": { "type": "easeIn", "p1": {"x":0.25,"y":0.1}, "p2": {"x":0.25,"y":1} },
            "decay": 0.9,
            "sampleLength": 1024,
            "midi": { "lowNote": 36, "highNote": 96, "spread": 0.5, "sustain": false }
        }"##;
        let cfg: ShowConfig = serde_json::from_str(json).unwrap();

        assert_eq!(cfg.eq.len(), 1);
        assert_eq!(cfg.led_keyframes.len(), 2);
        assert!(cfg.reverse && cfg.mirror);
        assert_eq!(cfg.threshold, -55.5);
        assert_eq!(cfg.curve.kind, CurveKind::EaseIn);
        assert_eq!(cfg.hop(), 1024);
        assert_eq!(cfg.layout().led_keyframes[1].led, 149);
        assert_eq!(cfg.midi.range(), (36, 96));
        assert!(!cfg.midi.sustain);
    }

    /// An editor that predates a field must not fail the whole message.
    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let cfg: ShowConfig = serde_json::from_str(r#"{"reverse":true}"#).unwrap();
        assert!(cfg.reverse);
        assert_eq!(cfg.decay(), REFERENCE_DECAY);
        assert_eq!(cfg.hop(), DEFAULT_HOP);
        assert!(cfg.eq.is_empty());
        assert_eq!(cfg.midi, MidiConfig::default());

        let cfg: ShowConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.threshold, -62.0);
    }

    #[test]
    fn hostile_numbers_are_clamped_not_trusted() {
        let json = r#"{"sampleLength": 1, "decay": 5.0}"#;
        let cfg: ShowConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.hop(), MIN_HOP);
        assert!(cfg.decay() < 1.0);

        let json = r#"{"sampleLength": 999999999, "decay": -3.0}"#;
        let cfg: ShowConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.hop(), MAX_HOP);
        assert_eq!(cfg.decay(), 0.0);
    }

    #[test]
    fn survives_a_round_trip() {
        let original = ShowConfig::spanning(150);
        let json = serde_json::to_string(&original).unwrap();
        let back: ShowConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.led_keyframes.len(), 2);
        assert_eq!(back.threshold, original.threshold);
        assert_eq!(back.sample_length, original.sample_length);
    }
}
