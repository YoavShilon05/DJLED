/**
 * Colour conversions the editor owns end to end.
 *
 * The surface itself is pinned against the Rust by `reference.test.ts`. What is
 * *not* covered there is the hex boundary: Mantine's picker hands back
 * `#rrggbbaa` and the surface has to read it, and `oklabToHex` has to hand back
 * something the picker and the engine will both accept. A break in that loop
 * silently drops whatever opacity the user just dialled in, which looks like the
 * feature not working rather than like a bug.
 */

import { describe, expect, it } from "vitest";

import {
  BLACK,
  hexToLinearRgb,
  hexToOklab,
  linearRgbToOklab,
  oklabToHex,
  oklabToLedBytes,
  oklabToLinearRgb,
  over,
  srgbToLinear,
  tint,
  type LinearRgb,
} from "./oklab";

const close = (a: number, b: number, tol = 1e-6) => Math.abs(a - b) < tol;

describe("hex", () => {
  it("treats six digits as fully opaque", () => {
    // The compatibility guarantee: presets predate opacity and must not shift.
    expect(hexToLinearRgb("#ff2000").alpha).toBe(1);
    expect(hexToLinearRgb("ff2000").alpha).toBe(1);
  });

  it("reads the opacity byte as coverage, not as light", () => {
    // Half coverage is 128/255, not the sRGB decode of it (~0.216). Getting this
    // wrong makes every authored opacity read far too low.
    expect(close(hexToLinearRgb("#ff200080").alpha, 128 / 255)).toBe(true);
    expect(hexToLinearRgb("#ff2000ff").alpha).toBe(1);
    expect(hexToLinearRgb("#ff200000").alpha).toBe(0);
  });

  it("keeps colour independent of opacity", () => {
    const opaque = hexToLinearRgb("#40ff60");
    for (const hex of ["#40ff60ff", "#40ff6080", "#40ff6000"]) {
      const faded = hexToLinearRgb(hex);
      expect(close(faded.r, opaque.r), hex).toBe(true);
      expect(close(faded.g, opaque.g), hex).toBe(true);
      expect(close(faded.b, opaque.b), hex).toBe(true);
    }
  });

  it("round-trips through Oklab and back to a string", () => {
    for (const hex of ["#ff2000", "#40ff6080", "#00b3ff40", "#00000000"]) {
      expect(oklabToHex(hexToOklab(hex))).toBe(hex);
    }
  });

  it("emits six digits when opaque and eight when not", () => {
    // Short form for the common case, because that is what the engine's default
    // palette is written in and what a colour picker expects back.
    expect(oklabToHex(hexToOklab("#ff2000"))).toBe("#ff2000");
    expect(oklabToHex(hexToOklab("#ff2000ff"))).toBe("#ff2000");
    expect(oklabToHex(hexToOklab("#ff2000fe"))).toHaveLength(9);
  });
});

describe("compositing", () => {
  const red: LinearRgb = { r: srgbToLinear(1), g: 0, b: 0, alpha: 1 };
  const blue: LinearRgb = { r: 0, g: 0, b: srgbToLinear(1), alpha: 1 };

  it("scales the light when the backdrop is black", () => {
    for (let i = 0; i <= 10; i++) {
      const alpha = i / 10;
      const out = over({ ...red, alpha }, BLACK);
      expect(close(out.r, red.r * alpha), `alpha ${alpha}`).toBe(true);
      expect(close(out.alpha, 1)).toBe(true);
    }
  });

  it("does not, over anything else", () => {
    // The case layer stacking exists for: opacity is not a second brightness.
    const overBlue = over({ ...red, alpha: 0.5 }, blue);
    const overBlack = over({ ...red, alpha: 0.5 }, BLACK);
    expect(overBlue.b).toBeGreaterThan(0.4 * blue.b);
    expect(Math.abs(overBlue.b - overBlack.b)).toBeGreaterThan(0.05);
  });

  it("handles the degenerate ends without dividing by zero", () => {
    expect(close(over(red, blue).r, red.r)).toBe(true);
    expect(close(over({ ...red, alpha: 0 }, blue).b, blue.b)).toBe(true);

    const empty = over({ r: 1, g: 1, b: 1, alpha: 0 }, { r: 1, g: 1, b: 1, alpha: 0 });
    expect(empty.alpha).toBe(0);
    expect(Number.isFinite(empty.r)).toBe(true);
  });

  it("composites onto black on the way to the LEDs", () => {
    const [r] = oklabToLedBytes(hexToOklab("#ff2000"));
    const [half] = oklabToLedBytes(hexToOklab("#ff200080"));
    expect(Math.abs(half - Math.round(r * (128 / 255)))).toBeLessThanOrEqual(1);
    expect(oklabToLedBytes(hexToOklab("#ff200000"))).toEqual([0, 0, 0]);
  });
});

describe("colour conversion", () => {
  it("carries opacity through untouched rather than smearing it into lightness", () => {
    for (const alpha of [0, 0.25, 0.5, 1]) {
      const rgb: LinearRgb = { r: 0.3, g: 0.6, b: 0.1, alpha };
      expect(linearRgbToOklab(rgb).alpha).toBe(alpha);
      expect(oklabToLinearRgb(linearRgbToOklab(rgb)).alpha).toBe(alpha);
    }
  });
});

describe("tint", () => {
  const field = hexToOklab("#ff2000");
  const channel = hexToOklab("#0040ff");

  /**
   * The two ends of the opacity slider, which are the two claims a channel
   * colour makes: at full it *is* the colour, at none it was never there.
   */
  it("replaces the colour at full opacity and does nothing at none", () => {
    expect(tint(field, { ...channel, alpha: 1 })).toEqual({ ...channel, alpha: field.alpha });
    expect(tint(field, { ...channel, alpha: 0 })).toEqual(field);
  });

  /** Halfway is the Oklab midpoint — not a midpoint in RGB, and not a
   *  composite. */
  it("mixes in Oklab, linearly in the overlay's opacity", () => {
    const mid = tint(field, { ...channel, alpha: 0.5 });
    expect(close(mid.l, (field.l + channel.l) / 2)).toBe(true);
    expect(close(mid.a, (field.a + channel.a) / 2)).toBe(true);
    expect(close(mid.b, (field.b + channel.b) / 2)).toBe(true);
  });

  /**
   * Opacity here is how much of the colour to take, never coverage. The base
   * keeps its own alpha whatever the overlay's is, which is what stops a
   * palette from changing what the layers underneath contribute.
   */
  it("keeps the base's alpha, whatever the overlay's", () => {
    const faded = { ...field, alpha: 0.25 };
    for (const alpha of [0, 0.5, 1]) {
      expect(tint(faded, { ...channel, alpha }).alpha).toBe(0.25);
    }
  });

  /** A hostile alpha is clamped rather than extrapolated past the two
   *  colours. */
  it("clamps an out-of-range opacity", () => {
    expect(tint(field, { ...channel, alpha: 2 })).toEqual({ ...channel, alpha: field.alpha });
    expect(tint(field, { ...channel, alpha: -1 })).toEqual(field);
  });
});
