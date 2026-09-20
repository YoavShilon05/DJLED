//! End-to-end: MIDI notes in, LED bytes out.
//!
//! The audio chain has [`pipeline`](../pipeline.rs) for the same job. This one
//! exists because MIDI takes a completely different route to the same place —
//! no FFT, no bands, a grid of semitones instead — and the whole claim being
//! made is that everything downstream cannot tell the difference. That claim is
//! only worth anything if it is tested against the real renderer and the real
//! wire format, which is what this does.
//!
//! It is also the only coverage of the MIDI path that runs without a MIDI port
//! attached: [`NoteEngine`] is deliberately separable from the driver, so notes
//! can be delivered as bytes rather than by playing a keyboard at the CI box.

use djled_engine::color::intensity::IntensityConfig;
use djled_engine::color::surface::Keyframe;
use djled_engine::color::{
    Geometry, LayerVisual, LayoutConfig, RenderConfig, Renderer, SurfaceConfig,
};
use djled_engine::link::protocol::max_bands;
use djled_engine::link::{expand_bands_to_leds, Link, MockLink};
use djled_engine::midi::notes::{DEFAULT_HIGH, DEFAULT_LOW};
use djled_engine::midi::{Message, MidiConfig, NoteEngine};
use djled_engine::show::ShowConfig;

const LEDS: usize = 150;
/// The note engine's frame period, as the run loop drives it.
const DT: f32 = 1.0 / 125.0;

struct Pipeline {
    notes: NoteEngine,
    renderer: Renderer,
    link: MockLink,
    leds: Vec<[u8; 3]>,
}

impl Pipeline {
    fn new() -> Self {
        Self::with(MidiConfig::default(), LayoutConfig::spanning(LEDS))
    }

    fn with(cfg: MidiConfig, layout: LayoutConfig) -> Self {
        Self::driving(cfg, layout, MockLink::new(LEDS))
    }

    /// Built the way the engine builds it: the point count is whatever the
    /// *link* says it will take, never what the protocol theoretically allows.
    fn driving(cfg: MidiConfig, layout: LayoutConfig, link: MockLink) -> Self {
        Self::painting(cfg, layout, link, SurfaceConfig::default())
    }

    fn painting(
        cfg: MidiConfig,
        layout: LayoutConfig,
        link: MockLink,
        surface: SurfaceConfig,
    ) -> Self {
        Self::coloring(cfg, layout, link, surface, Vec::new())
    }

    /// The same, with a colour per MIDI channel over the field. Built through
    /// `Renderer::stacked` because that is the only way into a `LayerVisual`,
    /// which is where a palette lives — and a one-layer stack is byte-identical
    /// to `Renderer::new`, which the rest of this file leans on.
    fn coloring(
        cfg: MidiConfig,
        layout: LayoutConfig,
        link: MockLink,
        surface: SurfaceConfig,
        channel_colors: Vec<String>,
    ) -> Self {
        let notes = NoteEngine::new(&cfg, None, ShowConfig::default().base().decay());
        let visual = LayerVisual {
            channel_colors,
            ..LayerVisual::new(
                surface,
                layout,
                IntensityConfig::pass_through(),
                notes.centers().to_vec(),
            )
        };
        let renderer = Renderer::stacked(
            RenderConfig::default(),
            std::slice::from_ref(&visual),
            notes.centers().len().min(link.max_points()),
            LEDS,
        )
        .unwrap();
        Self { notes, renderer, link, leds: vec![[0; 3]; LEDS] }
    }

    fn send(&mut self, msg: Message) {
        self.notes.handle(msg);
    }

    fn note_on(&mut self, note: u8, velocity: u8) {
        self.send(Message { status: 0x90, data1: note, data2: velocity });
    }

    /// The same note, played on a given channel — 0-based, as the wire has it.
    fn note_on_channel(&mut self, channel: u8, note: u8, velocity: u8) {
        self.send(Message { status: 0x90 | channel, data1: note, data2: velocity });
    }

    /// Note-off at velocity 0: the fastest release there is, and the one the
    /// rest of this file wants when it lets a note go.
    fn note_off(&mut self, note: u8) {
        self.note_off_at(note, 0);
    }

    fn note_off_at(&mut self, note: u8, velocity: u8) {
        self.send(Message { status: 0x80, data1: note, data2: velocity });
    }

    /// Advance the whole chain for `secs`, at the rate the engine runs it.
    fn run(&mut self, secs: f32) {
        for _ in 0..(secs / DT) as usize {
            self.notes.advance(DT);
            // Levels and channels together, as the run loop sends them: they
            // are one answer about one grid point and looking either up without
            // the other is how a note ends up the wrong colour.
            let pixels = self
                .renderer
                .render_stack_with(&[self.notes.levels()], &[self.notes.channels()]);
            self.link.send(pixels).unwrap();
        }
        expand_bands_to_leds(self.renderer.pixels(), &mut self.leds);
    }

    /// The brightest colour on the *wire*, before the firmware spreads the
    /// frame across the run. Where a level claim belongs: the expansion to LEDs
    /// blurs neighbouring control points together by design, so a note narrower
    /// than `leds / points` is smeared there however right its level was.
    fn brightest_point(&self) -> [u8; 3] {
        *self
            .renderer
            .pixels()
            .iter()
            .max_by_key(|px| px.iter().map(|&c| c as u32).sum::<u32>())
            .unwrap()
    }

    fn brightest_led(&self) -> usize {
        self.leds
            .iter()
            .enumerate()
            .max_by_key(|(_, px)| px.iter().map(|&c| c as u32).sum::<u32>())
            .map(|(i, _)| i)
            .unwrap()
    }

    fn lit(&self) -> u32 {
        self.leds.iter().flatten().map(|&c| c as u32).sum()
    }
}

/// The wire format is the constraint MIDI has to live inside: a frame carries
/// at most 255 payload bytes, and a full-range note grid is wider than that.
/// The grid stays at one point per semitone regardless; it is the *frame* that
/// is capped, and the renderer resamples across the axis to fill it.
#[test]
fn a_full_note_range_fits_the_wire_format() {
    let p = Pipeline::new();
    assert_eq!(p.notes.centers().len(), 88, "one grid point per key");
    assert!(p.renderer.pixels().len() <= max_bands());
    assert_eq!(p.link.leds().len(), LEDS);
}

/// The regression this file exists for as much as any of the rest.
///
/// A note grid is wider than the protocol's 85 colours and *much* wider than
/// the 64 the stock sketch is built with, and a firmware that is handed more
/// than its buffer holds stops reading and answers nothing. Every display on
/// the PC keeps working — the spectrum, the strip preview, the terminal bars —
/// because none of them cross the wire, so the only symptom is a strip that
/// stays black while everything says it should not.
///
/// The fix is that the point count comes from what the link says it takes. This
/// pins that: the frame the engine builds for a board with the stock limit must
/// be one that board accepts, with notes still reaching the LEDs through it.
#[test]
fn a_note_grid_fits_the_limit_the_firmware_reports() {
    // What `firmware/djled/djled.ino` ships with.
    const STOCK_MAX_BANDS: usize = 64;

    let mut p = Pipeline::driving(
        MidiConfig::default(),
        LayoutConfig::spanning(LEDS),
        MockLink::with_max_points(LEDS, STOCK_MAX_BANDS),
    );
    assert_eq!(p.notes.centers().len(), 88, "the grid itself is not narrowed");
    assert!(
        p.renderer.pixels().len() <= STOCK_MAX_BANDS,
        "{} colours would be dropped unread by the firmware",
        p.renderer.pixels().len()
    );

    // And the frame is not merely small enough — it still carries the note.
    p.note_on(60, 127);
    p.run(0.4);
    assert!(p.lit() > 0, "a note must still reach the LEDs through the narrowed frame");
}

/// The mock has to refuse what the hardware refuses, or this whole class of bug
/// stays invisible until it is a dark strip in someone's room.
#[test]
fn a_link_refuses_a_frame_past_the_limit_it_reports() {
    let mut link = MockLink::with_max_points(LEDS, 64);
    assert!(link.send(&[[10u8; 3]; 64]).is_ok());

    let err = link.send(&vec![[10u8; 3]; 88]).unwrap_err().to_string();
    assert!(err.contains("64"), "the error should name the limit: {err}");
}

/// Velocity has to reach the *top* of the colour surface, not merely near it.
///
/// The surface's y is the level, so a note that arrives short of the velocity
/// it was played at is painted a different colour rather than a dimmer one —
/// which is what this caught: the note grid is one point per semitone and the
/// wire carries 64, so point-sampling the grid slid off a note's peak and lost
/// up to a quarter of its level. A ramp from black through green to red reads
/// that back as a colour: every note struck as hard as MIDI allows must come
/// out red, whatever the frame the board will take.
///
/// Audio never showed it because a band plan is coarser than the frame, so the
/// frame is never the one doing the resampling. See
/// [`djled_engine::color::strip::peak_level_between`].
#[test]
fn full_velocity_reaches_the_top_of_the_colour_surface_at_every_note() {
    // Black at silence, green halfway, red at full, at both ends of the axis so
    // the field is a pure vertical ramp and colour reports level alone.
    let ramp = SurfaceConfig {
        keyframes: [0.0, 1.0]
            .iter()
            .flat_map(|&x| {
                [
                    Keyframe::new(x, 0.0, "#000000"),
                    Keyframe::new(x, 0.5, "#00ff00"),
                    Keyframe::new(x, 1.0, "#ff0000"),
                ]
            })
            .collect(),
        sigma: 0.25,
    };

    for max_points in [85, 64] {
        for note in DEFAULT_LOW..=DEFAULT_HIGH {
            let mut p = Pipeline::painting(
                MidiConfig::default(),
                LayoutConfig::spanning(LEDS),
                MockLink::with_max_points(LEDS, max_points),
                ramp.clone(),
            );
            p.note_on(note, 127);
            p.run(0.4);

            let px = p.brightest_point();
            assert!(
                px[0] > 4 * px[1],
                "note {note} at full velocity went on the wire as {px:?} through a \
                 {max_points}-point frame — green means its level never reached \
                 the top of the surface"
            );
        }
    }
}

/// The stretch, seen from the far end of the chain: the lowest note lights the
/// start of the strip and the highest lights the end, with nothing left dark at
/// the top the way real pitches would leave it.
#[test]
fn the_note_range_reaches_both_ends_of_the_strip() {
    let mut p = Pipeline::new();
    p.note_on(DEFAULT_LOW, 127);
    p.run(0.4);
    let low = p.brightest_led();
    assert!(low < LEDS / 8, "the lowest note lit LED {low} of {LEDS}");

    let mut p = Pipeline::new();
    p.note_on(DEFAULT_HIGH, 127);
    p.run(0.4);
    let high = p.brightest_led();
    assert!(high > LEDS - LEDS / 8, "the highest note lit LED {high} of {LEDS}");
}

/// The colour surface is sampled by axis position, so a note near the bottom
/// gets the warm end of the palette and one near the top the cool end —
/// exactly as a bass or treble band would.
#[test]
fn low_notes_are_warm_and_high_notes_cool() {
    let mut p = Pipeline::new();
    p.note_on(DEFAULT_LOW + 3, 127);
    p.run(0.4);
    let px = p.leds[p.brightest_led()];
    assert!(px[0] > px[2], "expected warm at the low end, got {px:?}");

    let mut p = Pipeline::new();
    p.note_on(DEFAULT_HIGH - 3, 127);
    p.run(0.4);
    let px = p.leds[p.brightest_led()];
    assert!(px[2] > px[0], "expected cool at the high end, got {px:?}");
}

/// Velocity is the y axis. Two notes at the same pitch and different velocities
/// have to arrive at the strip as different brightnesses, or the axis is a lie.
#[test]
fn velocity_reaches_the_leds_as_brightness() {
    let mut hard = Pipeline::new();
    hard.note_on(60, 127);
    hard.run(0.4);

    let mut soft = Pipeline::new();
    soft.note_on(60, 40);
    soft.run(0.4);

    assert!(
        soft.lit() * 2 < hard.lit(),
        "velocity 40 lit {} against velocity 127's {}",
        soft.lit(),
        hard.lit()
    );
    assert!(soft.lit() > 0, "a soft note must still light something");
}

#[test]
fn silence_leaves_the_strip_dark() {
    let mut p = Pipeline::new();
    p.run(1.0);
    assert_eq!(p.lit(), 0, "no notes must mean no light");
}

/// The gate, end to end: the strip holds while the key is down and goes dark
/// after it comes up. This is the behaviour that separates MIDI from the audio
/// path, where nothing is ever "held".
#[test]
fn a_held_note_holds_the_strip_and_a_released_one_lets_it_go() {
    let mut p = Pipeline::new();
    p.note_on(60, 127);
    p.run(0.2);
    let held = p.lit();
    p.run(2.0);
    // Not exact: the renderer dithers, so a steady level still moves by an LSB
    // or two between frames. A fade would be a different order of magnitude.
    assert!(
        p.lit().abs_diff(held) < held / 50,
        "a held note must not fade: {} after two seconds, from {held}",
        p.lit()
    );

    p.note_off(60);
    p.run(3.0);
    assert_eq!(p.lit(), 0, "a released note must reach true black");
}

/// Two notes an octave apart are two separate places on the strip, and the
/// space between them stays dark. This is the whole reason the grid is per
/// semitone rather than reusing the audio band plan.
#[test]
fn a_chord_lights_separate_places() {
    let mut p = Pipeline::new();
    p.note_on(48, 127);
    p.note_on(72, 127);
    p.run(0.4);

    let led_of = |note: u8| (note - DEFAULT_LOW) as usize * (LEDS - 1) / 87;
    let (low, high) = (led_of(48), led_of(72));
    let bright = |i: usize| p.leds[i].iter().map(|&c| c as u32).sum::<u32>();

    assert!(bright(low) > 40, "the lower note should be lit: {}", bright(low));
    assert!(bright(high) > 40, "the upper note should be lit: {}", bright(high));

    let middle = (low + high) / 2;
    assert!(
        bright(middle) < bright(low) / 2,
        "the gap between two notes an octave apart should stay dark: {}",
        bright(middle)
    );
}

/// Mirror and reverse are spatial transforms applied at the very end, so they
/// have to work on notes for exactly the same reason they work on bands —
/// neither knows what produced the level it is moving.
#[test]
fn mirror_folds_the_note_range_into_both_halves() {
    let layout = LayoutConfig { mirror: true, ..LayoutConfig::spanning(LEDS) };
    let mut p = Pipeline::with(MidiConfig::default(), layout);
    p.note_on(DEFAULT_LOW, 127);
    p.run(0.4);

    let edge = LEDS / 10;
    let start: u32 = p.leds[..edge].iter().flatten().map(|&c| c as u32).sum();
    let end: u32 = p.leds[LEDS - edge..].iter().flatten().map(|&c| c as u32).sum();
    assert!(start > 0 && end > 0, "mirrored, the lowest note lights both ends: {start} / {end}");
}

/// Narrowing the range rescales the whole strip: the same note that sat at the
/// bottom of an 88-key spread lands in the middle of a two-octave one.
#[test]
fn a_narrower_range_respreads_the_same_notes() {
    let cfg = MidiConfig { low_note: 48, high_note: 72, ..Default::default() };
    let mut p = Pipeline::with(cfg, LayoutConfig::spanning(LEDS));
    p.note_on(60, 127);
    p.run(0.4);

    let led = p.brightest_led();
    let middle = LEDS / 2;
    assert!(
        led.abs_diff(middle) < LEDS / 10,
        "C4 in a C3-C5 range should sit near the middle, not at LED {led}"
    );
}

/// The channel palette, end to end: a note played on channel 6 reaches the LED
/// bytes in channel 6's colour, and one played on channel 1 does not.
///
/// The row this file owes the "every editor control reaches the wire" rule. It
/// is a real claim about wiring and not about colour maths — the note engine,
/// the strip map and the renderer each have to carry the channel, and any one
/// of them dropping it leaves a grid that is perfectly lit in the wrong colour.
#[test]
fn a_note_reaches_the_leds_in_its_channel_colour() {
    // A field that paints white everywhere, so anything else on the wire came
    // from the palette rather than from the surface.
    let white = SurfaceConfig {
        keyframes: vec![Keyframe::new(0.0, 0.0, "#ffffff"), Keyframe::new(1.0, 1.0, "#ffffff")],
        sigma: 0.5,
    };
    let mut palette = vec!["#00000000".to_string(); 16];
    palette[5] = "#0000ffff".to_string();

    let mut p = Pipeline::coloring(
        MidiConfig::default(),
        LayoutConfig::spanning(LEDS),
        MockLink::new(LEDS),
        white.clone(),
        palette,
    );
    p.note_on_channel(5, 60, 127);
    p.run(0.4);
    let px = p.brightest_point();
    assert!(px[2] > px[0] && px[2] > px[1], "channel 6 should be blue, got {px:?}");

    // The same note on a channel whose colour is transparent is left to the
    // field — which is the compatibility half of the same claim.
    let mut q = Pipeline::coloring(
        MidiConfig::default(),
        LayoutConfig::spanning(LEDS),
        MockLink::new(LEDS),
        white,
        vec!["#00000000".to_string(); 16],
    );
    q.note_on_channel(0, 60, 127);
    q.run(0.4);
    let px = q.brightest_point();
    assert!(
        px[0].abs_diff(px[2]) < 4 && px[0] > 128,
        "an untinted note keeps the field's white, got {px:?}"
    );
}

/// Two hands at once, on one layer. A chord split across two channels comes out
/// as two colours in two places, which is the thing the whole feature is for.
#[test]
fn two_channels_reach_the_strip_in_two_colours() {
    let white = SurfaceConfig {
        keyframes: vec![Keyframe::new(0.0, 0.0, "#ffffff"), Keyframe::new(1.0, 1.0, "#ffffff")],
        sigma: 0.5,
    };
    let mut palette = vec!["#00000000".to_string(); 16];
    palette[0] = "#ff0000ff".to_string();
    palette[1] = "#0000ffff".to_string();

    let mut p = Pipeline::coloring(
        MidiConfig::default(),
        LayoutConfig::spanning(LEDS),
        MockLink::new(LEDS),
        white,
        palette,
    );
    p.note_on_channel(0, 36, 127);
    p.note_on_channel(1, 96, 127);
    p.run(0.4);

    let led_of = |note: u8| (note - DEFAULT_LOW) as usize * (LEDS - 1) / 87;
    let low = p.leds[led_of(36)];
    let high = p.leds[led_of(96)];
    assert!(low[0] > low[2], "the left hand should be red, got {low:?}");
    assert!(high[2] > high[0], "the right hand should be blue, got {high:?}");
}


/// Release velocity reaches the LED bytes, which is the only place it counts.
///
/// A note let go gently and the same note let go hard are the same note-off
/// message but for one byte, and a second later the wall has to disagree about
/// them — otherwise the control is a slider that moves a number nobody reads.
#[test]
fn release_velocity_decides_how_long_a_note_stays_on_the_wall() {
    let after = |velocity| {
        let mut p = Pipeline::new();
        p.note_on(60, 127);
        p.run(1.0);
        p.note_off_at(60, velocity);
        p.run(0.5);
        p.lit()
    };

    let cut = after(0);
    let middling = after(64);
    let slow = after(127);
    assert_eq!(cut, 0, "velocity 0 is gone, not fading");
    assert!(middling < slow, "{middling} !< {slow}");
}

/// And the control is the top of that range: shorten it and every release
/// shortens with it, including one that asked for the maximum.
#[test]
fn max_decay_time_bounds_the_longest_release() {
    let held_for = |max_decay_ms| {
        let cfg = MidiConfig { max_decay_ms, ..MidiConfig::default() };
        let mut p = Pipeline::with(cfg, LayoutConfig::spanning(LEDS));
        p.note_on(60, 127);
        p.run(1.0);
        p.note_off_at(60, 127);
        p.run(0.5);
        p.lit()
    };
    assert!(held_for(100.0) < held_for(4000.0));
}
