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
| `ui/` | React + TypeScript. The 2D keyframe colour editor. |
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
cargo test --manifest-path engine/Cargo.toml   # 97
cd ui && npm test && npm run typecheck
```

The ones worth knowing about live in `engine/tests/artifacts.rs`: they assert the
reported bug stays fixed, and one of them independently computes what a naive
analyser would produce, so the suppression tests are not merely asserting that
nothing ever happens.

## Scaling to 600 LEDs

The 10 m strip will not run on an Uno — 600 LEDs is 1800 bytes of framebuffer in
2048 bytes of SRAM. The sketch catches this at compile time. Use a Mega (drop-in,
~47 fps) or an ESP32 (DMA output, 100 fps+). The PC side does not change: LED
count is a firmware constant reported in the handshake, and all colour decisions
already happen on the PC.
