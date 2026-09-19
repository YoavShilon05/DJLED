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
| `source.rs` | `Source` (what to listen to, including nothing) and `LiveSource` (the open thing + its analyser) |
| `capture.rs` | WASAPI via cpal: loopback of a render endpoint, or a capture endpoint |
| `midi/mod.rs` | WinMM port open, callback → SPSC ring, message parsing |
| `midi/notes.rs` | Notes → levels on the frequency axis. Envelope, sustain, note range |
| `engine.rs` | `Engine` — the analysis chain assembled: DC block → STFT tiers → fast path → post |
| `dsp/bands.rs` | Bark-like band layout, and the FFT size assigned to each band |
| `dsp/mrstft.rs` | Runs the tier ladder, merges to per-band magnitudes |
| `dsp/fastpath.rs` | Gated parallel bandpass so bass attacks aren't stuck behind a long window |
| `dsp/post.rs` | Floor tracking → dB → tilt → AGC → range map → ballistics → smoothing |
| `dsp/eq.rs` | RBJ cookbook biquads evaluated as `\|H(e^jw)\|`, summed in dB |
| `color/surface.rs` | The 2D keyframe field over (strip position × intensity), still and animated |
| `color/timeline.rs` | That field over time: a per-layer looping stack of them |
| `color/oklab.rs` | Oklab ↔ linear RGB, and what opacity means when compositing |
| `color/intensity.rs` | Level → brightness: threshold, clamp, curve |
| `color/strip.rs` | Where each frequency lands: LED sectors, reverse, mirror |
| `color/render.rs` | Samples every layer, folds the stack, quantises to bytes |
| `link/protocol.rs` | Framing, CRC8, `Kind`, handshake, incremental decoder |
| `link/serial.rs` | Speaks it to a real board |
| `link/mock.rs` | Renders frames in the terminal — full pipeline with no hardware |
| `ui.rs` | WebSocket server: `Snapshot` out per frame, `Command` in |
| `presets.rs` | Twelve shows on disk, one live. Coalesced writes, atomic rename |
| `hotkeys.rs` | `RegisterHotKey` for ctrl+alt+F1..F12 on a message-pump thread |

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
6. Before that fold, `Renderer::seek` moves every animated layer's field to
   where its own loop has reached. It takes seconds since the epoch, not an
   uptime — the editor reads the same clock, so both sides land on the same
   instant with nothing exchanged. Seeking after rendering would show every
   frame one behind.

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
| `SLOTS` | 12 — fixed by the keyboard, not chosen | `presets.rs` |
| `UNCONFINED` | 1.45 — what `radius: None` stands in as when interpolated | `color/surface.rs` |
| `MIN_LENGTH` / `MAX_LENGTH` | 0.05 s / 600 s | `color/timeline.rs` |
| `WRITE_DELAY` | 750 ms between preset writes | `presets.rs` |

## Rules that are easy to break

- **`Layer::feed_key` drops the channel for MIDI and keeps it for audio.** That
  asymmetry is the whole pooling scheme: a WASAPI endpoint's channel is chosen
  when the stream is built (two channels really are two streams), a MIDI channel
  is not a stream at all.
- **A MIDI channel travels beside its level, never separately.** `NoteEngine`
  fills `channels()` from the same note that won each grid point, `StripMap::
  map_with` carries that index through the aggregation (`peak_between`) or the
  nearest grid point where the level was interpolated, and
  `Renderer::render_stack_with` looks the colour up from it. Deriving the
  channel from frequency afterwards would look right and be wrong at every
  boundary. `NO_CHANNEL` means audio *and* silence, which is why
  `ControlPoint::default` is hand-written — a derived zero is channel 1.
- **A channel colour's opacity is a mix weight, not coverage.** `render::tint`
  interpolates in Oklab and keeps the *field's* alpha. A palette therefore never
  changes what the layers below contribute, which is what lets a MIDI layer be
  coloured by channel and still compose.
- **Never have bytes in flight while the board is deaf.** One READY buys exactly
  one frame, and `Readiness` in `link/serial.rs` expires it after the sketch's
  own 25 ms listening window. That single-token discipline is what keeps the
  write path from queueing: the moment the PC can get ahead, the strip runs
  seconds late and the sketch resynchronises on garbage.
- **A layer with no source is not a layer that failed.** `SourceKind::None`
  binds no feed, so `feed: None` means either that or a device that would not
  open; `error` is the only thing distinguishing them, and `retry_failed` tests
  for *that* rather than for a missing feed — otherwise every rescan rebuilds
  the whole stack and restarts every capture. `LiveStack::visuals` gives such a
  layer a flattened surface and `IntensityConfig::pass_through`, and
  `LiveStack::poll` advances it with `Analysis::tick` instead of a feed.
- **A device that won't open is not fatal.** The layer keeps its place, sits at
  silence, and carries the reason in `LayerStatus`. `ListSources` retries them,
  because a rescan is exactly what someone does after plugging the interface
  back in.
- **Opening a source is the only operation that makes the caller rebuild its
  renderer** — it can change the *shape* of `(levels, centers)` (48 bands become
  88 semitones). A config edit never can.
- **A hidden layer is skipped outright**, not composited at zero. Muting is a
  real saving; that is the point.
- **A timeline's keys win over `Layer::surface` wherever there are any.**
  `ColorSurface::for_layer` decides this once; nothing else should ask. The
  stored surface is the *degraded* view of the first key — what an editor that
  predates timelines, or somebody reading the preset file, is shown — and
  preferring it would leave two peers that both understand loops arguing about
  which field is live.
- **`ColorSurface::seek` must not allocate.** It runs once per layer per frame
  and refills a buffer it already owns; `seeking_reuses_its_buffer` is there to
  fail if that changes. Interpolation happens in *authored* units — sigma, not
  falloff; radius, not `1/r` — because lerping the reciprocals makes an area of
  effect open fast and close slowly for no reason anybody asked for.
- **`main.rs` cannot sleep through an idle source while anything animates.** A
  timeline is the one stage that is a function of the clock rather than of
  levels, so the idle branch now renders on the display clock when
  `renderer.animates()`. Without it a show of loops freezes the moment the music
  stops, on the wall only — the editor's preview has its own clock and would
  keep running, which is the worst way for this to fail.
- **Applying a `ShowConfig` must stay cheap.** The editor sends the whole config
  on every pointer move. Each stage compares against what it already holds and
  rebuilds only what changed; don't add unconditional reallocation to an apply
  path.
- **A position no sector reaches contributes nothing**, which is not the same as
  contributing black. Invisible until there is a layer underneath.
- `#[serde(default)]` on every `Layer` field, and `ShowConfig::normalise` never
  leaves the stack empty — `base()` calls `.expect()` on that guarantee.
- **Every `Command::Config` is stored into the live preset slot.** There is no
  save message and no save button; that is what makes a hotkey switch safe,
  since nothing is ever in flight to lose. Writes are coalesced by
  `Presets::tick`, called once per pass of the loop.
- **`Presets::select` flushes before it switches**, so the slot being left
  reaches disk before the slot being entered replaces it in memory. Delayed
  writes are the reason that has to be explicit.
- **A preset switch is `announce_state`, a config edit is `update_state`.** The
  editor cannot predict a switch — a hotkey or a second tab can cause one — and
  `active_preset` changing is the only signal it has that the show was replaced
  by something other than its own hands.
- **The hotkey drain sits outside the `if let Some(server)` block.** The keys are
  the half of this that works with no editor attached, which is the entire reason
  they are registered with the OS rather than handled in the browser.
- **Failing to persist is never fatal.** A corrupt or unwritable presets file
  costs the presets and not the strip; the reason goes to the status line.

## Tests

330 total: 266 unit (in-module `#[cfg(test)]`) + 64 integration.

| File | What it defends |
|---|---|
| `tests/artifacts.rs` (8) | The reported bugs stay fixed. One independently computes what a naive analyser would produce, so the suppression tests aren't just asserting nothing happens |
| `tests/pipeline.rs` (30) | Every editor control's effect reaches the LED bytes, including a show of still layers driven through `LiveStack` with no device in the chain. **Add a row here for any new control** |
| `tests/midi_pipeline.rs` (14) | The note path end to end, delivered as bytes — the only MIDI coverage that runs without a port. Includes the channel palette reaching the LEDs |
| `tests/color_reference.rs` (6) | The Rust side of the TS parity fixture, stills and loops |
| `tests/presets.rs` (4) | A preset stored, reloaded from disk, and rendered — the bytes have to match the show it was saved from |

Layer behaviour is pinned in three places on purpose: `color/render.rs` (the
fold, including one-layer byte-identity), `stack.rs` (pooling against the
machine's *real* devices — a mock can't show that a second layer shares a
handle), and `ui/src/spectrum/stack.test.ts` (the browser's copy of the fold).

Run: `cargo test --manifest-path engine/Cargo.toml`
