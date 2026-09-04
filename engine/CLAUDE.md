# engine/ — Rust

Capture, analysis, colour, compositing, wire protocol and the UI bridge. All
signal processing happens here; the Arduino is a pixel expander.

Edition 2024, no build script, no `rustfmt.toml`. See the root `CLAUDE.md` for
cross-cutting invariants — especially that `cargo fmt` would rewrite 30 files.

## Reading order

`lib.rs` → `main.rs` (the loop) → `stack.rs` (what owns what) → the stage you
care about. Every module opens with a `//!` block giving the reasoning; those are
the primary documentation and are usually more current than any summary here.

## Where each stage lives

| File | Responsibility |
|---|---|
| `main.rs` | CLI, the 60 fps loop, terminal preview, device listing, probe and test patterns |
| `stack.rs` | The running stack: one analyser per layer, one open device per *selection* |
| `show.rs` | `ShowConfig` / `Layer` — everything the editor can change, in one struct |
| `source.rs` | `Source` (what to listen to) and `LiveSource` (the open thing + its analyser) |
| `capture.rs` | WASAPI via cpal: loopback of a render endpoint, or a capture endpoint |
| `midi/mod.rs` | WinMM port open, callback → SPSC ring, message parsing |
| `midi/notes.rs` | Notes → levels on the frequency axis. Envelope, sustain, note range |
| `engine.rs` | `Engine` — the analysis chain assembled: DC block → STFT tiers → fast path → post |
| `dsp/bands.rs` | Bark-like band layout, and the FFT size assigned to each band |
| `dsp/mrstft.rs` | Runs the tier ladder, merges to per-band magnitudes |
| `dsp/fastpath.rs` | Gated parallel bandpass so bass attacks aren't stuck behind a long window |
| `dsp/post.rs` | Floor tracking → dB → tilt → AGC → range map → ballistics → smoothing |
| `dsp/eq.rs` | RBJ cookbook biquads evaluated as `\|H(e^jw)\|`, summed in dB |
| `color/surface.rs` | The 2D keyframe field over (strip position × intensity) |
| `color/oklab.rs` | Oklab ↔ linear RGB, and what opacity means when compositing |
| `color/intensity.rs` | Level → brightness: threshold, clamp, curve |
| `color/strip.rs` | Where each frequency lands: LED sectors, reverse, mirror |
| `color/render.rs` | Samples every layer, folds the stack, quantises to bytes |
| `link/protocol.rs` | Framing, CRC8, `Kind`, handshake, incremental decoder |
| `link/serial.rs` | Speaks it to a real board |
| `link/mock.rs` | Renders frames in the terminal — full pipeline with no hardware |
| `ui.rs` | WebSocket server: `Snapshot` out per frame, `Command` in |

## Signal flow, in order

1. `stack.rs` polls each `Feed` once per pass; every analyser bound to that feed
   sees the **same** block. (Two layers racing one ring buffer would each get
   half the audio.)
2. `Engine::push` → DC block → fast path sees *every* sample → STFT tiers at hop
   boundaries → fast path clamps into the STFT's own decaying peak-hold → post.
3. `post.rs` order is load-bearing: floor subtraction in the linear domain
   (it is a subtraction of energy), everything after it in dB.
4. **The EQ sits after AGC and before the range map.** Before the AGC an
   authored boost looks like drift and gets unwound; after the range map it
   would lift silence and make an idle strip glow. Both pinned by tests in
   `post.rs`.
5. `color/render.rs` samples each layer at its own control points and folds
   bottom → top with `source-over` in linear light. Master brightness is applied
   *after* the fold — it is the power budget, not a layer property.

## Key constants

| Constant | Value | Where |
|---|---|---|
| `DEFAULT_HOP` | 256 | `dsp/mrstft.rs` |
| `MIN_HOP` / `MAX_HOP` | 64 / 8192 | `show.rs` |
| `DEFAULT_FFT_SIZES` | 128 … 32768 (only assigned tiers are built) | `dsp/bands.rs` |
| `REFERENCE_DECAY` | 0.82 | `dsp/post.rs` |
| `DEFAULT_BAUD` | 500 000 | `link/serial.rs` |
| `READY` / `MAGIC` | `0x7E` / `A5 5A` | `link/protocol.rs` |
| `MAX_PAYLOAD` / `OVERHEAD` | 255 / 5 → `max_bands()` = 85 | `link/protocol.rs` |
| `DEFAULT_PORT` (UI) | 9001 | `ui.rs` |

## Rules that are easy to break

- **`Layer::feed_key` drops the channel for MIDI and keeps it for audio.** That
  asymmetry is the whole pooling scheme: a WASAPI endpoint's channel is chosen
  when the stream is built (two channels really are two streams), a MIDI channel
  is a per-layer filter (Windows won't open the port twice).
- **A device that won't open is not fatal.** The layer keeps its place, sits at
  silence, and carries the reason in `LayerStatus`. `ListSources` retries them,
  because a rescan is exactly what someone does after plugging the interface
  back in.
- **Opening a source is the only operation that makes the caller rebuild its
  renderer** — it can change the *shape* of `(levels, centers)` (48 bands become
  88 semitones). A config edit never can.
- **A hidden layer is skipped outright**, not composited at zero. Muting is a
  real saving; that is the point.
- **Applying a `ShowConfig` must stay cheap.** The editor sends the whole config
  on every pointer move. Each stage compares against what it already holds and
  rebuilds only what changed; don't add unconditional reallocation to an apply
  path.
- **A position no sector reaches contributes nothing**, which is not the same as
  contributing black. Invisible until there is a layer underneath.
- `#[serde(default)]` on every `Layer` field, and `ShowConfig::normalise` never
  leaves the stack empty — `base()` calls `.expect()` on that guarantee.

## Tests

240 total: 199 unit (in-module `#[cfg(test)]`) + 41 integration.

| File | What it defends |
|---|---|
| `tests/artifacts.rs` (8) | The reported bugs stay fixed. One independently computes what a naive analyser would produce, so the suppression tests aren't just asserting nothing happens |
| `tests/pipeline.rs` (19) | Every editor control's effect reaches the LED bytes. **Add a row here for any new control** |
| `tests/midi_pipeline.rs` (11) | The note path end to end, delivered as bytes — the only MIDI coverage that runs without a port |
| `tests/color_reference.rs` (3) | The Rust side of the TS parity fixture |

Layer behaviour is pinned in three places on purpose: `color/render.rs` (the
fold, including one-layer byte-identity), `stack.rs` (pooling against the
machine's *real* devices — a mock can't show that a second layer shares a
handle), and `ui/src/spectrum/stack.test.ts` (the browser's copy of the fold).

Run: `cargo test --manifest-path engine/Cargo.toml`
