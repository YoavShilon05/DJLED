# DJLED

An audio-reactive LED wall. The PC captures whatever it is playing, analyses the
spectrum, maps each frequency band to a colour, and streams the result to an
Arduino driving a WS2812B strip.

```
WASAPI loopback ─► analysis ─► colour surface ─► serial ─► Arduino ─► strip
                       │                                      (expands bands
                       └──────► WebSocket ─► editor UI         across LEDs)
```

## Layout

| Path | What it is |
|---|---|
| `engine/` | Rust. Capture, DSP, colour, wire protocol, UI bridge. |
| `ui/` | React + TypeScript + Mantine. The spectrum editor. |
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
--probe 3             capture for 3s and report what arrived
--test rgb|chase|white   wiring diagnostics, see docs/wiring.md
--bands 48            frequency bands, and colours sent per frame
--brightness 0.5      master brightness
--no-ui               don't open the editor's WebSocket port
```

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

## The editor

One plot does everything. The x axis is log frequency, the y axis is dB, and the
background is the colour surface — so a bar is not drawn in some arbitrary
accent colour, it is drawn by *revealing* the field it reaches into. The pixel
under the tip of a bar is the colour that band will send to the strip.

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

The two strips at the bottom are the check on that: `Preview` is the config
applied locally in the browser, `Engine` is what the wall is actually doing.
They should agree.

Styling is Mantine with a theme and no per-component overrides — `src/theme.ts`
holds every colour decision, including the palette the canvas and SVG layers
paint with, and `src/styles.css` is two rules long.

## Where each control acts

The editor's controls land in three different places, and which one matters:

| Control | Stage | Why there |
|---|---|---|
| EQ | `dsp/post.rs`, after AGC, before the range map | see below |
| Decay | release ballistics in `dsp/post.rs` | it *is* the release time |
| Frame hop | rebuilds the analyser | changes how often transforms run |
| Threshold, clamp, curve | `color/intensity.rs` | output shaping, not analysis |
| LED sectors, reverse, mirror | `color/strip.rs` | spatial, applied last |

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
cargo test --manifest-path engine/Cargo.toml   # 151
cd ui && npm test && npm run typecheck
```

The ones worth knowing about live in `engine/tests/artifacts.rs`: they assert the
reported bug stays fixed, and one of them independently computes what a naive
analyser would produce, so the suppression tests are not merely asserting that
nothing ever happens.

`engine/tests/pipeline.rs` covers the other failure mode: a control that is
computed correctly and then dropped on the way to the wire. Every editor control
has an end-to-end test that its effect reaches the LED bytes — mirroring lights
both ends, a sector confines a tone to its own LEDs, a 24 dB cut halves the
output — because the unit tests for each stage all pass whether or not anything
is plugged into them.

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
