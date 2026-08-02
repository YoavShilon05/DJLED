# Wiring

For an Arduino Uno/Nano driving a WS2812B strip. Read this before applying power
— most of the ways this project can go wrong are electrical, not software.

## The three rules that matter most

1. **Never power the strip from the Arduino's 5V pin.** That pin is fed through
   the on-board regulator and USB, good for a few hundred milliamps. The strip
   wants amps. Doing this browns out the board mid-frame and can damage the
   regulator or the USB port.
2. **Connect PSU ground to Arduino ground.** WS2812B reads its data line
   relative to its own ground. Without a shared reference the data is
   meaningless and you get random flicker — the single most common "my strip is
   possessed" cause.
3. **Set `MAX_MILLIAMPS` in the sketch to your PSU's real rating.** FastLED then
   scales brightness to stay inside it, turning a brownout into a graceful dim.

## Diagram

```
   5V PSU  ─┬────────────────────── strip +5V  ──── (inject again at far end)
            │
            ├── 1000 µF ──┐
            │   (at strip)│
   PSU GND ─┴─────────────┴──────── strip GND
            │
            └──────────────┐
                           │  common ground — required
   Arduino GND ────────────┘

   Arduino D6 ──── 330-470 Ω ─────── strip DIN
                   (at the Arduino end, keep this run under ~30 cm)
```

## Parts

| Part | Value | Why |
|---|---|---|
| Series resistor | 330–470 Ω | Protects the first pixel's input from ringing on the data edge |
| Bulk capacitor | 1000 µF, ≥6.3 V | Absorbs inrush when the strip jumps from dark to bright |
| Power injection wire | 18 AWG | Carries current to the far end without dropping volts |
| Fuse | at PSU output | A shorted strip otherwise draws until something melts |

No level shifter is needed. The Uno's logic is 5 V, which drives WS2812B
directly — this is the one genuine advantage the AVR has over an ESP32 here,
which needs a 74AHCT125.

## Power budget

At full white a WS2812B draws about 60 mA. Music-reactive content averages far
below that — typically 20–30% — but the supply must survive the peaks.

| Strip | Full white | Realistic average | Suggested PSU |
|---|---|---|---|
| **150 LEDs** (5 m @ 30/m) | 9.0 A | ~2–3 A | 5 V 5 A, or 5 V 10 A for headroom |
| **600 LEDs** (10 m @ 60/m) | 36 A | ~8–11 A | 5 V 20 A, injected in several places |

The sketch ships with `MAX_MILLIAMPS 2000`, deliberately conservative. Raise it
to match your supply, leaving perhaps 20% margin.

## Power injection

Strip copper is thin, and voltage drops along its length. At 30 LEDs/m a full
metre pulls ~1.8 A at white, and by the far end of a 5 m run the voltage has
sagged enough that white turns orange — the blue and green channels dim before
red does, because the red LED has the lowest forward voltage.

- **150 LEDs / 5 m** — feed +5 V and GND at *both* ends. That is usually enough.
- **600 LEDs / 10 m** — feed every 1.5–2 m, using 18 AWG run alongside the strip
  rather than through it.

Run `--test white` to see this directly. If the far end is visibly warmer than
the near end, you need another injection point.

## Bring-up order

Do these in order, before trusting anything about the audio path. Each isolates
one failure mode.

```bash
cargo run --release -- --list-ports              # find the board
cargo run --release -- --port COM3 --test rgb    # channel order
cargo run --release -- --port COM3 --test chase  # count and direction
cargo run --release -- --port COM3 --test white --brightness 0.3
```

| Test | Shows | If it looks wrong |
|---|---|---|
| `rgb` | Solid red, then green, then blue | Red showing as green means the colour order is wrong — change `GRB` in `FastLED.addLeds<...>` in the sketch |
| `chase` | One dot walking end to end | Dot vanishing early means `LED_COUNT` is higher than the real strip; dot starting at the wrong end means the strip is mounted backwards |
| `white` | Even white along the strip | Far end orange or dim means power injection is needed |

Then run without `--test` and play something.

## Known gotchas

**WS2812B is GRB, not RGB.** The firmware handles this in the `addLeds`
template parameter, which is why the wire protocol stays in plain RGB order.
Some strips sold as WS2812B are actually RGB-ordered clones; `--test rgb` tells
you in one second.

**Opening the serial port resets the board.** Asserting DTR reboots the Uno into
its bootloader for about two seconds. The engine waits this out during the
handshake, so the first connection after plugging in is simply slow, not broken.
If you want to suppress it permanently, a 10 µF capacitor between RESET and GND
blocks the auto-reset — but you must remove it to upload new sketches.

**500000 baud, not 115200.** At 16 MHz the ATmega's U2X divisor is exact at
500 kbaud and off by 2.1% at 115200. Some Uno R3 and FT232 boards also manage
1 Mbaud, which halves transfer time; CH340 clones often do not.

**Data line length.** Keep the Arduino-to-first-pixel run under about 30 cm. If
the strip must sit further away, move the Arduino rather than lengthening the
data wire.

## Scaling to 600 LEDs

The 10 m / 60-per-metre strip will not run on an Uno. A framebuffer is 3 bytes
per LED, so 600 LEDs is 1800 bytes of the ATmega328P's 2048 — leaving nothing
for the stack and serial buffers. The sketch catches this at compile time rather
than letting it fail mysteriously at runtime.

- **Arduino Mega** — 8 KB SRAM, drop-in, no code changes. The strip write still
  costs 18 ms, so expect ~47 fps.
- **ESP32** — drives WS2812B over DMA, so the write costs no CPU and does not
  block serial at all. Splitting into 2×300 gives 9 ms and 100 fps+. Needs a
  74AHCT125 level shifter for its 3.3 V logic.

Either way the PC side is unchanged: LED count is a compile-time constant in the
firmware and a field in the handshake, and all colour decisions already happen
on the PC.
