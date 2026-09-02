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
 *
 * Every colour carries an `alpha` alongside its three colour channels. With one
 * layer composited onto an unlit strip that reads as a straight dimming, but
 * that equivalence is a property of the backdrop being black and it ends as soon
 * as layers stack. Two rules follow, and both are mirrored in the Rust:
 *
 *   - Colour is stored un-premultiplied, so `l`/`a`/`b` mean the same thing at
 *     any opacity and fading a keyframe out never drags its hue toward black.
 *   - Alpha is linear coverage, so it gets no sRGB transfer function either way.
 *
 * Note `a` (a chroma axis) and `alpha` (coverage) are unrelated despite the
 * names.
 */

export interface Oklab {
  l: number;
  a: number;
  b: number;
  /** Opacity, 0 = invisible, 1 = fully covering. */
  alpha: number;
}

export interface LinearRgb {
  r: number;
  g: number;
  b: number;
  /** Opacity, 0 = invisible, 1 = fully covering. */
  alpha: number;
}

/** The unlit strip: what the whole stack is finally composited onto. */
export const BLACK: LinearRgb = { r: 0, g: 0, b: 0, alpha: 1 };

/**
 * Nothing at all — the identity the layer stack accumulates from.
 *
 * Not the same as {@link BLACK}, and the difference is the whole point of the
 * stack: black *covers* what is under it, transparent does not.
 */
export const CLEAR: LinearRgb = { r: 0, g: 0, b: 0, alpha: 0 };

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
    // Un-premultiplied, so opacity survives the colour conversion untouched.
    alpha: c.alpha,
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
    alpha: c.alpha,
  };
}

/**
 * Source-over composite: `src` painted on top of `backdrop`, in linear light.
 *
 * The operator that gives opacity its meaning, and what layer stacking will fold
 * over a stack. With {@link BLACK} as the backdrop it reduces to
 * `colour × alpha`, which is why one translucent layer currently looks like
 * nothing more than a dimmer one.
 */
export function over(src: LinearRgb, backdrop: LinearRgb): LinearRgb {
  const s = clamp01(src.alpha);
  const under = clamp01(backdrop.alpha) * (1 - s);
  const alpha = s + under;
  if (alpha <= 0) return { r: 0, g: 0, b: 0, alpha: 0 };
  // Divide back out, so the result stays un-premultiplied like its inputs.
  return {
    r: (src.r * s + backdrop.r * under) / alpha,
    g: (src.g * s + backdrop.g * under) / alpha,
    b: (src.b * s + backdrop.b * under) / alpha,
    alpha,
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

/**
 * Parse `#rrggbb` or `#rrggbbaa`, with or without the hash.
 *
 * Six digits is fully opaque, which is what keeps presets written before opacity
 * existed loading with the appearance they had. Only the colour channels get the
 * sRGB decode — alpha is coverage, not a light level, exactly as CSS treats it.
 */
export function hexToLinearRgb(hex: string): LinearRgb {
  const t = hex.trim().replace(/^#/, "");
  const rgb = parseInt(t.slice(0, 6), 16);
  const alpha = t.length >= 8 ? parseInt(t.slice(6, 8), 16) / 255 : 1;
  return {
    r: srgbToLinear(((rgb >> 16) & 0xff) / 255),
    g: srgbToLinear(((rgb >> 8) & 0xff) / 255),
    b: srgbToLinear((rgb & 0xff) / 255),
    alpha: Number.isFinite(alpha) ? alpha : 1,
  };
}

export function hexToOklab(hex: string): Oklab {
  return linearRgbToOklab(hexToLinearRgb(hex));
}

const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);

/**
 * Oklab to a CSS hex string — `#rrggbb` when opaque, `#rrggbbaa` when not.
 *
 * Six digits for an opaque colour so the common case stays the short familiar
 * form, and because that is the shape the engine's default palette is written
 * in; both are accepted everywhere either is.
 *
 * Note the sRGB encode on the colour channels. That is correct *here* and wrong
 * on the path to the LEDs: a monitor expects gamma-encoded sRGB, whereas WS2812
 * brightness is linear in the byte value, so the firmware receives linear values
 * directly. Alpha is encoded either way — it is coverage, not light.
 */
export function oklabToHex(c: Oklab): string {
  const rgb = oklabToLinearRgb(c);
  const hex = (v: number) =>
    Math.round(clamp01(v) * 255)
      .toString(16)
      .padStart(2, "0");
  const byte = (v: number) => hex(linearToSrgb(clamp01(v)));
  const base = `#${byte(rgb.r)}${byte(rgb.g)}${byte(rgb.b)}`;
  return rgb.alpha >= 1 ? base : `${base}${hex(rgb.alpha)}`;
}

/**
 * The bytes the firmware would receive: linear light, composited onto the unlit
 * strip, no sRGB encode. Use this anywhere the goal is to predict the strip
 * rather than to draw on a screen.
 */
export function oklabToLedBytes(c: Oklab): [number, number, number] {
  // Clamped *before* compositing, matching the Rust. Clamping after would let a
  // slightly out-of-gamut channel survive as a fraction of itself rather than
  // being pinned to 1, and the two ports would disagree by a byte.
  const rgb = over(clampedRgb(oklabToLinearRgb(c)), BLACK);
  const byte = (v: number) => Math.round(clamp01(v) * 255);
  return [byte(rgb.r), byte(rgb.g), byte(rgb.b)];
}

/** Clamp into the representable cube, and opacity into 0..1. */
export function clampedRgb(c: LinearRgb): LinearRgb {
  return { r: clamp01(c.r), g: clamp01(c.g), b: clamp01(c.b), alpha: clamp01(c.alpha) };
}
