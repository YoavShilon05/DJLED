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
cargo test --manifest-path engine/Cargo.toml        # 255 pass (210 lib + 45 integration)
cd ui && npm test                                   # 65 pass across 8 files
cd ui && npm run typecheck                          # tsc --noEmit, clean

# End-to-end over the real socket, against a running engine
cd ui && npm run smoke              # the layer stack
cd ui && npm run preset-smoke       # the twelve preset slots

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
sides (`engine/tests/color_reference.rs`, `ui/src/color/reference.test.ts`).
*Touch either side → regenerate the fixture and run both suites.*

**2. `ShowConfig` is one struct in two languages.** `engine/src/show.rs` and the
interfaces at the top of `ui/src/engine.ts` are the same shape, camelCase on the
wire. `ui/src/config/editor.ts` (`toEngineConfig` / `fromEngineConfig`) is the
*only* place the editor knows that shape — a protocol change lands there, not
scattered through components. Every Rust field carries `#[serde(default)]` so an
older or newer peer degrades instead of dropping the whole edit.

**3. A one-layer stack must stay byte-identical to the pre-layers renderer.**
Pinned in `engine/src/color/render.rs` and `engine/tests/pipeline.rs`. Old
presets with flat fields and no `layers` key parse as a one-layer show via the
custom `Deserialize` in `show.rs`.

**4. Every editor control needs an end-to-end test that it reaches the LED
bytes.** `engine/tests/pipeline.rs` exists because unit tests for each stage
pass whether or not the stages are wired to each other. A new control without a
row there can be computed perfectly and dropped on the way to the wire.

**5. Colour is stored un-premultiplied**, and blending weights colour by `w·α`
while weighting opacity by `w` alone. Without that asymmetry an invisible
keyframe tints its neighbours. See `README.md` § Colour.

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
- **A MIDI input opens once, per process.** Windows hands a port to one
  application at a time. Two layers on two channels of one keyboard share the
  handle and filter per layer (`Layer::feed_key` drops the channel for MIDI,
  keeps it for audio). Never open a MIDI port per layer.
- **Commands are drained before audio is read**, on every pass, not only on
  passes that complete a frame. A dead loopback delivers no samples, and the
  command that most needs through is the one moving off it.
- **MIDI mode's x axis is positions, not pitches.** The note range is stretched
  across 20 Hz–20 kHz. A keyframe "at 250 Hz" means a fifth of the way along.
- **"Frame hop" is not an FFT window length.** Bands each draw from one of five
  tiers (8192→128); the hop is how often those transforms run.
- **The editor plot shows one layer; the strip preview shows the stack.** Judge
  composite results on the preview, never on the graph.
- **Gizmo checkboxes are visibility, not bypass.** A hidden EQ still shapes the
  signal.
- **There is no save button, and the preset dropdown is not a loader.** Selecting
  a slot makes it live *and* makes it the thing being edited; every config the
  editor sends goes straight into it. `Reset` therefore replaces the live preset
  with the default show — it does not put an older one back.
- **An empty preset slot behaves differently by dropdown and by hotkey.** The
  dropdown opens a blank canvas, the hotkey declines. A mis-hit during a set must
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
