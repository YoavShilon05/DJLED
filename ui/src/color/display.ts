/**
 * Screen-side colour conversion.
 *
 * `oklabToLedBytes` is deliberately linear-light — that is what WS2812 wants.
 * Drawing those same bytes into a canvas is a category error: the browser reads
 * them as sRGB, so an authored `#40ff60` would paint back as something visibly
 * darker and the editor would misreport the colour the user just picked.
 *
 * Everything drawn on screen goes through here, so a keyframe's swatch, the
 * field under it and the strip preview all agree.
 */

import { linearToSrgb, oklabToLinearRgb, type Oklab } from "./oklab";

export type Rgb = [number, number, number];

const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);

/**
 * Oklab to sRGB bytes for the screen.
 *
 * `gain` scales linear light before the encode, which is where a brightness
 * multiplier belongs — halving the byte of an sRGB value does not halve the
 * light, and the LED is driven in linear terms anyway.
 */
export function oklabToDisplay(c: Oklab, gain = 1): Rgb {
  const rgb = oklabToLinearRgb(c);
  const byte = (v: number) => Math.round(clamp01(linearToSrgb(clamp01(v * gain))) * 255);
  return [byte(rgb.r), byte(rgb.g), byte(rgb.b)];
}

/** Bytes as sent to the strip (linear) to bytes for a canvas (sRGB). */
export function ledBytesToDisplay(r: number, g: number, b: number): Rgb {
  const byte = (v: number) => Math.round(clamp01(linearToSrgb(v / 255)) * 255);
  return [byte(r), byte(g), byte(b)];
}
