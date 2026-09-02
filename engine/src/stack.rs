//! The running stack: one analyser per layer, one open device per *selection*.
//!
//! # Why the two are pooled differently
//!
//! A layer owns everything about how it hears — its hop, its EQ, its decay, its
//! note range — so it needs an analyser of its own, and analysers are cheap. It
//! does not own the *device*: a WASAPI endpoint opened twice is two captures of
//! the same audio for twice the cost, and a MIDI input opened twice is an
//! error, because Windows hands a port to one application at a time and that
//! application is already this one.
//!
//! So [`crate::source::Feed`]s are keyed by [`Layer::feed_key`] and shared, and
//! analysers are not. Four layers on the default output cost one capture and
//! four analysers, which is the right shape — the capture is the scarce half.
//!
//! One consequence is worth being explicit about: a feed is polled once per
//! pass and every analyser bound to it sees the *same* block of samples or the
//! same run of messages. Anything else would have two layers on one device
//! racing for the ring buffer and each getting half the audio.
//!
//! # What a failure does
//!
//! Opening a device can fail — it is unplugged, or something else holds it —
//! and a stack is exactly the situation where that must not be fatal. A layer
//! whose device will not open still has its place in the stack: it analyses
//! nothing, sits at silence, and carries the reason. Every other layer runs.
//! The strip stays lit and the editor says which row is dark and why.

use anyhow::Result;
use serde::Serialize;

use crate::color::LayerVisual;
use crate::engine::EngineConfig;
use crate::show::{Layer, ShowConfig};
use crate::source::{Analysis, Feed, Source, SourceKind};

/// The rate a layer with no device is analysed at.
///
/// It never sees a sample, so this only sizes a band plan nobody reads. Picking
/// the commonest rate means that if the device does come back, the plan it gets
/// is usually the one it was already going to have.
const ORPHAN_SAMPLE_RATE: f64 = 48_000.0;

/// What the editor is told about one running layer.
///
/// The authored selection is already in the config the editor sent; this is the
/// half only the engine can answer — what that selection actually resolved to,
/// and what went wrong if anything did.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerStatus {
    /// Matches [`Layer::id`], which is how a row finds its telemetry again
    /// after a reorder.
    pub id: String,
    /// The endpoint the selection resolved to. Not redundant with the layer's
    /// own `source`: "the default output" names no device, and you still want
    /// to see which one you got.
    pub device_name: String,
    pub kind: SourceKind,
    /// Channels the device offers, which is what bounds `source.channel`.
    pub channels: usize,
    /// Zero for MIDI, which has no sample rate.
    pub sample_rate: f64,
    /// Points this layer would put on the wire — bands for audio, semitones for
    /// MIDI.
    pub points: usize,
    /// Notes dropped for falling outside the note range. `None` for an audio
    /// layer, which has no notes to drop.
    pub notes_out_of_range: Option<u32>,
    /// Why this layer has no device, or how its stream died. Cleared by the
    /// next success. Every *other* layer keeps running regardless.
    pub error: Option<String>,
}

/// One layer, live.
struct LiveLayer {
    /// The config this layer is currently running, so an edit can be compared
    /// against it rather than reapplied wholesale.
    config: Layer,
    /// Index into [`LiveStack::feeds`], or `None` when the device would not
    /// open. A layer without a feed keeps its analyser and stays at silence
    /// rather than disappearing from the stack.
    feed: Option<usize>,
    analysis: Analysis,
    error: Option<String>,
}

struct FeedSlot {
    /// The selection this feed answers, as authored. Two layers whose
    /// [`Layer::feed_key`] match share this slot.
    key: Source,
    feed: Feed,
}

pub struct LiveStack {
    feeds: Vec<FeedSlot>,
    layers: Vec<LiveLayer>,
    base: EngineConfig,
    buffer_secs: f32,
}

impl LiveStack {
    /// Open every layer of a show.
    ///
    /// Never fails as a whole: a layer whose device will not open is reported
    /// through its [`LayerStatus`] and the rest of the stack still runs. The
    /// alternative — refusing to start because one endpoint is busy — would
    /// make a five-layer show hostage to its least important layer.
    pub fn open(show: &ShowConfig, base: &EngineConfig, buffer_secs: f32) -> Self {
        let mut stack =
            Self { feeds: Vec::new(), layers: Vec::new(), base: base.clone(), buffer_secs };
        stack.rebuild(show);
        stack
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Distinct devices actually open. Two layers on one endpoint count once,
    /// which is the whole point of the pool.
    pub fn feed_count(&self) -> usize {
        self.feeds.len()
    }

    /// Levels for one layer, by position in the stack.
    pub fn levels(&self, index: usize) -> &[f32] {
        self.layers.get(index).map(|l| l.analysis.levels()).unwrap_or(&[])
    }

    /// Where those levels sit on the editor frequency axis.
    pub fn centers(&self, index: usize) -> &[f32] {
        self.layers.get(index).map(|l| l.analysis.centers()).unwrap_or(&[])
    }

    /// Every layer's levels, bottom first — the shape
    /// [`crate::color::Renderer::render_stack`] takes.
    pub fn all_levels(&self) -> Vec<&[f32]> {
        self.layers.iter().map(|l| l.analysis.levels()).collect()
    }

    /// The widest grid any layer produces — 48 audio bands, or 88 semitones of
    /// MIDI. The stack is rendered at this resolution so no layer is resampled
    /// down to something coarser than what it has.
    pub fn points(&self) -> usize {
        self.layers.iter().map(|l| l.analysis.points()).max().unwrap_or(0)
    }

    /// The audio analyser behind one layer, for the diagnostics that only
    /// apply to one — the band plan and its FFT tiers.
    pub fn engine(&self, index: usize) -> Option<&crate::engine::Engine> {
        self.layers.get(index).and_then(|l| l.analysis.engine())
    }

    /// How one layer describes the device it opened, or why it has none.
    pub fn describe(&self, index: usize) -> String {
        let Some(layer) = self.layers.get(index) else { return String::new() };
        match layer.feed.and_then(|i| self.feeds.get(i)) {
            Some(slot) => slot.feed.describe(),
            None => layer
                .error
                .clone()
                .unwrap_or_else(|| format!("{} — not open", layer.config.source.kind.label())),
        }
    }

    /// What to tell the editor about each layer.
    pub fn status(&self) -> Vec<LayerStatus> {
        self.layers
            .iter()
            .map(|l| {
                let feed = l.feed.and_then(|i| self.feeds.get(i)).map(|s| &s.feed);
                LayerStatus {
                    id: l.config.id.clone(),
                    device_name: feed.map(|f| f.device_name().to_string()).unwrap_or_default(),
                    kind: feed.map(|f| f.kind()).unwrap_or(l.config.source.kind),
                    channels: feed.map(|f| f.channels()).unwrap_or(0),
                    sample_rate: feed.map(|f| f.sample_rate()).unwrap_or(0.0),
                    points: l.analysis.points(),
                    notes_out_of_range: l.analysis.out_of_range(),
                    error: l.error.clone(),
                }
            })
            .collect()
    }

    /// What the renderer needs to paint the stack: one visual per layer, bottom
    /// first, each carrying its own axis.
    pub fn visuals(&self) -> Vec<LayerVisual> {
        self.layers
            .iter()
            .map(|l| LayerVisual {
                surface: l.config.surface.clone(),
                layout: l.config.layout(),
                intensity: l.config.intensity(),
                // A layer that has been switched off contributes nothing rather
                // than being dropped from the stack, so the renderer and the
                // editor keep agreeing about which row is which.
                opacity: if l.config.enabled { l.config.opacity() } else { 0.0 },
                centers: l.analysis.centers().to_vec(),
            })
            .collect()
    }

    /// Faults that have surfaced since the last call, as `(layer id, reason)`.
    ///
    /// Checked before anything gated on samples arriving, because a stream that
    /// has died delivers nothing at all — gate the report behind audio and the
    /// only symptom is bars that quietly stop.
    pub fn take_faults(&mut self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for i in 0..self.layers.len() {
            let Some(slot) = self.layers[i].feed else { continue };
            let Some(fault) = self.feeds[slot].feed.take_fault() else { continue };
            self.layers[i].error = Some(fault.clone());
            out.push((self.layers[i].config.id.clone(), fault));
        }
        out
    }

    /// Take in whatever every device has delivered and advance every analyser.
    ///
    /// Each feed is polled exactly once and its block handed to every layer
    /// bound to it, so two layers on one endpoint hear the same audio rather
    /// than half of it each.
    ///
    /// Returns true when *any* layer produced a new frame, which is what tells
    /// the run loop there is something new to render. A stack where one layer
    /// is idle and another is not still renders; the idle one keeps its last
    /// levels.
    pub fn poll(&mut self) -> bool {
        // Every feed is drained whether or not anyone is listening to it. A
        // ring left unread by a muted layer would overflow and then deliver a
        // jump of stale audio the moment it was unmuted.
        for slot in &mut self.feeds {
            slot.feed.poll();
        }

        let mut advanced = false;
        for layer in &mut self.layers {
            let Some(index) = layer.feed else { continue };
            // A layer nobody can see is not analysed at all, so muting one is a
            // real saving rather than a cosmetic one — which matters when the
            // stack is how ideas get auditioned.
            if !layer.config.contributes() {
                continue;
            }
            advanced |= layer.analysis.advance(&self.feeds[index].feed);
        }
        advanced
    }

    /// Adopt a new show.
    ///
    /// Returns true when the *shape* of the stack changed — a layer added,
    /// removed, reordered, moved to a different device, or its grid resized —
    /// which is what tells the caller to rebuild its renderer rather than just
    /// hand it new visuals.
    ///
    /// Everything else is compared against what is already running, because
    /// this is the hot path: dragging a keyframe sends one whole show per
    /// pointer move, and reopening a device or rebuilding an analyser on each
    /// of those would make the editor unusable.
    pub fn apply(&mut self, next: &ShowConfig) -> bool {
        let same_stack = self.layers.len() == next.layers.len()
            && self.layers.iter().zip(&next.layers).all(|(live, cfg)| live.config.id == cfg.id);

        if !same_stack {
            self.rebuild(next);
            return true;
        }

        let mut moved = false;
        for i in 0..self.layers.len() {
            let cfg = &next.layers[i];

            // A device change is the one edit that cannot be applied in place.
            //
            // Deliberately *only* on a change. A layer whose device would not
            // open is not retried here: opening one enumerates every endpoint
            // on the machine, and this runs once per pointer move, so a single
            // unplugged interface would make dragging a keyframe unusable.
            // Retries happen on a rescan instead — see [`Self::retry_failed`],
            // which is the button someone presses after plugging it back in.
            if self.layers[i].config.feed_key() != cfg.feed_key() {
                self.rebind(i, cfg);
                moved = true;
                continue;
            }

            let current = self.layers[i].config.clone();
            moved |= self.layers[i].analysis.apply(cfg, &current, false);
            self.layers[i].config = cfg.clone();
        }

        self.retire_unused_feeds();
        moved
    }

    /// Build the whole stack, reusing every device and analyser that is still
    /// answering the same question.
    ///
    /// Reuse is what keeps a reorder from restarting capture: moving a layer up
    /// the stack changes nothing about what it listens to, and dropping the
    /// stream would cost a gap of audio and a reset floor tracker for a purely
    /// cosmetic edit. Whatever is left over at the end is dropped, which is what
    /// closes a device the last layer using it just moved off.
    fn rebuild(&mut self, show: &ShowConfig) {
        let mut spare_feeds = std::mem::take(&mut self.feeds);
        let mut spare_layers = std::mem::take(&mut self.layers);
        let mut feeds: Vec<FeedSlot> = Vec::new();
        let mut layers: Vec<LiveLayer> = Vec::new();

        for cfg in &show.layers {
            let key = cfg.feed_key();
            let bound =
                bind(&mut feeds, &mut spare_feeds, &key, &cfg.source, self.buffer_secs);

            // The analyser this id was already running, if it is still pointed
            // at the same device.
            let recycled = spare_layers
                .iter()
                .position(|l| l.config.id == cfg.id)
                .map(|p| spare_layers.remove(p))
                .filter(|l| l.feed.is_some() && l.config.feed_key() == key);

            match bound {
                Ok(index) => {
                    let analysis = match recycled {
                        Some(mut previous) => {
                            let current = previous.config.clone();
                            previous.analysis.apply(cfg, &current, false);
                            previous.analysis
                        }
                        None => Analysis::new(
                            cfg,
                            &self.base,
                            feeds[index].feed.kind(),
                            feeds[index].feed.sample_rate(),
                        ),
                    };
                    layers.push(LiveLayer {
                        config: cfg.clone(),
                        feed: Some(index),
                        analysis,
                        error: None,
                    });
                }
                Err(e) => layers.push(LiveLayer {
                    // The analyser still has to exist: the layer still has a
                    // place in the stack and the renderer still asks it for an
                    // axis. It simply never advances, so it sits at silence.
                    analysis: Analysis::new(
                        cfg,
                        &self.base,
                        cfg.source.kind,
                        ORPHAN_SAMPLE_RATE,
                    ),
                    config: cfg.clone(),
                    feed: None,
                    error: Some(format!("{e:#}")),
                }),
            }
        }

        self.feeds = feeds;
        self.layers = layers;
    }

    /// Move one layer to a different device without disturbing any other.
    ///
    /// The new feed is opened before the old is let go, so a device that cannot
    /// be opened leaves the layer where it was rather than trading a working
    /// layer for an error message.
    fn rebind(&mut self, index: usize, cfg: &Layer) {
        let key = cfg.feed_key();

        let bound = if let Some(i) = self.feeds.iter().position(|s| s.key == key) {
            Some(i)
        } else if let Some(i) = self.feeds.iter_mut().position(|s| s.feed.retune(&cfg.source)) {
            self.feeds[i].key = key.clone();
            Some(i)
        } else {
            match Feed::open(&cfg.source, self.buffer_secs) {
                Ok(feed) => {
                    self.feeds.push(FeedSlot { key, feed });
                    Some(self.feeds.len() - 1)
                }
                Err(e) => {
                    // The layer keeps whatever it had. Reported here and nowhere
                    // else, so one dead endpoint costs exactly one dark layer.
                    self.layers[index].error = Some(format!("{e:#}"));
                    self.layers[index].config = cfg.clone();
                    None
                }
            }
        };

        let Some(i) = bound else { return };

        // A different device is a different grid and a different floor, so the
        // analyser starts again rather than spending its first second unwinding
        // a level that no longer exists.
        self.layers[index].analysis =
            Analysis::new(cfg, &self.base, self.feeds[i].feed.kind(), self.feeds[i].feed.sample_rate());
        self.layers[index].feed = Some(i);
        self.layers[index].config = cfg.clone();
        self.layers[index].error = None;
    }

    /// Close whatever nothing points at any more.
    ///
    /// Feeds are addressed by position, so this compacts and reindexes rather
    /// than leaving holes. An open capture nobody reads is a device held away
    /// from every other application on the machine for no reason.
    fn retire_unused_feeds(&mut self) {
        let mut mapping = vec![usize::MAX; self.feeds.len()];
        let mut next = 0usize;
        for (i, slot) in mapping.iter_mut().enumerate() {
            if self.layers.iter().any(|l| l.feed == Some(i)) {
                *slot = next;
                next += 1;
            }
        }
        if next == self.feeds.len() {
            return;
        }

        let mut kept = Vec::with_capacity(next);
        for (i, slot) in std::mem::take(&mut self.feeds).into_iter().enumerate() {
            if mapping[i] != usize::MAX {
                kept.push(slot);
            }
        }
        self.feeds = kept;
        for layer in &mut self.layers {
            layer.feed = layer.feed.map(|i| mapping[i]).filter(|&i| i != usize::MAX);
        }
    }

    /// Try the layers whose devices would not open again.
    ///
    /// For after a rescan, when the interface someone just plugged in is the
    /// reason a layer is dark. Returns false when there is nothing to retry, so
    /// the caller can skip announcing a change that did not happen.
    pub fn retry_failed(&mut self, show: &ShowConfig) -> bool {
        if self.layers.iter().all(|l| l.feed.is_some()) {
            return false;
        }
        self.rebuild(show);
        true
    }

    /// Start every analyser again from silence.
    pub fn reset(&mut self) {
        for layer in &mut self.layers {
            layer.analysis.reset();
        }
    }
}

/// Find a feed for `key` among the ones already kept, move it over from the
/// previous generation, or open a new one.
///
/// A free function rather than a method so it can hold two mutable borrows of
/// different feed vectors at once, which is the whole shape of the operation.
fn bind(
    kept: &mut Vec<FeedSlot>,
    spare: &mut Vec<FeedSlot>,
    key: &Source,
    requested: &Source,
    buffer_secs: f32,
) -> Result<usize> {
    if let Some(i) = kept.iter().position(|s| &s.key == key) {
        return Ok(i);
    }
    if let Some(i) = spare.iter().position(|s| &s.key == key) {
        kept.push(spare.remove(i));
        return Ok(kept.len() - 1);
    }
    // A selection that resolves to a port already open — the same MIDI input
    // under a different name for it — is adopted rather than reopened, which
    // the handle this process is holding would refuse.
    if let Some(i) = spare.iter_mut().position(|s| s.feed.retune(requested)) {
        let mut slot = spare.remove(i);
        slot.key = key.clone();
        kept.push(slot);
        return Ok(kept.len() - 1);
    }

    let feed = Feed::open(requested, buffer_secs)?;
    kept.push(FeedSlot { key: key.clone(), feed });
    Ok(kept.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::show::Layer;

    const BUFFER: f32 = 0.25;

    /// These run against whatever the machine actually has, because the thing
    /// worth testing here is a real device handle being shared rather than
    /// opened twice — which a mock cannot demonstrate. A machine with no audio
    /// endpoint at all returns early rather than failing: there is nothing to
    /// pool, and that is not a defect in this code.
    fn can_capture() -> bool {
        Feed::open(&Source::default_output(), BUFFER).is_ok()
    }

    fn show(layers: Vec<Layer>) -> ShowConfig {
        let json = serde_json::to_string(&ShowConfig { layers }).unwrap();
        serde_json::from_str(&json).unwrap()
    }

    fn layer(id: &str, source: Source) -> Layer {
        Layer { id: id.into(), name: id.into(), source, ..Layer::default() }
    }

    fn missing() -> Source {
        Source { id: Some("wasapi:no-such-device".into()), ..Source::default_output() }
    }

    /// The whole point of the pool. Four layers on one endpoint is one capture
    /// and four analysers — opening the device again would be a second stream
    /// of the same audio for twice the cost, and for a MIDI port it would be an
    /// outright failure.
    #[test]
    fn layers_on_one_device_share_one_feed() {
        if !can_capture() {
            return;
        }
        let show = show(
            (0..4).map(|i| layer(&format!("l{i}"), Source::default_output())).collect(),
        );
        let stack = LiveStack::open(&show, &EngineConfig::default(), BUFFER);

        assert_eq!(stack.len(), 4);
        assert_eq!(stack.feed_count(), 1, "one endpoint was opened more than once");
        assert!(stack.status().iter().all(|s| s.error.is_none()));
    }

    /// Two layers filtering two MIDI channels of one keyboard must resolve to
    /// one handle. Windows gives a MIDI input to one application at a time, so
    /// the second open would be refused by the handle this process already
    /// holds — and the user would be told something else has the port.
    #[test]
    fn midi_channels_share_one_port() {
        let ports = crate::midi::ports();
        let Some(port) = ports.first() else { return };
        let source = |channel| Source {
            id: Some(port.id.clone()),
            kind: SourceKind::Midi,
            channel: Some(channel),
        };
        let show = show(vec![layer("a", source(0)), layer("b", source(9))]);
        let stack = LiveStack::open(&show, &EngineConfig::default(), BUFFER);

        assert_eq!(stack.feed_count(), 1, "one MIDI port was opened twice");
        assert!(
            stack.status().iter().all(|s| s.error.is_none()),
            "a channel filter was treated as a second port: {:?}",
            stack.status()
        );
    }

    /// A stack is exactly the situation where one bad device must not be fatal.
    /// The failing layer keeps its place and carries the reason; every other
    /// layer runs.
    #[test]
    fn a_layer_that_cannot_open_leaves_the_rest_running() {
        if !can_capture() {
            return;
        }
        let show = show(vec![
            layer("good", Source::default_output()),
            layer("bad", missing()),
            layer("also-good", Source::default_output()),
        ]);
        let stack = LiveStack::open(&show, &EngineConfig::default(), BUFFER);

        assert_eq!(stack.len(), 3, "the failing layer lost its place in the stack");
        assert_eq!(stack.feed_count(), 1);

        let status = stack.status();
        assert!(status[0].error.is_none());
        assert!(status[1].error.is_some(), "a device that does not exist opened anyway");
        assert!(status[2].error.is_none(), "one dead layer took a working one with it");

        // It still has an axis, because the renderer will still ask it for one.
        assert!(!stack.centers(1).is_empty());
        assert_eq!(stack.visuals().len(), 3);
    }

    /// A reorder changes nothing about what anything is listening to, so it must
    /// not restart capture — that would cost a gap of audio and a reset floor
    /// tracker for a purely cosmetic edit.
    #[test]
    fn a_reorder_keeps_the_devices_open() {
        if !can_capture() {
            return;
        }
        let a = layer("a", Source::default_output());
        let b = layer("b", Source::default_output());
        let mut stack =
            LiveStack::open(&show(vec![a.clone(), b.clone()]), &EngineConfig::default(), BUFFER);
        assert_eq!(stack.feed_count(), 1);

        let reshaped = stack.apply(&show(vec![b, a]));
        assert!(reshaped, "a reorder must rebuild the renderer, which is built on the order");
        assert_eq!(stack.feed_count(), 1, "a reorder reopened a device");
        assert_eq!(stack.status()[0].id, "b");
        assert!(stack.status().iter().all(|s| s.error.is_none()));
    }

    /// A layer whose device is gone must not be retried on the hot path.
    /// Opening one enumerates every endpoint on the machine, and a config
    /// arrives per pointer move — one unplugged interface would otherwise make
    /// dragging a keyframe unusable.
    #[test]
    fn a_failed_layer_is_not_retried_on_every_edit() {
        if !can_capture() {
            return;
        }
        let mut stack = LiveStack::open(
            &show(vec![layer("a", Source::default_output()), layer("bad", missing())]),
            &EngineConfig::default(),
            BUFFER,
        );

        let mut edited = layer("bad", missing());
        edited.mirror = true;
        let reshaped =
            stack.apply(&show(vec![layer("a", Source::default_output()), edited]));
        assert!(!reshaped, "a cosmetic edit on a dead layer reopened its device");
        assert!(stack.status()[1].error.is_some());

        // A rescan is where the retry belongs, and it does happen there.
        assert!(stack.retry_failed(&show(vec![
            layer("a", Source::default_output()),
            layer("bad", missing()),
        ])));
    }

    /// An open capture nobody reads is a device held away from every other
    /// application on the machine for no reason.
    #[test]
    fn dropping_the_last_layer_on_a_device_closes_it() {
        if !can_capture() {
            return;
        }
        let mut stack = LiveStack::open(
            &show(vec![layer("a", Source::default_output()), layer("b", missing())]),
            &EngineConfig::default(),
            BUFFER,
        );
        assert_eq!(stack.feed_count(), 1);

        stack.apply(&show(vec![layer("b", missing())]));
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.feed_count(), 0, "the device outlived the last layer using it");
    }

    /// An edit that only moves a keyframe must not reopen anything or rebuild an
    /// analyser — this is the hot path, one message per pointer move.
    #[test]
    fn a_cosmetic_edit_is_applied_in_place() {
        if !can_capture() {
            return;
        }
        let base = layer("a", Source::default_output());
        let mut stack = LiveStack::open(&show(vec![base.clone()]), &EngineConfig::default(), BUFFER);

        let mut edited = base.clone();
        edited.mirror = true;
        edited.threshold = -40.0;
        assert!(!stack.apply(&show(vec![edited])), "a colour edit reshaped the stack");
        assert_eq!(stack.feed_count(), 1);
        assert!(stack.visuals()[0].layout.mirror, "the edit did not reach the renderer");
    }

    /// Moving one layer to another device touches that layer and nothing else.
    /// The failure this guards against is a rebind that rebuilds the whole
    /// stack, which would drop a working layer because a different one was
    /// pointed at a device that is not there.
    #[test]
    fn moving_a_layer_to_another_device_rebinds_only_that_layer() {
        if !can_capture() {
            return;
        }
        let mut stack = LiveStack::open(
            &show(vec![
                layer("keep", Source::default_output()),
                layer("move", Source::default_output()),
            ]),
            &EngineConfig::default(),
            BUFFER,
        );
        assert_eq!(stack.feed_count(), 1);

        // To a device that does not exist: the layer reports it and keeps what
        // it had, and the other layer is untouched throughout.
        stack.apply(&show(vec![layer("keep", Source::default_output()), layer("move", missing())]));
        let status = stack.status();
        assert!(status[0].error.is_none(), "rebinding one layer disturbed another");
        assert!(status[1].error.is_some());
    }

    /// A muted layer costs nothing: it is not composited and not analysed. That
    /// has to be true of the analyser too, or muting would be cosmetic.
    #[test]
    fn a_muted_layer_contributes_no_opacity() {
        if !can_capture() {
            return;
        }
        let mut off = layer("off", Source::default_output());
        off.enabled = false;
        let stack = LiveStack::open(
            &show(vec![layer("on", Source::default_output()), off]),
            &EngineConfig::default(),
            BUFFER,
        );

        let visuals = stack.visuals();
        assert_eq!(visuals.len(), 2, "a muted layer was dropped rather than silenced");
        assert_eq!(visuals[1].opacity, 0.0);
    }

    /// The widest grid in the stack, which is what the whole thing renders at —
    /// so no layer is resampled down to something coarser than it has.
    #[test]
    fn points_are_the_widest_grid_in_the_stack() {
        if !can_capture() {
            return;
        }
        let stack = LiveStack::open(
            &show(vec![layer("a", Source::default_output())]),
            &EngineConfig::default(),
            BUFFER,
        );
        assert_eq!(stack.points(), stack.status()[0].points);
        assert!(stack.points() > 0);
    }
}
