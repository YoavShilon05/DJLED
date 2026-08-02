// DJLED firmware — WS2812B strip driver for an audio-reactive wall EQ.
//
// This board does no signal processing. The PC captures audio, runs the
// spectrum analysis, samples the colour surface and sends one RGB triplet per
// frequency band; all this sketch does is spread those colours across the strip
// and drive the pixels.
//
// That split is deliberate. Sending colour per *band* rather than per LED keeps
// a frame at 149 bytes no matter how long the strip is — 3 ms at 500 kbaud —
// where per-LED frames would cost 1800 bytes and cap a 600-LED strip at 27 fps.
//
// ---------------------------------------------------------------------------
// WIRING — read docs/wiring.md before powering anything. In brief:
//   * Do NOT power the strip from the Arduino's 5V pin.
//   * PSU ground and Arduino ground MUST be connected.
//   * 330-470R in series on the data line, 1000uF across 5V/GND at the strip.
//   * Set MAX_MILLIAMPS below to your PSU's real rating.
// ---------------------------------------------------------------------------

#include <FastLED.h>

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

        #define LED_PIN     6
#define LED_COUNT   150      // 5 m at 30 LEDs/m
#define MAX_BANDS   64

// Power ceiling. FastLED scales global brightness to stay under this, which
// turns a brownout (flickering, random colours, or a board reset mid-frame)
// into a graceful dimming instead. 150 WS2812B at full white would draw 9 A;
// this default assumes a modest 5 V 2 A supply, so RAISE IT to match yours.
#define MAX_MILLIAMPS 300

// 0% baud error with U2X at 16 MHz, unlike 115200's 2.1%.
#define BAUD        500000

// Blank the strip if the PC stops talking, so a crash or unplug does not leave
// the wall frozen on the last frame.
#define WATCHDOG_MS 2000

// How long to wait for a frame after announcing readiness. Short enough that
// the loop stays responsive when idle.
#define FRAME_TIMEOUT_MS 25

#define PROTOCOL_VERSION 1
#define FIRMWARE_VERSION 1

// ---------------------------------------------------------------------------
// SRAM guard
// ---------------------------------------------------------------------------
//
// An ATmega328P has 2048 bytes total. The framebuffer alone is 3 bytes per LED,
// and Arduino's serial buffers plus the stack need roughly 450 more. Exceeding
// that does not fail loudly at runtime — the stack quietly grows into the heap
// and the board behaves erratically — so it is caught here instead.
//
// Practical ceiling is around 435 LEDs. For the planned 10 m / 600 LED strip
// you need an Arduino Mega (8 KB) or, better, an ESP32: it drives WS2812B over
// DMA so the strip write costs no CPU and does not block serial at all.
#if defined(__AVR_ATmega328P__) || defined(__AVR_ATmega168__)
static_assert(LED_COUNT * 3 + MAX_BANDS * 3 < 1500,
              "Too many LEDs for this board's SRAM. Use an Arduino Mega or an ESP32.");
#endif

// ---------------------------------------------------------------------------
// Protocol — mirrors engine/src/link/protocol.rs
// ---------------------------------------------------------------------------

static const uint8_t READY  = 0x7E;
static const uint8_t MAGIC0 = 0xA5;
static const uint8_t MAGIC1 = 0x5A;

enum Kind : uint8_t {
  KIND_HELLO    = 0x00,
  KIND_BAND_RGB = 0x01,
  KIND_TEST     = 0x02,
  KIND_BLACKOUT = 0x03,
};

enum TestPattern : uint8_t {
  TEST_RGB   = 0x00,
  TEST_CHASE = 0x01,
  TEST_WHITE = 0x02,
  TEST_NONE  = 0xFF,
};

// CRC-8, polynomial 0x07, init 0. Bitwise rather than table-driven: 256 bytes
// of flash is worth more here than the ~20 us this costs over a 149-byte frame.
//
// Must stay byte-for-byte identical to crc8() in engine/src/link/protocol.rs.
// Known answers, checked by a test on that side: "" -> 0x00,
// "123456789" -> 0xF4, {0xFF} -> 0xF3.
static inline uint8_t crc8_update(uint8_t crc, uint8_t b) {
  crc ^= b;
  for (uint8_t i = 0; i < 8; i++) {
    crc = (crc & 0x80) ? (uint8_t)((crc << 1) ^ 0x07) : (uint8_t)(crc << 1);
  }
  return crc;
}

static uint8_t crc8(const uint8_t* data, uint8_t len) {
  uint8_t crc = 0;
  while (len--) crc = crc8_update(crc, *data++);
  return crc;
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

static CRGB leds[LED_COUNT];
static uint8_t payload[MAX_BANDS * 3];
static uint8_t payloadLen = 0;
static uint8_t frameKind = 0;

static unsigned long lastFrameMs = 0;
static uint8_t testMode = TEST_NONE;
static unsigned long testStartMs = 0;

// ---------------------------------------------------------------------------
// Serial helpers
// ---------------------------------------------------------------------------

static int readByteUntil(unsigned long deadline) {
  while ((long)(millis() - deadline) < 0) {
    if (Serial.available()) return Serial.read();
  }
  return -1;
}

// Read one framed message into `payload`. Returns true on a complete, intact
// frame. Resynchronises by scanning for the magic pair, so a partial or
// corrupted frame costs one timeout rather than desyncing the link forever.
static bool readFrame() {
  unsigned long deadline = millis() + FRAME_TIMEOUT_MS;

  uint8_t prev = 0;
  for (;;) {
    int b = readByteUntil(deadline);
    if (b < 0) return false;
    // Comparing against the previous byte handles a run like A5 A5 5A, which a
    // simple "expect MAGIC1 next" check would reject.
    if (prev == MAGIC0 && (uint8_t)b == MAGIC1) break;
    prev = (uint8_t)b;
  }

  int kind = readByteUntil(deadline);
  if (kind < 0) return false;
  int len = readByteUntil(deadline);
  if (len < 0) return false;
  if ((uint8_t)len > sizeof(payload)) return false;

  for (uint8_t i = 0; i < (uint8_t)len; i++) {
    int b = readByteUntil(deadline);
    if (b < 0) return false;
    payload[i] = (uint8_t)b;
  }

  int crc = readByteUntil(deadline);
  if (crc < 0) return false;

  // CRC covers kind, length and payload — everything but the magic, which only
  // exists to resynchronise.
  uint8_t computed = crc8_update(crc8_update(0, (uint8_t)kind), (uint8_t)len);
  for (uint8_t i = 0; i < (uint8_t)len; i++) {
    computed = crc8_update(computed, payload[i]);
  }
  if (computed != (uint8_t)crc) return false;

  frameKind = (uint8_t)kind;
  payloadLen = (uint8_t)len;
  return true;
}

static void sendFrame(uint8_t kind, const uint8_t* data, uint8_t len) {
  uint8_t header[2] = { kind, len };
  uint8_t crc = crc8(header, 2);
  for (uint8_t i = 0; i < len; i++) {
    crc ^= data[i];
    for (uint8_t k = 0; k < 8; k++) {
      crc = (crc & 0x80) ? (uint8_t)((crc << 1) ^ 0x07) : (uint8_t)(crc << 1);
    }
  }

  Serial.write(MAGIC0);
  Serial.write(MAGIC1);
  Serial.write(kind);
  Serial.write(len);
  if (len) Serial.write(data, len);
  Serial.write(crc);
}

static void sendHello() {
  uint8_t body[5];
  body[0] = PROTOCOL_VERSION;
  body[1] = FIRMWARE_VERSION;
  body[2] = (uint8_t)(LED_COUNT & 0xFF);         // little-endian u16
  body[3] = (uint8_t)((LED_COUNT >> 8) & 0xFF);
  body[4] = MAX_BANDS;
  sendFrame(KIND_HELLO, body, sizeof(body));
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

// Spread per-band colours across the strip with linear interpolation.
//
// Uses 8.8 fixed point rather than float: an ATmega has no FPU, and a float
// multiply here would cost more than the entire rest of the frame.
static void expandBands(uint8_t bandCount) {
  if (bandCount == 0) return;

  if (bandCount == 1) {
    fill_solid(leds, LED_COUNT, CRGB(payload[0], payload[1], payload[2]));
    return;
  }

  const uint16_t lastLed = LED_COUNT - 1;
  const uint8_t lastBand = bandCount - 1;

  for (uint16_t i = 0; i < LED_COUNT; i++) {
    uint32_t pos = ((uint32_t)i * lastBand * 256u) / (lastLed ? lastLed : 1);
    uint8_t j = (uint8_t)(pos >> 8);
    uint8_t t = (uint8_t)(pos & 0xFF);
    if (j >= lastBand) { j = lastBand; t = 0; }

    const uint8_t* a = &payload[(uint16_t)j * 3];
    const uint8_t* b = &payload[(uint16_t)(j == lastBand ? j : j + 1) * 3];

    // (hi - lo) * t can reach 255*255, so the product must be wider than 16 bits.
    leds[i].r = (uint8_t)((int16_t)a[0] + (((int32_t)((int16_t)b[0] - (int16_t)a[0]) * t) >> 8));
    leds[i].g = (uint8_t)((int16_t)a[1] + (((int32_t)((int16_t)b[1] - (int16_t)a[1]) * t) >> 8));
    leds[i].b = (uint8_t)((int16_t)a[2] + (((int32_t)((int16_t)b[2] - (int16_t)a[2]) * t) >> 8));
  }
}

static void renderTest() {
  unsigned long elapsed = millis() - testStartMs;

  switch (testMode) {
    case TEST_RGB: {
      // Confirms channel order. WS2812B is physically GRB; if red shows as
      // green, the CRGB template parameter in setup() is wrong.
      uint8_t phase = (elapsed / 1000) % 3;
      CRGB c = (phase == 0) ? CRGB(255, 0, 0) : (phase == 1) ? CRGB(0, 255, 0) : CRGB(0, 0, 255);
      fill_solid(leds, LED_COUNT, c);
      break;
    }
    case TEST_CHASE: {
      // Confirms LED count and direction. If the dot vanishes early, LED_COUNT
      // is set higher than the strip really is.
      fill_solid(leds, LED_COUNT, CRGB::Black);
      leds[(elapsed / 20) % LED_COUNT] = CRGB::White;
      break;
    }
    case TEST_WHITE:
      // Reveals voltage droop. The far end turning orange before it turns dim
      // is the signature of needing power injection.
      fill_solid(leds, LED_COUNT, CRGB::White);
      break;
    default:
      break;
  }
}

static void handleFrame() {
  switch (frameKind) {
    case KIND_HELLO:
      sendHello();
      break;

    case KIND_BAND_RGB:
      testMode = TEST_NONE;
      // A truncated payload would read past the last complete triplet.
      expandBands(payloadLen / 3);
      break;

    case KIND_TEST:
      if (payloadLen >= 1) {
        testMode = payload[0];
        testStartMs = millis();
      }
      break;

    case KIND_BLACKOUT:
      testMode = TEST_NONE;
      fill_solid(leds, LED_COUNT, CRGB::Black);
      break;

    default:
      break;
  }
}

// ---------------------------------------------------------------------------

void setup() {
  Serial.begin(BAUD);

  // WS2812B is GRB internally. FastLED handles the reorder here, which is why
  // the wire protocol can stay in plain RGB order.
  FastLED.addLeds<WS2812B, LED_PIN, GRB>(leds, LED_COUNT);
  FastLED.setMaxPowerInVoltsAndMilliamps(5, MAX_MILLIAMPS);
  FastLED.clear(true);

  lastFrameMs = millis();
}

void loop() {
  // Flow control. FastLED bit-bangs WS2812B with interrupts disabled for the
  // whole strip write, and this chip's UART FIFO holds two bytes — about 40 us
  // at 500 kbaud. Streaming into that blackout loses nearly every frame, so the
  // board asks for one frame at a time and only while it can actually listen.
  Serial.write(READY);

  if (readFrame()) {
    handleFrame();
    lastFrameMs = millis();
  }

  if (testMode != TEST_NONE) {
    renderTest();
  } else if (millis() - lastFrameMs > WATCHDOG_MS) {
    // Ease out rather than cutting to black, so a brief stall looks like a fade
    // instead of a glitch.
    fadeToBlackBy(leds, LED_COUNT, 8);
  }

  FastLED.show();
}
