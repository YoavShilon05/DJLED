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
//!
//! # The stack
//!
//! A show is a *stack of layers*, bottom first, composited in order. Everything
//! that used to be a property of the show is now a property of a layer: its own
//! source, its own colour keyframes, its own EQ, decay, sectors, reverse,
//! mirror, thresholds and curve. Two layers reading the same device share one
//! open handle and nothing else — see [`crate::source::LiveStack`].
//!
//! The order of [`ShowConfig::layers`] *is* the stacking order, so a reorder in
//! the editor is a reorder of this vector and nothing more.
//!
//! A config that predates the stack — a saved preset, an older editor — parses
//! as a one-layer show. See the `Deserialize` implementation below.

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};

use crate::color::intensity::{CurveKind, IntensityConfig, IntensityCurve};
use crate::color::strip::{LayoutConfig, LedKeyframe};
use crate::color::{SurfaceConfig, Timeline};
use crate::dsp::eq::EqBand;
use crate::dsp::mrstft::DEFAULT_HOP;
use crate::dsp::post::REFERENCE_DECAY;
use crate::midi::MidiConfig;
use crate::source::{Source, SourceKind};

/// Hop sizes the editor offers. Anything else is clamped into this range —
/// a hop below 64 buries the machine in transforms and one above 8192 is
/// slower than the longest analysis window, which makes it pointless.
pub const MIN_HOP: usize = 64;
pub const MAX_HOP: usize = 8192;

/// One layer of the stack: a source, an analysis of it, and a colour field to
/// paint the result with.
///
/// Every field that shapes what a layer looks like lives here rather than on
/// the show, because the whole point of a stack is that two layers can disagree
/// about all of them — a slow red bass wash under a hard white snare flash is
/// two different decays, two different colour fields, two different sets of
/// sectors, and quite possibly two different devices.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Layer {
    /// The editor's handle on this layer, carried so per-layer telemetry can be
    /// matched back to the right row after a reorder. Blank ids are filled in
    /// on the way in; see [`ShowConfig::normalise`].
    pub id: String,
    /// What the editor calls it. Never interpreted.
    pub name: String,
    /// Off means the layer is skipped entirely: not composited, and its
    /// analysis is not advanced. A muted layer costs nothing.
    pub enabled: bool,
    /// Master opacity, multiplied into every sample's own opacity. This is the
    /// Photoshop slider — it fades the layer toward showing what is underneath,
    /// which is a different thing from fading it toward black.
    pub opacity: f32,
    /// What this layer listens to. Layers naming the same device share one open
    /// handle; they do not share an analyser, so they can still differ in hop,
    /// EQ and decay.
    pub source: Source,
    /// The colour field this layer paints with when it does not animate.
    ///
    /// Where [`Layer::timeline`] has keys they are authoritative and this is
    /// the *degraded* view of them: the editor keeps it equal to the field at
    /// the start of the loop, so a peer that knows nothing about timelines —
    /// an older editor, a preset read by hand — still sees a layer that looks
    /// like itself rather than an empty one. Nothing reads it while there are
    /// keys. See [`crate::color::ColorSurface::for_layer`].
    pub surface: SurfaceConfig,
    /// The same field over time: a loop of whole fields, interpolated. Empty of
    /// keys for a layer that does not animate, which is what every show written
    /// before timelines existed parses as — and byte for byte what such a show
    /// rendered as before.
    ///
    /// Per layer, length and all, because two layers have no reason to agree
    /// about time any more than they do about a decay or a device.
    pub timeline: Timeline,
    /// Join the two ends of this layer's colour field, so a colour that runs
    /// off one end of the strip comes back on the other.
    ///
    /// A property of the layer rather than of a keyframe or of a timeline key:
    /// it says what shape the position axis *is*, and half a field on a circle
    /// with the other half on a segment is not a shape anything could paint.
    /// For the same reason it is not on the timeline — like the sectors and the
    /// EQ, it builds the field rather than being sampled from it.
    ///
    /// See [`crate::color::ColorSurface::cycling`].
    pub cycle: bool,
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
    /// How MIDI notes land on the axis. Carried in the layer rather than beside
    /// the source selection because it is authored, not discovered — the note
    /// range and spread are as much a part of a look as the colours are, and
    /// they should travel with a saved show and survive switching to audio and
    /// back.
    pub midi: MidiConfig,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: true,
            opacity: 1.0,
            source: Source::default_output(),
            surface: SurfaceConfig::default(),
            timeline: Timeline::default(),
            cycle: false,
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

impl Layer {
    /// The default layer with LED sectors spanning a specific strip.
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

    /// Layer opacity, made safe to multiply by. A hostile value clamps rather
    /// than being trusted, for the same reason [`Layer::hop`] clamps.
    pub fn opacity(&self) -> f32 {
        if self.opacity.is_finite() { self.opacity.clamp(0.0, 1.0) } else { 1.0 }
    }

    /// Whether this layer contributes at all. An invisible one is skipped by the
    /// compositor and never advanced by the analyser.
    pub fn contributes(&self) -> bool {
        self.enabled && self.opacity() > 0.0
    }

    /// Which open device this layer's analysis has to be fed from.
    ///
    /// MIDI drops the channel, because the channel is a *filter* applied per
    /// layer by the note engine rather than a property of the open port —
    /// Windows hands a MIDI input to one application at a time, so two layers
    /// watching two channels of one keyboard have to share the one handle.
    /// Audio keeps it: which channel of an interface is analysed is chosen when
    /// the stream is built, so two channels genuinely are two streams.
    pub fn feed_key(&self) -> Source {
        let mut key = self.source.clone();
        if key.kind == SourceKind::Midi {
            key.channel = None;
        }
        key
    }
}

/// The whole show: a stack of layers, bottom first.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowConfig {
    /// Bottom first. Never empty — see [`ShowConfig::normalise`].
    pub layers: Vec<Layer>,
}

impl Default for ShowConfig {
    fn default() -> Self {
        Self { layers: ShowConfig::normalise(vec![Layer::default()]) }
    }
}

impl ShowConfig {
    /// The default with LED sectors spanning a specific strip, which is what
    /// the engine advertises before an editor has ever connected.
    pub fn spanning(led_count: usize) -> Self {
        Self { layers: ShowConfig::normalise(vec![Layer::spanning(led_count)]) }
    }

    /// The bottom layer. Always present, and the whole show for a one-layer
    /// config — which is what every preset written before the stack existed
    /// parses as.
    pub fn base(&self) -> &Layer {
        // Infallible by construction: every path that builds a `ShowConfig`
        // goes through `normalise`, which refuses to leave the stack empty.
        self.layers.first().expect("a show always has at least one layer")
    }

    pub fn layer(&self, id: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// The layers that will actually be composited, bottom first.
    pub fn visible(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter().filter(|l| l.contributes())
    }

    /// Fill in what the editor left out, and refuse to hold an empty stack.
    ///
    /// Ids are the one field the engine both needs and cannot invent
    /// meaningfully — they are how per-layer telemetry finds its way back to
    /// the right row after a reorder — so a blank one gets a positional
    /// stand-in rather than being left to collide with the next blank.
    fn normalise(mut layers: Vec<Layer>) -> Vec<Layer> {
        if layers.is_empty() {
            layers.push(Layer::default());
        }
        for (i, layer) in layers.iter_mut().enumerate() {
            if layer.id.trim().is_empty() {
                layer.id = format!("layer-{i}");
            }
            if layer.name.trim().is_empty() {
                layer.name = format!("Layer {}", i + 1);
            }
        }
        layers
    }
}

/// The flat pre-stack shape, kept parseable forever.
///
/// A show used to *be* a layer, so a config written before the stack existed
/// has these fields at the top level and no `layers` at all. Rather than
/// version the message, both shapes are read: `layers` wins when it is there,
/// and otherwise the flat fields fold into a single layer that behaves exactly
/// as it did. A saved preset therefore keeps working, and so does an editor
/// that has been rolled back — which is the same guarantee `#[serde(default)]`
/// gives field by field, extended to the shape as a whole.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ShowWire {
    layers: Option<Vec<Layer>>,
    surface: Option<SurfaceConfig>,
    eq: Option<Vec<EqBand>>,
    led_keyframes: Option<Vec<LedKeyframe>>,
    reverse: Option<bool>,
    mirror: Option<bool>,
    threshold: Option<f32>,
    clamp: Option<f32>,
    curve: Option<IntensityCurve>,
    decay: Option<f32>,
    sample_length: Option<usize>,
    midi: Option<MidiConfig>,
}

impl<'de> Deserialize<'de> for ShowConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ShowWire::deserialize(deserializer)?;

        if let Some(layers) = wire.layers.filter(|l| !l.is_empty()) {
            return Ok(Self { layers: ShowConfig::normalise(layers) });
        }

        let base = Layer::default();
        let folded = Layer {
            surface: wire.surface.unwrap_or(base.surface),
            eq: wire.eq.unwrap_or(base.eq),
            led_keyframes: wire.led_keyframes.unwrap_or(base.led_keyframes),
            reverse: wire.reverse.unwrap_or(base.reverse),
            mirror: wire.mirror.unwrap_or(base.mirror),
            threshold: wire.threshold.unwrap_or(base.threshold),
            clamp: wire.clamp.unwrap_or(base.clamp),
            curve: wire.curve.unwrap_or(base.curve),
            decay: wire.decay.unwrap_or(base.decay),
            sample_length: wire.sample_length.unwrap_or(base.sample_length),
            midi: wire.midi.unwrap_or(base.midi),
            ..Layer::default()
        };
        Ok(Self { layers: ShowConfig::normalise(vec![folded]) })
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
            "layers": [{
                "id": "l1",
                "name": "Bass",
                "enabled": true,
                "opacity": 0.75,
                "source": { "id": "wasapi:x", "kind": "input", "channel": 0 },
                "surface": { "keyframes": [{"x":0,"y":1,"color":"#ff2000"}], "sigma": 0.25 },
                "eq": [{"id":"eq-1","type":"peak","hz":2000,"gain":6,"q":1.4}],
                "ledKeyframes": [{"id":"a","led":0,"hz":20},{"id":"b","led":149,"hz":20000}],
                "cycle": true,
                "reverse": true,
                "mirror": true,
                "threshold": -55.5,
                "clamp": -12,
                "curve": { "type": "easeIn", "p1": {"x":0.25,"y":0.1}, "p2": {"x":0.25,"y":1} },
                "decay": 0.9,
                "sampleLength": 1024,
                "midi": { "lowNote": 36, "highNote": 96, "spread": 0.5, "sustain": false }
            }]
        }"##;
        let cfg: ShowConfig = serde_json::from_str(json).unwrap();
        let layer = cfg.base();

        assert_eq!(layer.id, "l1");
        assert_eq!(layer.name, "Bass");
        assert_eq!(layer.opacity(), 0.75);
        assert_eq!(layer.source.kind, SourceKind::Input);
        assert_eq!(layer.eq.len(), 1);
        assert_eq!(layer.led_keyframes.len(), 2);
        assert!(layer.reverse && layer.mirror);
        assert!(layer.cycle);
        assert_eq!(layer.threshold, -55.5);
        assert_eq!(layer.curve.kind, CurveKind::EaseIn);
        assert_eq!(layer.hop(), 1024);
        assert_eq!(layer.layout().led_keyframes[1].led, 149);
        assert_eq!(layer.midi.range(), (36, 96));
        assert!(!layer.midi.sustain);
    }

    /// A stack arrives bottom first and stays in that order, because the order
    /// *is* the compositing order — a reorder in the editor is nothing more
    /// than a reorder of this vector.
    #[test]
    fn a_stack_keeps_its_order() {
        let json = r#"{"layers":[{"id":"a"},{"id":"b"},{"id":"c"}]}"#;
        let cfg: ShowConfig = serde_json::from_str(json).unwrap();
        let ids: Vec<&str> = cfg.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(cfg.base().id, "a");
        assert!(cfg.layer("c").is_some());
        assert!(cfg.layer("nope").is_none());
    }

    /// The compatibility guarantee: a show written before the stack existed had
    /// these fields at the top level, and must load as the one-layer show it
    /// always was rather than as a parse error that drops the whole edit.
    #[test]
    fn a_pre_stack_config_folds_into_one_layer() {
        let json = r##"{
            "surface": { "keyframes": [{"x":0,"y":1,"color":"#ff2000"}], "sigma": 0.4 },
            "reverse": true,
            "threshold": -55.5,
            "decay": 0.9,
            "sampleLength": 1024,
            "midi": { "lowNote": 36, "highNote": 96, "spread": 0.5, "sustain": false }
        }"##;
        let cfg: ShowConfig = serde_json::from_str(json).unwrap();

        assert_eq!(cfg.layers.len(), 1);
        let layer = cfg.base();
        assert!(layer.reverse);
        assert_eq!(layer.threshold, -55.5);
        assert_eq!(layer.hop(), 1024);
        assert_eq!(layer.surface.sigma, 0.4);
        assert_eq!(layer.midi.range(), (36, 96));
        // Anything the old shape had no field for takes the layer default.
        assert!(layer.enabled);
        assert_eq!(layer.opacity(), 1.0);
        assert_eq!(layer.source, Source::default_output());
    }

    /// An editor that predates a field must not fail the whole message.
    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let cfg: ShowConfig = serde_json::from_str(r#"{"reverse":true}"#).unwrap();
        assert!(cfg.base().reverse);
        // A peer that predates the flag means "as it always was", which is a
        // strip with two ends rather than a ring.
        assert!(!cfg.base().cycle);
        assert_eq!(cfg.base().decay(), REFERENCE_DECAY);
        assert_eq!(cfg.base().hop(), DEFAULT_HOP);
        assert!(cfg.base().eq.is_empty());
        assert_eq!(cfg.base().midi, MidiConfig::default());

        let cfg: ShowConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.base().threshold, -62.0);
    }

    /// A stack with nothing in it is not a show. Rather than leaving every
    /// caller to handle an empty vector, one default layer is substituted —
    /// which is also what `base()` being infallible rests on.
    #[test]
    fn an_empty_stack_becomes_one_default_layer() {
        let cfg: ShowConfig = serde_json::from_str(r#"{"layers":[]}"#).unwrap();
        assert_eq!(cfg.layers.len(), 1);
        assert_eq!(cfg.base().threshold, -62.0);
    }

    /// Ids are how per-layer telemetry finds its row again after a reorder, so
    /// two blank ones must not collide.
    #[test]
    fn blank_ids_and_names_are_filled_in() {
        let cfg: ShowConfig = serde_json::from_str(r#"{"layers":[{},{},{"id":"kept"}]}"#).unwrap();
        let ids: Vec<&str> = cfg.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["layer-0", "layer-1", "kept"]);
        assert_eq!(cfg.layers[0].name, "Layer 1");
        assert_eq!(cfg.layers[2].name, "Layer 3");
    }

    #[test]
    fn hostile_numbers_are_clamped_not_trusted() {
        let json = r#"{"layers":[{"sampleLength": 1, "decay": 5.0, "opacity": 4.0}]}"#;
        let cfg: ShowConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.base().hop(), MIN_HOP);
        assert!(cfg.base().decay() < 1.0);
        assert_eq!(cfg.base().opacity(), 1.0);

        let json = r#"{"layers":[{"sampleLength": 999999999, "decay": -3.0, "opacity": -1.0}]}"#;
        let cfg: ShowConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.base().hop(), MAX_HOP);
        assert_eq!(cfg.base().decay(), 0.0);
        assert_eq!(cfg.base().opacity(), 0.0);
    }

    /// A layer that cannot be seen is skipped outright, which is what keeps a
    /// muted layer from costing an analyser.
    #[test]
    fn invisible_layers_do_not_contribute() {
        let json = r#"{"layers":[
            {"id":"on"},
            {"id":"muted","enabled":false},
            {"id":"clear","opacity":0}
        ]}"#;
        let cfg: ShowConfig = serde_json::from_str(json).unwrap();
        let visible: Vec<&str> = cfg.visible().map(|l| l.id.as_str()).collect();
        assert_eq!(visible, ["on"]);
    }

    /// Two layers on two MIDI channels of one keyboard must resolve to one open
    /// port, or the second would be refused by the handle the first is holding.
    #[test]
    fn midi_layers_share_a_port_across_channels() {
        let midi = |channel| Layer {
            source: Source { id: Some("midi:x".into()), kind: SourceKind::Midi, channel },
            ..Layer::default()
        };
        assert_eq!(midi(Some(0)).feed_key(), midi(Some(9)).feed_key());

        // Audio is the other way round: a channel is chosen when the stream is
        // built, so two channels really are two streams.
        let audio = |channel| Layer {
            source: Source { id: Some("wasapi:x".into()), kind: SourceKind::Input, channel },
            ..Layer::default()
        };
        assert_ne!(audio(Some(0)).feed_key(), audio(Some(1)).feed_key());
    }

    /// A timeline crosses the wire with the rest of the layer, keys and all.
    #[test]
    fn parses_a_timeline() {
        let json = r##"{
            "layers": [{
                "id": "l1",
                "surface": { "keyframes": [{"x":0,"y":1,"color":"#ff0000"}], "sigma": 0.25 },
                "timeline": {
                    "enabled": true,
                    "length": 2.5,
                    "keys": [
                        { "id": "k0", "at": 0,
                          "surface": { "keyframes": [{"x":0,"y":1,"color":"#ff0000"}], "sigma": 0.25 } },
                        { "id": "k1", "at": 0.5,
                          "surface": { "keyframes": [{"x":1,"y":1,"color":"#0000ff"}], "sigma": 0.4 } }
                    ]
                }
            }]
        }"##;
        let cfg: ShowConfig = serde_json::from_str(json).unwrap();
        let layer = cfg.base();

        assert!(layer.timeline.animates());
        assert_eq!(layer.timeline.length(), 2.5);
        assert_eq!(layer.timeline.keys.len(), 2);
        assert_eq!(layer.timeline.keys[1].at, 0.5);
        assert_eq!(layer.timeline.keys[1].surface.sigma, 0.4);

        // The stored surface mirrors the field the loop starts from, which is
        // what a peer that knows nothing about timelines is shown.
        assert_eq!(layer.surface.keyframes[0].color, "#ff0000");
        assert_eq!(layer.timeline.start().unwrap().keyframes[0].color, "#ff0000");
    }

    /// Every show written before timelines existed is a layer that does not
    /// animate, rather than one that fails to parse or one that starts moving.
    #[test]
    fn a_show_without_a_timeline_does_not_animate() {
        let cfg: ShowConfig = serde_json::from_str(r#"{"layers":[{"id":"a"}]}"#).unwrap();
        assert!(cfg.base().timeline.keys.is_empty());
        assert!(!cfg.base().timeline.animates());

        // And the flat pre-stack shape, for the same reason.
        let cfg: ShowConfig = serde_json::from_str(r#"{"reverse":true}"#).unwrap();
        assert!(!cfg.base().timeline.animates());
    }

    #[test]
    fn survives_a_round_trip() {
        let original = ShowConfig::spanning(150);
        let json = serde_json::to_string(&original).unwrap();
        let back: ShowConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.layers.len(), 1);
        assert_eq!(back.base().led_keyframes.len(), 2);
        assert_eq!(back.base().threshold, original.base().threshold);
        assert_eq!(back.base().sample_length, original.base().sample_length);
        assert_eq!(back.base().id, original.base().id);
    }
}
