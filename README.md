# DJLED

An audio-reactive LED wall. The PC captures what it is playing — or what an
instrument is playing into it, or the notes a DAW is sending it — maps that onto
a frequency axis, turns each position into a colour, and streams the result to
an Arduino driving a WS2812B strip.

A show is a **stack of layers**. Each one has its own source, its own colours and
its own everything else, and they composite like a Photoshop stack: the top one
paints over the rest, opacity blends with what is underneath, and black covers
it.

```
loopback / input ─┐   ┌─ layer 3 ─┐
                  ├──►├─ layer 2 ─┤─► composite ─► serial ─► Arduino ─► strip
MIDI port ────────┘   └─ layer 1 ─┘       │                  (expands points
   (one open handle           each: levels │ over the axis    across LEDs)
    per device,                → colour surface → intensity)
    shared by layers)                      └──► WebSocket ─► editor UI
```

Audio arrives as frequency bands and MIDI as notes, but both come out as *levels
over the editor's frequency axis*, and nothing downstream of that asks which one
produced them.

## Layout

| Path | What it is |
|---|---|
| `engine/` | Rust. Capture, MIDI, DSP, colour, compositing, wire protocol, UI bridge. |
| `ui/` | React + TypeScript + Mantine. The layer stack and the spectrum editor. |
| `firmware/djled/` | Arduino sketch. |
| `docs/wiring.md` | **Read before powering anything.** |

## Running it

No hardware needed — without `--port` the engine drives a mock link and previews
the strip in the terminal.

```bash
# engine (terminal 1)
cargo run --manifest-path engine/Cargo.toml --release

# editor (terminal 2)
cd ui && npm install && npm run dev
```

With hardware:

```bash
cargo run --manifest-path engine/Cargo.toml --release -- --port COM3
```

Useful flags:

```
--list-ports          find the Arduino
--list-devices        find an audio source
--source UR22         listen to a named device instead of the default output
--input               capture an input rather than what the PC is playing
--midi                listen to MIDI notes instead of audio
--channel 1           one input channel, or one MIDI channel, instead of all
--probe 3             capture for 3s and report what arrived
--test rgb|chase|white   wiring diagnostics, see docs/wiring.md
--bands 48            frequency bands, and colours sent per frame
--brightness 0.5      master brightness
--no-ui               don't open the editor's WebSocket port
--presets PATH        where the twelve presets live
--no-hotkeys          give ctrl+alt+F1..F12 back to whatever else wants them
```

## Presets

Twelve saved shows, and `ctrl+alt+F1`…`F12` to switch between them.

The shows live in the **engine**, not the editor — `%APPDATA%\djled\presets.json`
by default. That is the whole design decision, and it follows from when a preset
is actually switched: mid-set, with the browser behind a DAW or closed
altogether. A shortcut that only works while a particular window has focus is
not a shortcut for that, so the keys are registered with Windows itself
(`RegisterHotKey`) and the store had to move to the side that is always running.

That inverts the usual direction. Everywhere else the editor authors and pushes
down; here the engine is authoritative and the editor adopts what it is serving.
The editor's preset bar selects which slot is live, and *that is also which slot
is being edited* — there is no save button, because every config the editor sends
lands in the live slot as it is sent. Writes are coalesced to at most one every
750 ms, so dragging a keyframe is one file write rather than sixty.

The one asymmetry worth knowing:

- Chosen in the **preset bar**, an empty slot opens the default show. Picking an
  empty chip out of a row of twelve is a deliberate request for a blank canvas.
- Struck as a **hotkey**, an empty slot does nothing. A mis-hit during a set must
  not blank the wall.

If another application already owns one of the combinations, `RegisterHotKey`
refuses it and there is nothing to be done — that application will not give it
up. So the engine names the ones it lost at startup rather than leaving a key
that silently does nothing, which is the hardest kind of fault to diagnose.

A restart comes back on the slot it left on, so an engine that crashed mid-set
puts the same show back on the wall.

## Where the audio comes from

Three ways in, picked from the **Source** dropdown in the editor or the flags
above, and switchable while running. Two of them are audio; MIDI is the third,
and has a section of its own below.

The choice is made **per layer**, so a show can be listening to all three at
once. Layers naming the same device share one open handle — see
[Layers](#layers) — so this costs a capture per distinct device, not per layer.
The flags select the source for the one layer the engine starts with; the stack
is grown in the editor, because a stack is authored rather than typed.

**Loopback** taps a playback endpoint — the mix Windows is already sending to
the speakers. No cable, no Stereo Mix, no microphone: cpal sets
`AUDCLNT_STREAMFLAGS_LOOPBACK` for any device whose data flow is `eRender`, so
it is bit-exact and needs no extra hardware. This is the right source for music.

**Input** taps a capture endpoint — a microphone, a line input, the inputs of an
audio interface. This is the right source for an instrument, and it is the only
one that works while a DAW owns the interface: **an ASIO client bypasses the
render endpoint entirely, so there is nothing on the loopback side to hear.**
Open FL Studio on a Steinberg and the loopback goes quiet; the interface's
*input* keeps working, and a guitar plugged into it lights the wall.

A WASAPI endpoint is one or the other, never both, so an interface appears twice
— usually under the *same name*. That is why the dropdown groups by direction:
the group heading is the only thing telling the two entries apart.

Two smaller things that matter in practice:

- **Channel, not just device.** A guitar in input 1 of a stereo interface exists
  on one channel only; mixing it down with a silent neighbour costs 6 dB and
  adds that neighbour's noise floor. Pick the channel and the other one is never
  read.
- **Commands are drained before audio is read**, on every pass of the loop
  rather than only on passes that complete an analysis frame. A silent source
  delivers no samples at all, and the command that most needs to get through is
  the one moving off a source that has gone silent — gate it behind audio
  arriving and a dead loopback becomes unescapable from the editor.

Switching rebuilds the analyser only when the sample rate actually changes. Band
edges come from the scale config alone, so a rate change moves which FFT tier
each band draws from, never the centre frequencies the strip is mapped against —
which is why the renderer survives untouched.

## MIDI

A MIDI port is a third kind of source, picked from the same dropdown and
switchable while running. Notes land on the x axis and velocity on the y, which
is the same pair of axes audio uses — so the colour surface, the LED sectors,
reverse, mirror, the EQ and the intensity curve all work on notes without
knowing anything has changed.

Two layers can take two *channels* of one port, which is the natural way to
drive a wall from a DAW: one part per layer, each with its own colours and its
own decay. The port is opened once and the channel filter applied per layer,
because Windows would refuse the second open.

### Getting FL Studio into it

A keyboard needs no setup: plug it in, hit rescan, pick it. FL Studio needs one
step, and it is the step everyone gets stuck on:

**The MIDI Out plugin sends to a MIDI *output*. This listens on a MIDI *input*.
Windows has nothing that joins the two.** No amount of clicking in either
application will connect them, and the engine cannot fix it from its side —
creating a virtual MIDI port on Windows means a signed kernel driver, which a
user-space process cannot conjure.

So, once:

1. Install [loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html) and
   create a port in it.
2. In FL: Options → MIDI settings → **Output**, enable that port and note the
   port number it is given.
3. Set the MIDI Out plugin's **Port** knob to that number.

The loopMIDI port then shows up here like any other input. No MIDI hardware is
involved at any point. `--midi --probe 3` is the flag to reach for when nothing
appears to arrive: it separates "the port is wrong" from "the notes are landing
on a channel or in an octave that is being filtered out".

### The note range is stretched, not placed

The obvious mapping is to put a note at its real pitch. The axis is logarithmic,
so semitones would come out evenly spaced for free — and it wastes most of the
wall, because an 88-key piano tops out at 4186 Hz and the last quarter of the
strip would never light.

So the configured note range is stretched across the whole axis instead: the
lowest note at 20 Hz, the highest at 20 kHz, semitones evenly spaced between.
Narrowing the range therefore **magnifies rather than crops** — two octaves
across a wall is a legitimate and very different look.

The consequence worth being explicit about: in MIDI mode the axis is *positions*,
not pitches. A colour keyframe at "250 Hz" means a fifth of the way along. The
editor relabels the axis with note names so it is not quietly lying about it.

| Control | What it does |
|---|---|
| Note range | which notes fill the strip. Notes outside it are dropped, and counted — the editor says how many rather than leaving a dark strip unexplained |
| Note glow | how far a note bleeds into its neighbours, in semitones. 0 is one hard bar per note |
| Sustain pedal | whether CC64 holds released notes lit, as it holds them sounding |
| Decay | the release time after a note is let go — the same control, and the same mapping, as the audio ballistics |

The grid is one point per semitone, which is finer than any audio band plan and
is what keeps adjacent notes readable as separate bars. The *wire* carries fewer:
the protocol allows 85 colours per frame and the stock sketch is built with
`MAX_BANDS 64`, so a full 88-key range is resampled by frequency on the way out.
The engine says so at startup when it happens.

**That resample takes the loudest point in the span, not the level at the
middle of it.** The distinction is the difference between a note coming out the
colour it was played and coming out a different one, and it is MIDI's alone. A
band plan is coarser than the frame, so for audio the frame is interpolating a
grid finer than nothing and point-sampling is exactly right. The note grid is
the other way round: 88 points into 64, with each note about one point wide. A
sample taken between two grid points lands beside the peak, and the worst-placed
note loses a quarter of its level.

A quarter would be a forgivable loss if level were brightness. It is not — level
is the colour surface's *y*, so on a palette that ramps green at half velocity to
red at full, a note struck as hard as MIDI allows comes out green. At note glow 0,
where each note is a single hard point, the sample can miss it altogether. So a
control point that stands for more than one grid point takes the loudest of them,
for the same reason two notes a semitone apart take the loudest rather than
summing: a note is a note, and its velocity is not an average of the silence
around it.

What is left is genuinely only spatial: two notes closer together than
`88 / points` share a control point, and the firmware smooths between control
points as it always has.

**The limit comes from the board, not from the protocol.** The firmware sizes
its receive buffer from its own `MAX_BANDS` and stops reading anything longer,
with nothing to reply on, so an over-long frame is dropped in silence — and every
display on the PC keeps working, because none of them cross the wire. That
failure looks exactly like broken hardware. The handshake carries the real limit
and the geometry is built from it. Raising `MAX_BANDS` in
`firmware/djled/djled.ino` and reflashing gets the resolution back; at 150 LEDs
there is room for the full 85.

### What it does with the messages

Note-on sets the level to velocity and holds it there — a held chord does not
fade under your fingers. Note-off starts the release. Note-on at velocity 0 is
treated as note-off, which every sequencer including FL's relies on. CC64 latches
notes like a piano; CC120 and CC123 release everything. Pitch bend, aftertouch
and program change are ignored.

Two notes a semitone apart take the loudest rather than summing, so they read as
two notes instead of one twice as bright.

## Why the DSP looks the way it does

The design is driven by one problem: with a short-window real-time FFT, low
frequency bars light up even for a static pure tone. Measurement showed several
independent causes, and each fix is documented where it lives.

**Time-frequency uncertainty is the whole story.** Resolving a 40 Hz band takes
a long window; treble does not need one, and paying that latency at 10 kHz is
waste. So every band draws from the *smallest* FFT that can still resolve it —
five tiers from 8192 down to 128 points, assigned at startup from the real
sample rate (`dsp/bands.rs`).

**Pure log band spacing is unbuildable at the bottom.** 60 log bands from 30 Hz
makes the lowest one 3.3 Hz wide, needing a ~29000-point FFT. It also asks for
resolution the ear does not have — critical bandwidth down there is ~100 Hz. The
scale is Bark-like instead: fixed-width below the crossover, log above, joined so
widths never decrease (which is what keeps the tier ladder monotonic).

**DC offset, not leakage, is the usual culprit.** An inaudible 0.01 offset puts
the sub-bass bar at −31 dB, and no window fixes it — DC lands in bin 0–1, and at
short FFT sizes bin 1 *is* the sub-bass band. Two structural defences plus a DC
blocker mean it cannot reach a displayed band (`dsp/dcblock.rs`).

**The fast path nearly reintroduced the bug it was added to avoid.** Bass tiers
are accurate but slow, so a parallel bandpass supplies the attack. A single
biquad leaks a 1 kHz tone into the 52 Hz band at −31 dB; cascading three helps,
but a 300 Hz tone still gets through at −45 dB. The fix was to stop treating it
as a level meter: its output is clamped to a decaying peak-hold of the band's own
STFT magnitude, so it can only accelerate a band the accurate path has already
confirmed (`dsp/fastpath.rs`).

Latency lands around 32 ms for treble. Bass steady-state is slower by physics,
but the transient path puts its *perceived* response near 25 ms.

## Layers

A show is a **stack of layers**, bottom first, composited in order. Everything
that shapes a look belongs to a layer: its own source, its own colour keyframes,
its own EQ, decay, hop, sectors, reverse, mirror, thresholds, curve and note
range. Two layers can be watching two different devices — a slow red bass wash
from the PC output under hard white snare flashes from a MIDI channel — and
neither knows about the other until they meet in strip space.

Drag a row to reorder. The list is drawn **top first**, the reverse of how the
config stores it, because a stack is read top-down: the top row is the one
painting over everything else.

Only master brightness stays outside a layer. It is the power budget for the
whole strip, and a layer that dimmed the ones below it would be a blend mode
rather than a brightness.

### Black blocks, opacity blends

Two layers meet with plain `source-over` in linear light, and the interesting
half is what that means for the two ways a layer can be dark:

- **Black is a colour, and it covers.** A band the threshold has closed is
  painted with its field's colour at zero intensity — black, in the stock
  palette — at whatever opacity that keyframe carries. So an opaque quiet band
  hides what is under it, exactly as an opaque bright one does. The intensity
  curve scales *light*, never coverage, which is what makes this true.
- **Opacity is the absence of a colour, and it reveals.** A layer that should
  show the one below in its quiet regions says so by authoring opacity at the
  bottom of its field, and the layer below shows through there.

That is why colour is stored un-premultiplied and authored as `#rrggbbaa`. Over
an unlit strip the two are indistinguishable — `#ff0000` at half opacity and
`#800000` reach the LEDs as the same bytes — and it is precisely stacking that
tells them apart.

There is a third case that is neither: **an LED no sector reaches is not painted
at all.** Sectors are how a layer is confined to part of the wall, and a layer
covering the first fifty LEDs must not blank the other hundred. "Outside my
sectors" is not a colour.

A new layer is seeded to make this immediate: transparent at the bottom of its
field and coloured at the top, so it lights where there is signal and shows what
is underneath where there is not.

### One device, many layers

Layers naming the same device **share one open handle**. Four layers on the PC
output cost one capture and four analysers, which is the right shape — the
capture is the scarce half:

- A WASAPI endpoint opened twice is two captures of the same audio for twice the
  cost.
- A MIDI input opened twice is an outright failure. Windows hands a port to one
  application at a time, and that application is already this one. So the MIDI
  *channel* is a filter applied per layer by the note engine rather than a
  property of the port — which is what lets two layers take two channels of one
  keyboard.

Audio is the other way round: which channel of an interface is analysed is
chosen when the stream is built, so two channels genuinely are two streams.

A feed is polled once per pass and every analyser bound to it sees the *same*
block. Anything else would have two layers racing for one ring buffer and each
getting half the audio.

**One dead device costs one dark layer.** A layer whose endpoint will not open
keeps its place in the stack, sits at silence, and carries the reason; every
other layer runs and the strip stays lit. The editor says which row is dark and
why. Rescanning retries them, because plugging the missing interface back in is
exactly when someone presses it.

### What the stack costs

The wire does not change. Still one colour per control point, still 149 bytes,
however many layers went into computing them — the whole fold happens on the PC,
which is the same reason LED sectors needed no firmware change. The stack is
rendered at the widest grid any layer has, so a 48-band audio layer under 88
semitones of MIDI is not resampled down to 48.

What it does cost is an analyser per layer, which is why a hidden layer is
skipped outright rather than composited at zero: muting one is a real saving.

The engine reads a config that predates all of this — flat fields, no `layers` —
as the one-layer show it always was, and a one-layer stack is byte-identical to
what the renderer produced before layers existed. Both are pinned by tests.

## The editor

One plot does everything. The x axis is log frequency, the y axis is dB, and the
background is the colour surface — so a bar is not drawn in some arbitrary
accent colour, it is drawn by *revealing* the field it reaches into. The pixel
under the tip of a bar is the colour that band will send to the strip.

The plot shows **one layer at a time**, the one selected in the stack. Every
gizmo on it belongs to a single layer — its keyframes, its sectors, its EQ, its
two rail handles — and six sets of them on one graph would be unreadable and
unclickable. The composited result of the whole stack is the strip preview
underneath, which is the honest place to judge it.

Everything else hangs off those two axes:

| Gizmo | Where | What it sets |
|---|---|---|
| Parametric EQ | on the plot | a gain curve applied before anything else. Double-click to add a band |
| Colour keyframes | on the plot | the surface. Right-click to add, drag to move, `Del` to remove |
| LED keyframes | track under the axis | which LED indices cover which frequencies |
| Threshold / clamp | rail to the right | the dB window: dark below, full brightness above |
| Intensity curve | between those two handles | brightness against level inside that window |

The curve box is deliberately bounded by the two handles: its top edge *is* the
clamp line and its bottom edge *is* the threshold line, so the mapping can be
read straight across from a band rather than mentally rescaled.

The gizmo checkboxes are **visibility, not bypass**. A hidden EQ still shapes
the spectrum and a hidden threshold still cuts; nothing in that panel touches
the signal.

### The EQ

Bands are the RBJ cookbook biquads — bell, two shelves, two pass filters —
evaluated as `|H(e^jw)|` and summed in dB, which is what cascading them does.
The engine works on band magnitudes rather than samples, so it never runs a
filter; the *shape* is the entire specification, and computing it properly
means the engine can use the same coefficients instead of being
reverse-engineered from a drawing. `ui/src/config/eq.test.ts` asserts the
closed-form properties (a bell is exactly its gain at centre, a pass filter is
−3.01 dB at cutoff when `Q = 1/√2`) rather than values captured from a run.

Gain gets its own vertical scale — zero at the centre of the plot, ±24 dB over
the full height — because a gain is not a level and there is no level a "+6 dB
boost" belongs at. The EQ is applied upstream of the plot, so the bars always
show what the LEDs are actually fed.

### Reverse and mirror

Both are spatial: they rearrange where colour lands on the wall and change
nothing about the analysis, so they apply at the very end and are deliberately
invisible to the graph. Mirror folds the whole range into each half — low end at
both tips, high end at the centre — and reverse flips the result, which is what
puts bass in the middle when the two are combined. An even-length strip has no
LED at its centre, so the fold approaches the top of the range rather than
landing on it; `render.test.ts` pins that to one index.

The whole configuration crosses the wire as one `ShowConfig` message, including
on the hot path where dragging a keyframe sends one per pointer move. Each stage
in the engine compares against what it already holds, so applying it is a few
float comparisons — and one message means the two sides cannot end up
disagreeing about which half of an edit landed.

The two strips at the bottom are the check on that: `Preview` is the whole stack
composited locally in the browser, `Engine` is what the wall is actually doing.
They should agree — and with layers they are the only view that shows the result
rather than one contributor to it. `ui/scripts/stack-smoke.mjs` (`npm run smoke`)
drives that seam over the real socket against a running engine.

Styling is Mantine with a theme and no per-component overrides — `src/theme.ts`
holds every colour decision, including the palette the canvas and SVG layers
paint with, and `src/styles.css` is two rules long.

## Where each control acts

Everything in this table is **per layer**; only master brightness is not. The
controls land in three different places, and which one matters:

| Control | Stage | Why there |
|---|---|---|
| Layer order, opacity, enable | `color/render.rs` | the fold itself, applied after every layer has been sampled |
| Source, channel | `stack.rs`, then `source.rs` | it is the signal itself; the stack decides which layers share a device, and a kind change rebuilds that layer's renderer |
| Note range, glow, sustain | `midi/notes.rs` | they define the grid the levels sit on |
| EQ | `dsp/post.rs`, after AGC, before the range map | see below |
| Decay | release ballistics in `dsp/post.rs` | it *is* the release time |
| Frame hop | rebuilds the analyser | changes how often transforms run |
| Threshold, clamp, curve | `color/intensity.rs` | output shaping, not analysis |
| LED sectors, reverse, mirror | `color/strip.rs` | spatial, applied last — and per layer, so sectors also confine a layer to part of the wall |
| Master brightness | `color/render.rs`, after the fold | the power budget for the whole strip, which is why it is the one control not on a layer |

**The EQ sits after the AGC and before the range map**, and both ends are load-
bearing. Before the AGC, an authored boost looks like drift and gets unwound
over the AGC time constant. After the range map — applied to the normalised
level, where zero *is* the floor — a boost would lift silence and make an idle
strip glow. Both are pinned by tests in `dsp/post.rs`.

**"Frame hop" is not an FFT window length.** Every band already draws from the
smallest transform that can resolve it, five tiers from 8192 down to 128, so
there is no single window to set; this is how often those transforms run.

## Spatial mapping without a firmware change

LED sectors, reverse and mirror are per-LED, and the protocol carries per-band
colour. Those look incompatible, but the firmware only ever *spreads N colours
across the strip* — it cannot tell whether colour *j* means "band j" or "strip
position j/N". So the meaning changed and the wire did not: the PC now computes
the whole spatial mapping and sends control points in strip space.

The alternative was per-LED frames, which is 1800 bytes at 600 LEDs and caps the
display at 27 fps, on a board that cannot evaluate a sector table and an Oklab
surface inside an interrupt-disabled strip write.

The cost is spatial resolution: sector boundaries and the mirror fold are
smoothed over `leds / bands` LEDs — three at 150 LEDs, twelve at 600. Raise
`--bands` to sharpen them.

One consequence worth knowing: the surface's x axis is now log frequency rather
than band index, which is what the editor was always drawing. The two agreed
only approximately before.

## Colour

Interpolation happens in Oklab — sRGB drags gradients through mud, HSV hue bands
badly. The keyframe surface uses normalised Gaussian weighting, chosen over RBF
(overshoots, ill-conditioned when two points coincide) and inverse-distance
weighting (flat plateaus at every point).

There is no gamma step on the way to the LEDs, deliberately. WS2812 brightness is
linear in the byte value and Oklab's lightness is already perceptual, so
`Oklab → linear RGB → byte` is complete. The usual reason people add gamma is
that they started from an sRGB value and drove the LED with it directly.

Every colour carries opacity as a fourth channel, authored as `#rrggbbaa` (six
digits still means opaque, so older presets load unchanged). With one layer
composited onto an unlit strip that reads as a dimming — `#ff0000` at half
opacity and `#800000` reach the LEDs as the same bytes — but that is a property
of the backdrop being black, and it ends the moment layers stack: over a blue
layer the first shows purple and the second still shows dark red. That
distinction is what [Layers](#layers) is built on. Two rules make it work rather
than merely look like it does:

- Colour is stored **un-premultiplied**, so a hue means the same thing at any
  opacity and fading a keyframe out and back in returns what was authored.
- Blending weights colour by `w · α` while weighting opacity by `w` alone.
  Without that asymmetry an invisible keyframe would tint its neighbours with a
  colour nobody can see.

### Area of effect

A keyframe can also be told how far it reaches, and past that it contributes
nothing. That is a different question from the blend radius, which is easy to
conflate with it: the blend radius decides how two keyframes that *both* reach a
point share it, and it is one number for the whole layer — so narrowing it to
confine one colour sharpens every other colour at the same time. The area of
effect is per keyframe and decides whether a keyframe is in that conversation at
all.

Two consequences follow, and both are choices rather than fallout:

- **The field need not be covered.** Where nothing reaches — including a layer
  with no keyframes at all, which is now a legal thing to author — the sample is
  *transparent* black, not opaque black. An unreached position has to let the
  layer below through, exactly like a position no LED sector reaches; anything
  else would make an area of effect useless in a stack, which is the only place
  it is interesting.
- **The edge is a fade, not a disc.** Presence holds at 1 across the interior of
  the radius and smoothsteps to zero over the outer 35% of it. A hard cutoff was
  the obvious reading and is wrong on a wall: every other edge in this field is
  smooth, so the one hard line reads as a fault in the strip. Fading over the
  whole radius is wrong the other way, because then full opacity is reached only
  at the exact centre and an opaque keyframe never looks opaque.

There is a subtlety in the second point worth recording, because it is invisible
until you hit it. Normalised weighting is what keeps the blend convex, and it is
also what silently undoes a taper: with one keyframe in reach, the taper appears
in both the numerator and the denominator and cancels exactly, so opacity holds
its authored value right up to the edge and then falls off a cliff into the
baseline. The fix is to carry the taper a second time, *before* normalisation —
as a maximum rather than a sum, so two overlapping areas of effect are covered
where either one covers and not covered twice.

The UI reimplements this in TypeScript to preview without a round trip.
`ui/src/color/reference.json` is generated from the Rust and asserted by tests on
**both** sides, so a divergence fails a test rather than making the editor lie:

```bash
cargo run --manifest-path engine/Cargo.toml --example color_reference \
  > ui/src/color/reference.json
```

## Protocol

The Arduino does no signal processing. Colour is sent **per band**, not per LED:
149 bytes regardless of strip length, where per-LED frames would be 1800 bytes at
600 LEDs and cap the display at 27 fps. The firmware interpolates across the LEDs
between bands.

Flow control matters. FastLED bit-bangs WS2812B with interrupts disabled for the
entire strip write, and the ATmega's UART FIFO holds two bytes — about 40 µs at
500 kbaud. So the board asks for one frame at a time, and the PC computes the
next during the blackout.

## Tests

```bash
cargo test --manifest-path engine/Cargo.toml   # 255
cd ui && npm test && npm run typecheck         # 65
```

The ones worth knowing about live in `engine/tests/artifacts.rs`: they assert the
reported bug stays fixed, and one of them independently computes what a naive
analyser would produce, so the suppression tests are not merely asserting that
nothing ever happens.

`engine/tests/midi_pipeline.rs` makes the same end-to-end claim for notes, and
is the only coverage of the MIDI path that runs without a port attached — the
note engine is deliberately separable from the driver, so notes can be delivered
as bytes rather than by playing a keyboard at the test runner.

`engine/tests/pipeline.rs` covers the other failure mode: a control that is
computed correctly and then dropped on the way to the wire. Every editor control
has an end-to-end test that its effect reaches the LED bytes — mirroring lights
both ends, a sector confines a tone to its own LEDs, a 24 dB cut halves the
output, red under green comes out green — because the unit tests for each stage
all pass whether or not anything is plugged into them.

`engine/tests/presets.rs` makes the same claim for the preset store, and for
the same reason: a preset that reloads with every field intact and still lights
the wall differently would look like a save bug and would not be one. So the
tests render — a show is stored, the file is reloaded, and the LED bytes have to
match those of the show it was saved from.

The layer tests are in three places, because there are three ways a stack can be
wrong. `color/render.rs` pins the fold itself, including that a one-layer stack
is byte-identical to what came before it. `stack.rs` pins the pooling against
whatever devices the machine really has — that a second layer on one endpoint
shares the handle rather than opening it twice is not something a mock can
demonstrate. `ui/src/spectrum/stack.test.ts` makes the same claims about the
browser's copy of the fold, so the Preview strip cannot quietly disagree with the
wall.

The EQ is asserted against closed-form properties on **both** sides — a bell is
exactly its gain at centre, a pass filter is −3.01 dB at cutoff when `Q = 1/√2`,
cascades add in dB — rather than against values captured from a run. A
transcription slip in either implementation's coefficients fails its own tests
instead of being baked in as the new expectation.

## Scaling to 600 LEDs

The 10 m strip will not run on an Uno — 600 LEDs is 1800 bytes of framebuffer in
2048 bytes of SRAM. The sketch catches this at compile time. Use a Mega (drop-in,
~47 fps) or an ESP32 (DMA output, 100 fps+). The PC side does not change: LED
count is a firmware constant reported in the handshake, and all colour decisions
already happen on the PC.
