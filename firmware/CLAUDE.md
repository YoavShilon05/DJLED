# firmware/ — Arduino sketch

One file: `djled/djled.ino`. It does **no signal processing**. The PC sends one
RGB triplet per control point; this spreads them across the strip and drives the
pixels. Every colour decision already happened on the PC.

Depends on **FastLED** only. There is no `arduino-cli` on this machine and no
build config in the repo — flashing is done from the Arduino IDE (or
`arduino-cli compile --fqbn arduino:avr:uno firmware/djled` if you install it).
Nothing in `cargo test` or `npm test` compiles this file, so a change here is
unverified until someone flashes a board.

**Read `docs/wiring.md` before anyone powers a strip.** Most of the ways this
project goes wrong are electrical.

## The two constants the PC side depends on

```c
#define LED_COUNT   150   // reported in the handshake; the engine builds geometry from it
#define MAX_BANDS   64    // colours per frame the board will accept
```

Both cross the wire in the HELLO reply, so the engine adapts rather than
guessing — but changing either means reflashing before the PC sees it.

`MAX_BANDS` is the one that bites. The protocol allows 85 (`MAX_PAYLOAD / 3` in
`engine/src/link/protocol.rs`); the stock sketch takes 64. The receive buffer is
sized from `MAX_BANDS` and the board **stops reading anything longer, with
nothing to reply on** — so an over-long frame is dropped in silence while every
display on the PC keeps working. That failure looks exactly like broken
hardware. A full 88-key MIDI range is resampled down on the way out and the
engine says so at startup. At 150 LEDs there is SRAM for the full 85.

## Other constants

| Name | Value | Note |
|---|---|---|
| `LED_PIN` | 6 | |
| `MAX_MILLIAMPS` | 300 | **Raise to your PSU's real rating.** FastLED scales brightness to stay under it, turning a brownout into a graceful dim |
| `BAUD` | 500000 | 0% error with U2X at 16 MHz, unlike 115200's 2.1% |
| `WATCHDOG_MS` | 2000 | Blanks the strip if the PC stops talking, so a crash doesn't freeze the wall |
| `FRAME_TIMEOUT_MS` | 25 | How long to wait for a frame after announcing READY |
| `PROTOCOL_VERSION` | 1 | Must match `link/protocol.rs` |

## Why the flow control exists

FastLED bit-bangs WS2812B with **interrupts disabled for the entire strip
write** — 30 µs per LED, so 4.5 ms at 150 LEDs and 18 ms at 600. The ATmega328P's
UART FIFO holds two bytes, about 40 µs at 500 kbaud. Streaming into a deaf MCU
loses nearly every frame.

So the board asks: it emits `READY` (`0x7E`) when it can actually receive, the PC
sends exactly one frame, and the board goes deaf to write the strip before asking
again. The PC computes the next frame during that blackout, so the pipeline
overlaps for free. Don't "optimise" this into a free-running stream.

## The SRAM guard

```c
static_assert(LED_COUNT * 3 + MAX_BANDS * 3 < 1500, ...)
```

An ATmega328P has 2048 bytes. Overrunning it doesn't fail loudly — the stack
grows into the heap and the board behaves erratically — so it is caught at
compile time. Practical ceiling ~435 LEDs. The planned 600-LED strip needs a Mega
(drop-in, ~47 fps) or an ESP32 (DMA output, 100 fps+, but needs a 74AHCT125 level
shifter that the 5 V Uno does not).

## Notes

- `FastLED.addLeds<WS2812B, LED_PIN, GRB>` handles the GRB reorder, which is why
  the wire protocol stays in RGB order.
- The sketch mirrors `engine/src/link/protocol.rs` by hand. A framing or CRC
  change has to land in both, and only the Rust side has tests.
- Test patterns (`--test rgb|chase|white`) are implemented here, not on the PC,
  so they work even when the audio path is entirely broken. That is the point:
  each isolates one failure mode. See the bring-up order in `docs/wiring.md`.
