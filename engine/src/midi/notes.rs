//! MIDI notes to bar levels — the MIDI half of what the analyser does for audio.
//!
//! # The same two axes
//!
//! The renderer downstream does not know what a "band" is. It asks for a level
//! at a frequency, so anything that can produce *levels over the frequency
//! axis* drives the strip: the colour surface, the LED sectors, mirror, reverse
//! and the intensity curve all work unchanged. Notes map onto that axis on the
//! x, velocity onto level on the y — which is the dB axis the editor draws, so
//! velocity 127 reads as 0 dB and velocity 64 as -40 dB.
//!
//! # Why the note range is stretched
//!
//! A note has a real pitch, and putting it at that pitch would be the obvious
//! choice — the axis is logarithmic, so semitones would come out evenly spaced
//! for free. It is also nearly all wasted strip: an 88-key piano tops out at
//! 4186 Hz, so the last quarter of the wall would never light.
//!
//! So the configured note range is stretched across the whole axis instead: the
//! lowest note sits at 20 Hz and the highest at 20 kHz, with the semitones in
//! between evenly spaced. The grid is one point per semitone, which is finer
//! than any audio band plan and is what makes adjacent notes readable as
//! separate bars.
//!
//! The consequence is worth being explicit about: in MIDI mode the axis is
//! *positions*, not pitches. A colour keyframe at 250 Hz means "a fifth of the
//! way along", not "middle C". The editor relabels the axis with note names to
//! keep that honest.
//!
//! # Envelope
//!
//! A note holds at its velocity while the key is down and releases when it
//! comes up, with the sustain pedal (CC64) latching notes exactly as a piano
//! does. The release uses the editor's decay control, mapped through the same
//! curve the audio ballistics use, so one slider means one thing in both modes.

use serde::{Deserialize, Serialize};

use super::Message;
use crate::color::intensity::{DISPLAY_DB_MAX, DISPLAY_DB_MIN};
use crate::color::strip::norm_to_hz;
use crate::dsp::biquad::coefficient;
use crate::dsp::eq::{EqBand, EqCurve};
use crate::dsp::post::decay_scale;

/// MIDI has 128 notes, and that is not a number that will ever change.
pub const NOTE_COUNT: usize = 128;

/// A0 — the bottom of an 88-key piano.
pub const DEFAULT_LOW: u8 = 21;
/// C8 — the top of an 88-key piano.
pub const DEFAULT_HIGH: u8 = 108;

/// The narrowest range worth stretching across a wall. Below an octave the
/// strip becomes a handful of enormous bars and stops reading as pitch at all.
pub const MIN_SPAN: u8 = 12;

/// Rise time to a struck note. Fast enough to feel instant, slow enough that a
/// hard velocity does not read as a single-frame flicker.
const ATTACK_MS: f32 = 6.0;

/// Release at the reference decay, in the middle of the audio path's tuned
/// bass/treble spread. Pitch does not divide into bass and treble here — a note
/// is a note — so unlike the audio ballistics this is one number.
const BASE_RELEASE_MS: f32 = 200.0;

/// Below this an envelope is snapped to zero. Exponential release never
/// actually reaches zero, and a floor matters for more than tidiness: the EQ is
/// applied to *sounding* notes only, so a note that never quite stopped
/// sounding would leave a boosted region faintly lit forever.
const SILENCE: f32 = 1.0e-3;

/// The dB the editor's vertical axis spans, which is what a gain has to be
/// divided by to become a level offset.
const DISPLAY_SPAN: f32 = DISPLAY_DB_MAX - DISPLAY_DB_MIN;

/// The EQ is a real filter response and needs a rate to be evaluated at. There
/// is no sample rate in MIDI, so the curve is sampled at a nominal one — only
/// its shape across 20 Hz–20 kHz is being used, and that is the same at any
/// rate high enough not to warp the top of the range.
const NOMINAL_SAMPLE_RATE: f32 = 48_000.0;

/// What the editor can change about MIDI.
///
/// Every field defaults, so an editor that predates one still produces a valid
/// config rather than a parse error that would drop the whole edit.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MidiConfig {
    /// Note at the left end of the axis.
    pub low_note: u8,
    /// Note at the right end.
    pub high_note: u8,
    /// How far a note bleeds into its neighbours, in semitones. 0 is one hard
    /// bar per note; around 1 reads as a glow without smearing a chord into a
    /// single blob.
    pub spread: f32,
    /// Honour the sustain pedal (CC64). Off makes a held pedal do nothing,
    /// which is what you want when a controller sends one by accident.
    pub sustain: bool,
}

impl Default for MidiConfig {
    fn default() -> Self {
        Self { low_note: DEFAULT_LOW, high_note: DEFAULT_HIGH, spread: 1.0, sustain: true }
    }
}

impl MidiConfig {
    /// The range as it can actually be used: ordered, inside MIDI's 0..127, and
    /// at least an octave wide. Hostile numbers are clamped rather than trusted,
    /// for the same reason [`crate::ShowConfig::hop`] clamps.
    pub fn range(&self) -> (u8, u8) {
        let max = (NOTE_COUNT - 1) as u8;
        let (lo, hi) = if self.low_note <= self.high_note {
            (self.low_note, self.high_note)
        } else {
            (self.high_note, self.low_note)
        };
        let lo = lo.min(max - MIN_SPAN);
        (lo, hi.clamp(lo + MIN_SPAN, max))
    }

    /// Grid points: one per semitone in the range.
    pub fn points(&self) -> usize {
        let (lo, hi) = self.range();
        (hi - lo) as usize + 1
    }

    pub fn spread(&self) -> f32 {
        if self.spread.is_finite() { self.spread.clamp(0.0, 12.0) } else { 0.0 }
    }
}

/// Note name with octave, in the convention where middle C (note 60) is C4.
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] =
        ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[note as usize % 12], note as i32 / 12 - 1)
}

/// Concert pitch of a note. For the record rather than for the mapping — the
/// axis is stretched, so this is not where the note lands.
pub fn note_hz(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

/// Live note state, rendered onto the strip's frequency axis.
pub struct NoteEngine {
    cfg: MidiConfig,
    low: u8,
    high: u8,
    /// MIDI channel to listen to, 0-based. `None` merges all sixteen.
    channel: Option<u8>,

    /// Velocity a note is holding at, 0 when released.
    gate: [f32; NOTE_COUNT],
    /// Key is physically down.
    held: [bool; NOTE_COUNT],
    /// Key is up but the pedal is holding the note.
    latched: [bool; NOTE_COUNT],
    pedal: bool,
    env: [f32; NOTE_COUNT],

    centers: Vec<f32>,
    levels: Vec<f32>,
    /// EQ gain per grid point, already divided into the level's units.
    eq: Vec<f32>,
    eq_bands: Vec<EqBand>,
    /// Half a Gaussian; index 0 is the note itself.
    weights: Vec<f32>,
    release_ms: f32,

    out_of_range: u32,
}

impl NoteEngine {
    pub fn new(cfg: &MidiConfig, channel: Option<usize>, decay: f32) -> Self {
        let (low, high) = cfg.range();
        let points = cfg.points();
        let mut engine = Self {
            cfg: *cfg,
            low,
            high,
            channel: sanitise_channel(channel),
            gate: [0.0; NOTE_COUNT],
            held: [false; NOTE_COUNT],
            latched: [false; NOTE_COUNT],
            pedal: false,
            env: [0.0; NOTE_COUNT],
            centers: grid_centers(points),
            levels: vec![0.0; points],
            eq: vec![0.0; points],
            eq_bands: Vec::new(),
            weights: kernel(cfg.spread()),
            release_ms: BASE_RELEASE_MS * decay_scale(decay),
            out_of_range: 0,
        };
        engine.resample_eq();
        engine
    }

    /// Axis positions of the grid points, in Hz. Not pitches — see the module
    /// docs; these are where the notes are *drawn*.
    pub fn centers(&self) -> &[f32] {
        &self.centers
    }

    /// Bar brightness per grid point, 0..1.
    pub fn levels(&self) -> &[f32] {
        &self.levels
    }

    pub fn config(&self) -> &MidiConfig {
        &self.cfg
    }

    pub fn range(&self) -> (u8, u8) {
        (self.low, self.high)
    }

    /// Notes seen outside the configured range. They are dropped rather than
    /// piled onto the end bar, and counted so the editor can say so instead of
    /// leaving the user staring at a dark strip.
    pub fn out_of_range(&self) -> u32 {
        self.out_of_range
    }

    /// Notes currently making light, for the status line.
    pub fn sounding(&self) -> usize {
        self.env.iter().filter(|&&e| e > 0.0).count()
    }

    /// Apply a channel voice message. Anything else is ignored.
    pub fn handle(&mut self, msg: Message) {
        if self.channel.is_some_and(|c| c != msg.channel()) {
            return;
        }
        match msg.kind() {
            // A note-on at velocity 0 is a note-off. Every sequencer including
            // FL's MIDI Out uses this, so treating it as a silent note-on would
            // leave notes stuck on forever.
            Message::NOTE_ON if msg.data2 > 0 => self.note_on(msg.data1, msg.data2),
            Message::NOTE_ON | Message::NOTE_OFF => self.note_off(msg.data1),
            Message::CONTROL_CHANGE => match msg.data1 {
                Message::CC_SUSTAIN => self.set_pedal(msg.data2 >= 64),
                Message::CC_ALL_SOUND_OFF | Message::CC_ALL_NOTES_OFF => self.silence(),
                _ => {}
            },
            _ => {}
        }
    }

    /// Advance the envelopes by `dt` seconds and redraw the grid.
    pub fn advance(&mut self, dt: f32) {
        let attack = coefficient(ATTACK_MS / 1000.0, dt);
        let release = coefficient(self.release_ms / 1000.0, dt);

        for note in self.low..=self.high {
            let n = note as usize;
            let gate = self.gate[n];
            // The EQ lands on the target rather than on the output, so it can
            // shape a note that is sounding without ever lifting one that is
            // not — the same reason the audio path puts it before the range map.
            let target = if gate > 0.0 {
                (gate + self.eq[(note - self.low) as usize]).clamp(0.0, 1.0)
            } else {
                0.0
            };

            let env = &mut self.env[n];
            let c = if target > *env { attack } else { release };
            *env += c * (target - *env);
            if gate <= 0.0 && *env < SILENCE {
                *env = 0.0;
            }
        }

        self.render();
    }

    /// Install a new configuration. Returns true when the grid changed shape,
    /// which is what tells the caller its renderer has to be rebuilt.
    pub fn set_config(&mut self, cfg: &MidiConfig) -> bool {
        let (low, high) = cfg.range();
        let moved = (low, high) != (self.low, self.high);
        self.cfg = *cfg;
        self.weights = kernel(cfg.spread());

        if moved {
            let points = cfg.points();
            self.low = low;
            self.high = high;
            self.centers = grid_centers(points);
            self.levels = vec![0.0; points];
            self.eq = vec![0.0; points];
            self.out_of_range = 0;
            self.resample_eq();
            // Cut rather than released. Only notes inside the range are
            // advanced, so an envelope left outside one would never fall — it
            // would sit there and reappear intact the moment the range widened
            // back over it.
            self.silence();
            self.env = [0.0; NOTE_COUNT];
        }
        // A pedal switched off in the editor must not keep holding what it
        // latched while it was on.
        if !cfg.sustain && self.pedal {
            self.set_pedal(false);
        }
        moved
    }

    pub fn set_channel(&mut self, channel: Option<usize>) {
        let next = sanitise_channel(channel);
        if next != self.channel {
            self.channel = next;
            self.silence();
        }
    }

    /// Retune the release, in the same terms the audio ballistics use.
    pub fn set_decay(&mut self, decay: f32) {
        self.release_ms = BASE_RELEASE_MS * decay_scale(decay);
    }

    pub fn set_eq(&mut self, bands: &[EqBand]) {
        self.eq_bands = bands.to_vec();
        self.resample_eq();
    }

    /// Drop every note and every envelope. For a source switch, where carrying
    /// state over would strand whatever was held when the port changed.
    pub fn reset(&mut self) {
        self.silence();
        self.env = [0.0; NOTE_COUNT];
        self.levels.fill(0.0);
        self.out_of_range = 0;
    }

    fn note_on(&mut self, note: u8, velocity: u8) {
        if note < self.low || note > self.high {
            self.out_of_range = self.out_of_range.saturating_add(1);
            return;
        }
        let n = note as usize;
        self.gate[n] = velocity as f32 / 127.0;
        self.held[n] = true;
        self.latched[n] = false;
    }

    fn note_off(&mut self, note: u8) {
        if note < self.low || note > self.high {
            return;
        }
        let n = note as usize;
        self.held[n] = false;
        if self.pedal {
            self.latched[n] = true;
        } else {
            self.gate[n] = 0.0;
        }
    }

    fn set_pedal(&mut self, down: bool) {
        if down && !self.cfg.sustain {
            return;
        }
        self.pedal = down;
        if !down {
            for n in 0..NOTE_COUNT {
                if self.latched[n] {
                    self.latched[n] = false;
                    self.gate[n] = 0.0;
                }
            }
        }
    }

    /// Release everything. Envelopes are left to decay rather than cut, so an
    /// all-notes-off in the middle of a phrase does not snap to black.
    fn silence(&mut self) {
        self.gate = [0.0; NOTE_COUNT];
        self.held = [false; NOTE_COUNT];
        self.latched = [false; NOTE_COUNT];
        self.pedal = false;
    }

    fn resample_eq(&mut self) {
        if self.eq_bands.is_empty() {
            self.eq.fill(0.0);
            return;
        }
        let curve = EqCurve::new(&self.eq_bands, NOMINAL_SAMPLE_RATE);
        for (slot, &hz) in self.eq.iter_mut().zip(&self.centers) {
            *slot = curve.gain_db(hz) / DISPLAY_SPAN;
        }
    }

    /// Spread each envelope over its neighbours, keeping the loudest rather
    /// than summing: two notes a semitone apart should read as two notes, not
    /// as one twice as bright.
    fn render(&mut self) {
        self.levels.fill(0.0);
        let radius = self.weights.len() - 1;
        let last = self.levels.len() - 1;

        for note in self.low..=self.high {
            let level = self.env[note as usize];
            if level <= 0.0 {
                continue;
            }
            let i = (note - self.low) as usize;
            for j in i.saturating_sub(radius)..=(i + radius).min(last) {
                let lit = level * self.weights[j.abs_diff(i)];
                if lit > self.levels[j] {
                    self.levels[j] = lit;
                }
            }
        }
    }
}

fn sanitise_channel(channel: Option<usize>) -> Option<u8> {
    channel.and_then(|c| u8::try_from(c).ok()).filter(|&c| c < 16)
}

/// The stretch: `points` semitones spread evenly across the whole axis, so the
/// lowest note sits at 20 Hz and the highest at 20 kHz.
fn grid_centers(points: usize) -> Vec<f32> {
    if points <= 1 {
        return vec![norm_to_hz(0.5)];
    }
    let last = (points - 1) as f32;
    (0..points).map(|i| norm_to_hz(i as f32 / last)).collect()
}

/// Half a Gaussian, truncated at three sigma where it is worth less than a
/// hundredth of a bar — far below what eight bits can show.
fn kernel(spread: f32) -> Vec<f32> {
    if spread <= 0.0 {
        return vec![1.0];
    }
    let radius = (spread * 3.0).ceil() as usize;
    (0..=radius).map(|d| (-0.5 * (d as f32 / spread).powi(2)).exp()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::strip::{hz_to_norm, F_MAX, F_MIN};
    use crate::dsp::post::REFERENCE_DECAY;

    /// The note engine's frame period, matching what the run loop drives it at.
    const DT: f32 = 1.0 / 125.0;

    fn on(note: u8, velocity: u8) -> Message {
        Message { status: 0x90, data1: note, data2: velocity }
    }

    fn off(note: u8) -> Message {
        Message { status: 0x80, data1: note, data2: 0 }
    }

    fn cc(controller: u8, value: u8) -> Message {
        Message { status: 0xB0, data1: controller, data2: value }
    }

    fn engine() -> NoteEngine {
        NoteEngine::new(&MidiConfig::default(), None, REFERENCE_DECAY)
    }

    fn settle(e: &mut NoteEngine, frames: usize) {
        for _ in 0..frames {
            e.advance(DT);
        }
    }

    /// Index of a note on the default grid.
    fn at(note: u8) -> usize {
        (note - DEFAULT_LOW) as usize
    }

    /// The whole point of stretching: the configured range fills the axis the
    /// strip is mapped against, end to end.
    #[test]
    fn the_note_range_spans_the_whole_axis() {
        let e = engine();
        let centers = e.centers();
        assert_eq!(centers.len(), 88, "an 88-key piano is 88 grid points");
        assert!((centers[0] - F_MIN).abs() < 0.01);
        assert!((centers[87] - F_MAX).abs() < 1.0);

        // Evenly spaced in log frequency, which is what puts one semitone per
        // equal step of strip.
        let step = hz_to_norm(centers[1]) - hz_to_norm(centers[0]);
        for pair in centers.windows(2) {
            let d = hz_to_norm(pair[1]) - hz_to_norm(pair[0]);
            assert!((d - step).abs() < 1e-4, "uneven spacing: {d} vs {step}");
        }
    }

    #[test]
    fn velocity_becomes_level() {
        let mut e = engine();
        e.handle(on(60, 127));
        settle(&mut e, 40);
        assert!(e.levels()[at(60)] > 0.98, "full velocity: {}", e.levels()[at(60)]);

        let mut e = engine();
        e.handle(on(60, 64));
        settle(&mut e, 40);
        assert!((e.levels()[at(60)] - 0.5).abs() < 0.02, "half: {}", e.levels()[at(60)]);
    }

    /// The gate: a held note stays up, and only starts falling when released.
    #[test]
    fn a_held_note_holds_and_a_released_one_decays() {
        let mut e = engine();
        e.handle(on(60, 127));
        settle(&mut e, 200);
        let held = e.levels()[at(60)];
        assert!(held > 0.98, "a held note must not decay: {held}");

        e.handle(off(60));
        e.advance(DT);
        assert!(e.levels()[at(60)] < held, "release should start immediately");

        settle(&mut e, 500);
        assert_eq!(e.levels()[at(60)], 0.0, "release should reach zero, not leave a tail");
    }

    /// Every sequencer sends note-on at velocity 0 for note-off; missing this
    /// leaves notes stuck on forever.
    #[test]
    fn a_zero_velocity_note_on_is_a_note_off() {
        let mut e = engine();
        e.handle(on(60, 100));
        e.advance(DT);
        e.handle(on(60, 0));
        settle(&mut e, 500);
        assert_eq!(e.levels()[at(60)], 0.0);
    }

    #[test]
    fn the_sustain_pedal_holds_a_released_note() {
        let mut e = engine();
        e.handle(cc(64, 127));
        e.handle(on(60, 127));
        settle(&mut e, 60);
        e.handle(off(60));
        settle(&mut e, 60);
        assert!(e.levels()[at(60)] > 0.98, "the pedal should be holding this note");

        e.handle(cc(64, 0));
        settle(&mut e, 500);
        assert_eq!(e.levels()[at(60)], 0.0, "lifting the pedal releases it");
    }

    #[test]
    fn sustain_can_be_switched_off() {
        let cfg = MidiConfig { sustain: false, ..Default::default() };
        let mut e = NoteEngine::new(&cfg, None, REFERENCE_DECAY);
        e.handle(cc(64, 127));
        e.handle(on(60, 127));
        e.advance(DT);
        e.handle(off(60));
        settle(&mut e, 500);
        assert_eq!(e.levels()[at(60)], 0.0);
    }

    #[test]
    fn all_notes_off_releases_everything() {
        let mut e = engine();
        for note in [60, 64, 67] {
            e.handle(on(note, 120));
        }
        e.advance(DT);
        e.handle(cc(123, 0));
        settle(&mut e, 500);
        assert!(e.levels().iter().all(|&l| l == 0.0));
        assert_eq!(e.sounding(), 0);
    }

    /// Two notes a semitone apart must read as two notes. Summing instead of
    /// taking the loudest is what would blow them past full and blur them.
    #[test]
    fn neighbouring_notes_stay_separate() {
        let mut e = engine();
        e.handle(on(60, 127));
        e.handle(on(61, 127));
        settle(&mut e, 40);
        assert!(e.levels()[at(60)] <= 1.0 && e.levels()[at(61)] <= 1.0);
        assert!(e.levels()[at(60)] > 0.98 && e.levels()[at(61)] > 0.98);
    }

    #[test]
    fn spread_lights_the_neighbours_and_zero_does_not() {
        let wide = MidiConfig { spread: 2.0, ..Default::default() };
        let mut e = NoteEngine::new(&wide, None, REFERENCE_DECAY);
        e.handle(on(60, 127));
        settle(&mut e, 40);
        assert!(e.levels()[at(62)] > 0.05, "two semitones out should still glow");
        assert!(e.levels()[at(62)] < e.levels()[at(61)], "and fall off with distance");

        let sharp = MidiConfig { spread: 0.0, ..Default::default() };
        let mut e = NoteEngine::new(&sharp, None, REFERENCE_DECAY);
        e.handle(on(60, 127));
        settle(&mut e, 40);
        assert_eq!(e.levels()[at(61)], 0.0, "no spread means one bar per note");
    }

    /// Dropped, not clamped onto the end bar — and counted, so the editor can
    /// explain a strip that is dark for a reason.
    #[test]
    fn notes_outside_the_range_are_counted_not_piled_on_the_end() {
        let mut e = engine();
        e.handle(on(0, 127));
        e.handle(on(127, 127));
        settle(&mut e, 40);
        assert_eq!(e.out_of_range(), 2);
        assert!(e.levels().iter().all(|&l| l == 0.0));
    }

    #[test]
    fn a_channel_filter_ignores_the_others() {
        let mut e = NoteEngine::new(&MidiConfig::default(), Some(3), REFERENCE_DECAY);
        e.handle(Message { status: 0x93, data1: 60, data2: 127 });
        e.handle(Message { status: 0x95, data1: 64, data2: 127 });
        settle(&mut e, 40);
        assert!(e.levels()[at(60)] > 0.9);
        assert_eq!(e.levels()[at(64)], 0.0);
    }

    /// A hostile or inverted range must not panic or produce an empty grid.
    #[test]
    fn hostile_ranges_are_clamped_not_trusted() {
        let inverted = MidiConfig { low_note: 100, high_note: 20, ..Default::default() };
        assert_eq!(inverted.range(), (20, 100));

        let tiny = MidiConfig { low_note: 60, high_note: 61, ..Default::default() };
        let (lo, hi) = tiny.range();
        assert_eq!(hi - lo, MIN_SPAN);

        let top = MidiConfig { low_note: 127, high_note: 127, ..Default::default() };
        let (lo, hi) = top.range();
        assert!(hi <= 127 && hi - lo >= MIN_SPAN);
        assert!(!NoteEngine::new(&top, None, 0.9).centers().is_empty());
    }

    #[test]
    fn changing_the_range_rebuilds_the_grid_and_reports_it() {
        let mut e = engine();
        assert_eq!(e.centers().len(), 88);
        let narrow = MidiConfig { low_note: 48, high_note: 72, ..Default::default() };
        assert!(e.set_config(&narrow), "a moved range must report a rebuild");
        assert_eq!(e.centers().len(), 25);
        assert_eq!(e.levels().len(), 25);
        assert!(!e.set_config(&narrow), "an unchanged range must not");
    }

    /// A note left outside a narrowed range is cut, not stranded. Only notes
    /// inside the range are advanced, so an envelope left behind would never
    /// fall — and would light up again, intact, when the range widened back.
    #[test]
    fn narrowing_the_range_does_not_strand_a_sounding_note() {
        let mut e = engine();
        e.handle(on(30, 127));
        settle(&mut e, 40);
        assert_eq!(e.sounding(), 1);

        e.set_config(&MidiConfig { low_note: 48, high_note: 72, ..Default::default() });
        settle(&mut e, 40);
        assert_eq!(e.sounding(), 0, "the note is outside the range and must be gone");

        e.set_config(&MidiConfig::default());
        settle(&mut e, 40);
        assert_eq!(e.sounding(), 0, "and must not come back when the range widens");
        assert!(e.levels().iter().all(|&l| l == 0.0));
    }

    /// The EQ shapes notes that are sounding and must never lift silence off
    /// the floor — a boosted region with nothing playing stays dark.
    #[test]
    fn the_eq_shapes_notes_without_lighting_silence() {
        use crate::dsp::eq::EqType;
        let mut e = engine();
        e.set_eq(&[EqBand { kind: EqType::Peak, hz: 1000.0, gain: 12.0, q: 1.0 }]);
        settle(&mut e, 40);
        assert!(e.levels().iter().all(|&l| l == 0.0), "a boost must not light an idle strip");

        e.handle(on(60, 64));
        settle(&mut e, 60);
        let boosted = e.levels()[at(60)];
        assert!(boosted > 0.5, "a note under the boost should be lifted: {boosted}");
    }

    #[test]
    fn note_names_follow_the_middle_c_is_c4_convention() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(21), "A0");
        assert_eq!(note_name(108), "C8");
        assert_eq!(note_name(69), "A4");
        assert!((note_hz(69) - 440.0).abs() < 0.01);
    }
}
