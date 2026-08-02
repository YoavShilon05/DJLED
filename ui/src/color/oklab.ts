/**
 * Oklab conversions — a faithful port of `engine/src/color/oklab.rs`.
 *
 * This exists so the editor can show what the strip will actually do. That only
 * works if the two implementations agree, so the coefficients here are copied
 * verbatim from the Rust and pinned by tests in `oklab.test.ts`. If you change
 * one side, change both.
 *
 * Three spaces are in play and conflating them produces visibly wrong output:
 *   - sRGB        gamma-encoded, what a hex code gives you
 *   - linear RGB  proportional to light output, what a WS2812 PWM byte controls
 *   - Oklab       perceptually uniform, where interpolation belongs
 */

export interface Oklab {
  l: number;
  a: number;
  b: number;
}

export interface LinearRgb {
  r: number;
  g: number;
  b: number;
}

export function oklabToLinearRgb(c: Oklab): LinearRgb {
  const l_ = c.l + 0.3963377774 * c.a + 0.2158037573 * c.b;
  const m_ = c.l - 0.1055613458 * c.a - 0.0638541728 * c.b;
  const s_ = c.l - 0.0894841775 * c.a - 1.291485548 * c.b;

  const l = l_ * l_ * l_;
  const m = m_ * m_ * m_;
  const s = s_ * s_ * s_;

  return {
    r: 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    g: -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    b: -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  };
}

export function linearRgbToOklab(c: LinearRgb): Oklab {
  const l = 0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b;
  const m = 0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b;
  const s = 0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b;

  // Cube root rather than `** (1/3)`: defined for negatives, which occur for
  // out-of-gamut inputs and would otherwise give NaN.
  const l_ = Math.cbrt(l);
  const m_ = Math.cbrt(m);
  const s_ = Math.cbrt(s);

  return {
    l: 0.2104542553 * l_ + 0.793617785 * m_ - 0.0040720468 * s_,
    a: 1.9779984951 * l_ - 2.428592205 * m_ + 0.4505937099 * s_,
    b: 0.0259040371 * l_ + 0.7827717662 * m_ - 0.808675766 * s_,
  };
}

/** sRGB electro-optical transfer function: encoded value to linear light. */
export function srgbToLinear(c: number): number {
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

/** Inverse of {@link srgbToLinear}. Needed to display a colour on a monitor. */
export function linearToSrgb(c: number): number {
  return c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;
}

export function hexToLinearRgb(hex: string): LinearRgb {
  const t = hex.trim().replace(/^#/, "");
  const n = parseInt(t, 16);
  return {
    r: srgbToLinear(((n >> 16) & 0xff) / 255),
    g: srgbToLinear(((n >> 8) & 0xff) / 255),
    b: srgbToLinear((n & 0xff) / 255),
  };
}

export function hexToOklab(hex: string): Oklab {
  return linearRgbToOklab(hexToLinearRgb(hex));
}

const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);

/**
 * Oklab to an `#rrggbb` string for display in the browser.
 *
 * Note the sRGB encode at the end. That is correct *here* and wrong on the path
 * to the LEDs: a monitor expects gamma-encoded sRGB, whereas WS2812 brightness
 * is linear in the byte value, so the firmware receives linear values directly.
 */
export function oklabToHex(c: Oklab): string {
  const rgb = oklabToLinearRgb(c);
  const byte = (v: number) =>
    Math.round(clamp01(linearToSrgb(clamp01(v))) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${byte(rgb.r)}${byte(rgb.g)}${byte(rgb.b)}`;
}

/**
 * The bytes the firmware would receive: linear light, no sRGB encode.
 * Use this anywhere the goal is to predict the strip rather than to draw on a
 * screen.
 */
export function oklabToLedBytes(c: Oklab): [number, number, number] {
  const rgb = oklabToLinearRgb(c);
  const byte = (v: number) => Math.round(clamp01(v) * 255);
  return [byte(rgb.r), byte(rgb.g), byte(rgb.b)];
}
