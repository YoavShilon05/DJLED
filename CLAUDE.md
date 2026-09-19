# CLAUDE.md

Working notes for this repo. `README.md` explains *why* the system is shaped the
way it is and is worth reading before changing any of the DSP, colour or
protocol decisions — it is the design rationale, not a getting-started guide.
This file is the operational half: what to run, where things live, and the
invariants that fail silently if broken.

## What this is

An audio-reactive LED wall. The PC captures audio (loopback or an input) or MIDI
notes, turns it into *levels over a log-frequency axis*, composites a stack of
layers into colour, and streams control points over serial to an Arduino that
expands them across a WS2812B strip.

```
capture.rs / midi/  →  dsp/  →  color/  →  link/  →  Arduino
     (sources)        (levels)  (bytes)   (serial)
                          └──────────────→ ui.rs (WebSocket 9001) → ui/ (React)

color/timeline.rs: the colour field over time. Per layer, looping, clocked
off the wall clock, so the editor previews the same instant of it without
ever being told which one.

presets.rs (12 shows on disk) ←→ hotkeys.rs (ctrl+alt+F1..F12, system-wide)
```

The single most load-bearing idea: **everything downstream of `source.rs` reads
`(levels, centers)` and never asks whether audio or MIDI produced them.**

## Commands

Verified working on this machine (rustc/cargo 1.96.0, node 22.21.0, npm 10.9.4).
Run from the repo root.

```bash
# Engine, no hardware — mock link renders the strip in the terminal
cargo run --manifest-path engine/Cargo.toml --release

# Engine, real strip
cargo run --manifest-path engine/Cargo.toml --release -- --port COM3

# Editor UI (separate terminal); vite serves on 5173, talks to ws://127.0.0.1:9001
cd ui && npm install && npm run dev

# Tests
cargo test --manifest-path engine/Cargo.toml        # 319 pass (257 lib + 62 integration)
cd ui && npm test                                   # 120 pass across 10 files
cd ui && npm run typecheck                          # tsc --noEmit, clean

# End-to-end over the real socket, against a running engine
cd ui && npm run smoke              # the layer stack
cd ui && npm run preset-smoke       # the twelve preset slots
cd ui && npm run timeline-smoke     # a loop, against the clock both sides read

# Regenerate the Rust→TS colour parity fixture (see Invariants)
cargo run --manifest-path engine/Cargo.toml --example color_reference \
  > ui/src/color/reference.json
```

Diagnostics that answer "is it the wiring or the software":

```
--list-ports            find the Arduino
--list-devices          audio endpoints and MIDI ports in one list
--probe 3               capture for 3s and report what actually arrived
--test rgb|chase|white  drive the strip with no audio at all — see docs/wiring.md
--presets PATH          where the twelve presets live
--no-hotkeys            give ctrl+alt+F1..F12 back to whatever else wants them
```

There is no CI, no linter config, and no `rustfmt.toml`.

## Repo map

| Path | Contents |
|---|---|
| `engine/src/` | Rust. See `engine/CLAUDE.md`. |
| `ui/src/` | React + TS + Mantine editor. See `ui/CLAUDE.md`. |
| `firmware/djled/djled.ino` | Arduino sketch. See `firmware/CLAUDE.md`. |
| `docs/wiring.md` | Electrical. Read before anyone powers a strip. |
| `README.md` | Design rationale for every non-obvious decision. |
| `tmp_albumart.cs`, `tmp_colors.cs` (+ `.exe`) | **Orphans.** See Housekeeping. |

## Invariants

These are the things that break without failing loudly. Each has tests; know
which ones to run.

**1. The Rust and TypeScript colour paths must stay numerically identical.**
`ui/src/color/oklab.ts` and `ui/src/color/surface.ts` are ports of
`engine/src/color/oklab.rs` and `surface.rs`; `ui/src/spectrum/render.ts` is a
port of the compositing fold in `engine/src/color/render.rs`. If they diverge,
the editor's Preview strip lies about what the wall will do.
`ui/src/color/reference.json` is generated from the Rust and asserted on both
sides (`engine/tests/color_reference.rs`, `ui/src/color/reference.test.ts`). It
carries still palettes **and** whole timelines sampled at several instants, so
a blend that agrees at rest and drifts in motion fails too.
*Touch either side → regenerate the fixture and run both suites.*

**1a. The timeline's phase never crosses the wire, so both sides derive it.**
`SystemTime::now()` in `main.rs::epoch_seconds` and `Date.now()` in
`color/timeline.ts::nowSeconds`, modulo a per-layer length. That is what makes
the Preview strip show the instant the wall is at, and it is only checked
end to end by `ui/scripts/timeline-smoke.mjs`, which reads the same clock from
a third process. A unit test cannot see this one.

**2. `ShowConfig` is one struct in two languages.** `engine/src/show.rs` and the
interfaces at the top of `ui/src/engine.ts` are the same shape, camelCase on the
wire. `ui/src/config/editor.ts` (`toEngineConfig` / `fromEngineConfig`) is the
*only* place the editor knows that shape — a protocol change lands there, not
scattered through components. Every Rust field carries `#[serde(default)]` so an
older or newer peer degrades instead of dropping the whole edit.

**2a. A layer's stored `surface` is the *first key* of its timeline, derived
rather than duplicated.** `toEngineConfig` writes it from the same field it
writes key 0 from, so the two cannot disagree; the engine reads the keys
wherever there are any and the surface only when there are none. That is the
whole of the compatibility story — a peer that knows nothing about timelines
sees the start of the loop instead of an empty layer.

**3. A one-layer stack must stay byte-identical to the pre-layers renderer.**
Pinned in `engine/src/color/render.rs` and `engine/tests/pipeline.rs`. Old
presets with flat fields and no `layers` key parse as a one-layer show via the
custom `Deserialize` in `show.rs`.

**3a. A layer with no timeline keys is deaf to the clock.** Pinned by
`a_layer_without_a_timeline_ignores_the_clock` in `engine/tests/pipeline.rs` and
its twin in `ui/src/spectrum/stack.test.ts`. Together with (3) this is what
keeps every preset written before timelines existed rendering exactly as it did.

**4. Every editor control needs an end-to-end test that it reaches the LED
bytes.** `engine/tests/pipeline.rs` exists because unit tests for each stage
pass whether or not the stages are wired to each other. A new control without a
row there can be computed perfectly and dropped on the way to the wire.

**5. Colour is stored un-premultiplied**, and blending weights colour by `w·α`
while weighting opacity by `w` alone. Without that asymmetry an invisible
keyframe tints its neighbours. See `README.md` § Colour.

**5a. Where no keyframe reaches, the surface is *transparent* black.** That is
the baseline an empty field and the dead space past every area of effect both
fall to, and it is what lets either compose with a stack instead of blanking it.
Opaque black would be the same pixels on one layer and a blackout on six.

**6. No gamma step to the LEDs.** `Oklab → linear RGB → byte` is complete.
Adding one is the classic wrong fix.

**7. While a client is connected, the engine's show is authoritative — not the
editor's.** Presets inverted this. The twelve shows live in `engine/src/presets.rs`
and a global hotkey can replace the live one with the browser closed, so
`App.tsx` adopts the engine's config whenever `state.activePreset` changes and
otherwise leaves what is on screen alone. That single field is the whole rule;
`engine/tests/presets.rs` and `ui/scripts/preset-smoke.mjs` defend the round
trip.

## Gotchas

- **Windows-only in practice.** WASAPI loopback (`capture.rs`) and WinMM MIDI
  (`midi/mod.rs`) assume Windows. Nothing gates them behind `cfg`.
- **`cargo fmt` will rewrite 30 files.** The repo is not rustfmt-clean (319
  diffs) and there is no `rustfmt.toml` pinning the style it was written in.
  Don't run it — a formatting sweep would bury any real diff. Match the
  surrounding style by hand.
- **85 colours is the protocol ceiling, 64 is the stock firmware's.**
  `protocol::max_bands()` is `255 / 3 = 85`; the sketch's `MAX_BANDS` is 64 and
  the handshake reports it. An over-long frame is dropped by the board *in
  silence* while every PC-side display keeps working — which looks exactly like
  broken hardware. A full 88-key MIDI range is resampled on the way out, and the
  engine says so at startup.
- **READY is one invitation, not a credit.** The board announces it can receive,
  listens for 25 ms, then writes the strip with the UART deaf. So there is never
  more than one outstanding and an old one is worthless: `link/serial.rs` keeps
  a single timestamped token, and a stale input queue is binned rather than
  decoded. Banking them instead let a silent stretch — nothing to send, so
  nothing reading the port either — accumulate hundreds, which were then spent
  in a burst that desynced the sketch's reader with no gap to recover in. The
  wall fell to about a frame a second, seconds behind the music, while the
  terminal preview and the editor stayed perfect, because neither crosses the
  wire.
- **A MIDI input opens once, per process.** Windows hands a port to one
  application at a time. Two layers on two channels of one keyboard share the
  handle and filter per layer (`Layer::feed_key` drops the channel for MIDI,
  keeps it for audio). Never open a MIDI port per layer.
- **Commands are drained before audio is read**, on every pass, not only on
  passes that complete a frame. A dead loopback delivers no samples, and the
  command that most needs through is the one moving off it.
- **A layer can listen to *nothing*, and that is a source kind rather than a
  mode.** `SourceKind::None` opens no device, builds no analyser, and sits at a
  flat full level — a still colour along the strip. Three things follow and each
  is load-bearing. Its colour field is one dimensional, because a flat level
  reads exactly one row of the surface: the field is *projected* onto that row
  (`SurfaceConfig::flattened`, `toRenderSurface`) rather than sampled along it,
  or the editor would draw handles whose colour never reaches the wall. The
  intensity stage is replaced by `pass_through`, explicitly, because full level
  *is* the top of the axis and a threshold left at 0 dB would otherwise black
  out the one layer that is supposed to be unconditional. And it reports frames
  on its own clock (`Analysis::tick`), because the run loop renders only when a
  layer says it has something new — without it a show of still layers alone
  leaves the strip dark while every preview in the editor looks right.
- **A still layer's authored dB crosses the wire untouched.** The flattening is
  a property of how the field is read, not how it is stored. The engine keeps
  every config it is sent, so flattening on the way out would make a layer
  switched to no source and back permanently lose its 2D field one preset switch
  later. `dbOfY` on a flat plot is a constant for the same reason, and the drag
  handler discards the vertical rather than writing that constant back.
- **`feed: None` in the stack means two different things.** A device that would
  not open, and a layer that has none by design. The `error` beside it is what
  tells them apart — one is a fault with a red badge, the other is a choice.
  `retry_failed` tests for the error, not for the missing feed.
- **MIDI mode's x axis is positions, not pitches.** The note range is stretched
  across 20 Hz–20 kHz. A keyframe "at 250 Hz" means a fifth of the way along.
- **A frame narrower than the level grid aggregates; one at least as wide
  interpolates.** `StripMap` picks between `level_at` and `peak_level_between`
  on that alone. Audio is always the second case — a band plan is coarser than
  the frame — so this is MIDI's code path in practice: 88 semitones into 64
  points, where sampling *between* grid points slides off a note's peak and
  reports a velocity nobody played. Level is the colour surface's y, so that
  arrives as the wrong colour rather than a dimmer one. Pinned by
  `a_grid_finer_than_the_frame_keeps_every_peak` and
  `full_velocity_reaches_the_top_of_the_colour_surface_at_every_note`.
- **"Frame hop" is not an FFT window length.** Bands each draw from one of five
  tiers (8192→128); the hop is how often those transforms run.
- **A timeline key is the whole graph at a moment, and two keys are blended by
  *position in the list*.** A `Keyframe` carries no id over the wire — it is a
  point and a colour, so a preset stays readable — so index is the only
  correspondence there is. The editor holds up its end by adding and deleting a
  colour on *every* key at once (`mergeKeyEdit`): where a colour sits belongs to
  the key, the *set* of colours belongs to the layer. A hand-edited preset can
  still break that, so a keyframe present at one end of a span and absent at the
  other fades rather than popping — never rejects.
- **The loop runs off the wall clock and there is no transport.** Nothing pauses
  it, nothing scrubs it, and switching preset does not restart it. That is the
  price of both sides agreeing on the instant without exchanging it. The
  editor's playhead is a readout, written onto one element from an animation
  frame — putting it in React state would re-render that panel sixty times a
  second to move a line.
- **An animated show must keep rendering with every source idle.** `main.rs`
  used to sleep whenever `stack.poll()` reported nothing; a timeline is the one
  stage that is a function of the clock rather than of levels, so the idle path
  now renders on the display clock when `renderer.animates()`. Drop that and a
  show of loops freezes the moment the music stops — on the wall only, because
  the editor's preview has its own clock, which is the worst way for it to fail.
- **The first key cannot be deleted or moved.** It *is* `Layer::surface` —
  `config/timeline.ts` keeps only the keys after it — so deleting the last of
  the others leaves a still layer rather than an empty loop. `BASE_KEY` is the
  empty string and `keyOf` falls back to it, which is also what makes a stale
  `activeKeyId` from another layer harmless.
- **An area of effect cannot be halfway to "everywhere".** `radius` is an
  `Option`, so `UNCONFINED` (1.45 — the diagonal of the unit square, and the top
  of the editor's slider) stands in for `None` when interpolating, and a lerp
  that reaches it turns the limit off again. Both sides do this; it is in the
  reference fixture.
- **Colour cycle joins the position axis, and only that axis.** With it on,
  `x` distance is measured the short way round (`short_way_round` /
  `shortWayRound`), so the two ends of the strip are one point: an area of
  effect runs off one and comes back on the other, and a keyframe animated from
  0 to 1 arrives where it started. Level never wraps — a quiet band is not
  adjacent to a loud one. It is a `Layer` flag rather than part of
  `SurfaceConfig`, applied with `ColorSurface::cycling` *after* compiling a
  config or a timeline: it says how the field is read, not what was authored, so
  it survives `seek` and turning it off gives back the exact strip that was
  there before. It is also not mirror or reverse — those decide where a colour
  lands, this decides what the field considers adjacent.
- **Sectors, EQ, thresholds and the curve are not on the timeline.** They build
  a strip map and a filter chain rather than being sampled per frame, so
  animating them means rebuilding an analyser at frame rate. Only the colour
  field moves.
- **The editor plot shows one layer; the strip preview shows the stack.** Judge
  composite results on the preview, never on the graph. Each row of the layer
  list also carries a thumbnail of that layer alone — at full opacity and full
  brightness, so it stays legible; it identifies a layer, it does not measure
  one.
- **The editor is one screen, not a document.** Header, the twelve shows and
  both preview strips are pinned; the layer list, the inspector and the graph
  scroll under them. Anything added to the pinned half costs the graph that
  height on every screen the editor is ever opened on.
- **Anything a frame re-renders re-renders thirty times a second.** The engine
  publishes at a fixed rate with no back pressure, so the editor coalesces
  frames onto an animation frame (`engine.ts`) and memoises every panel that is
  not driven by one. Both halves matter: without the first a tab that falls
  behind banks the backlog until it is killed for running out of memory, and
  without the second it falls behind. A prop built inline, or a callback that
  loses its `useCallback`, puts a whole Mantine panel back on the frame path.
  The layer thumbnails are the exception that proves it: they read a ref on
  their own timer rather than taking a prop, precisely so twelve rows do not
  re-render to repaint twelve canvases.
- **A keyframe's area of effect is not the blend radius, and the two are one
  click apart in the same panel.** Blend radius (the surface's sigma) is one
  number for the layer and decides how two keyframes that *both* reach a point
  share it. The area of effect is per keyframe and decides whether a keyframe
  reaches the point at all. Narrowing sigma to confine one colour sharpens every
  other colour on the layer, which is the mistake this exists to make
  unnecessary.
- **A taper inside a normalised sum cancels itself.** The area of effect is
  applied twice on purpose — once as a weight, and once as `present`, outside the
  normalisation. Drop the second and a lone confined keyframe holds full opacity
  to its edge and then steps to nothing, which is a hard line across the wall.
  Pinned by `a_confined_keyframe_fades_out_rather_than_stopping`.
- **Gizmo checkboxes are visibility, not bypass.** A hidden EQ still shapes the
  signal. They live in the graph's own `Overlays` menu, not in the sidebar.
- **There is no save button, and the preset bar is not a loader.** Selecting a
  slot makes it live *and* makes it the thing being edited; every config the
  editor sends goes straight into it. `Reset` — in the ⋯ menu, beside the
  master brightness — therefore replaces the live preset with the default show;
  it does not put an older one back.
- **An empty preset slot behaves differently by the bar and by hotkey.** Clicking
  the chip opens a blank canvas, the hotkey declines. A mis-hit during a set must
  not blank the wall. Pinned in `presets.rs` and `main.rs::load_preset`.
- **`RegisterHotKey` refusals are normal and must stay visible.** Another
  application holding `ctrl+alt+F4` is not an error and cannot be fixed from
  here; the engine names the ones it lost at startup, because a hotkey that never
  registered is otherwise indistinguishable from one that fired and did
  nothing.

## Conventions

- **Doc comments carry the reasoning.** Every module opens with a `//!` / `/** */`
  block explaining why it exists and what was rejected. This is the house style —
  new modules get one, and a decision that took measurement to reach gets written
  down where it lives rather than in a commit message.
- **Test names are sentences**: `mirror_lights_both_ends_of_the_strip`,
  `opaque_black_blanks_the_strip_and_transparent_black_does_not`.
- **Assert closed-form properties, not captured values.** `eq.test.ts` checks
  that a bell is exactly its gain at centre and a pass filter is −3.01 dB at
  cutoff with `Q = 1/√2`, so a transcription slip fails rather than becoming the
  new expectation.
- British spelling in prose and doc comments (`colour`, `normalised`); American
  in identifiers (`color`, `centers`).
- Config defaults live next to the type (`impl Default`), not in the CLI.

## Housekeeping

`tmp_albumart.cs`, `tmp_colors.cs` and their committed `.exe` files are a spike:
pull the current Spotify track's album art via WinRT `GlobalSystemMediaTransport-
ControlsSession`, then k-means its dominant colours. Nothing in `engine/`, `ui/`
or `firmware/` references them and no doc mentions them. Treat them as dead
unless the user says otherwise — and check before deleting, since they are the
only record of that experiment.
