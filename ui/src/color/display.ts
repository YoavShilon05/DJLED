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
 *
 * Opacity gets two treatments, and which one is right depends on what is behind
 * the pixel:
 *
 *   - {@link oklabToDisplay} composites onto black, because the strip preview is
 *     a picture of LEDs and the thing behind an LED is an unlit LED.
 *   - {@link oklabToDisplayRgba} keeps opacity as opacity, because the colour
 *     field is drawn over the plot and a faded region should read as faded
 *     rather than as a dark patch that happens to look similar.
 */

import {
  BLACK,
  clampedRgb,
  linearToSrgb,
  oklabToLinearRgb,
  over,
  type LinearRgb,
  type Oklab,
} from "./oklab";

export type Rgb = [number, number, number];
export type Rgba = [number, number, number, number];

const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);

const srgbByte = (v: number) => Math.round(clamp01(linearToSrgb(clamp01(v))) * 255);

/**
 * Oklab to opaque sRGB bytes for the screen, composited onto black.
 *
 * `gain` scales linear light before the encode, which is where a brightness
 * multiplier belongs — halving the byte of an sRGB value does not halve the
 * light, and the LED is driven in linear terms anyway.
 */
export function oklabToDisplay(c: Oklab, gain = 1): Rgb {
  return linearRgbToDisplay(oklabToLinearRgb(c), gain);
}

/**
 * Linear RGB to opaque sRGB bytes for the screen, composited onto black.
 *
 * What {@link oklabToDisplay} is underneath, exposed separately because the
 * layer stack composites in linear RGB and has no Oklab value left by the time
 * it reaches the screen — the fold happens where light adds, not where colours
 * interpolate.
 */
export function linearRgbToDisplay(c: LinearRgb, gain = 1): Rgb {
  const rgb = over(clampedRgb(c), BLACK);
  return [srgbByte(rgb.r * gain), srgbByte(rgb.g * gain), srgbByte(rgb.b * gain)];
}

/**
 * Oklab to sRGB bytes plus an alpha byte, for drawing over something.
 *
 * Colour and alpha stay separate — canvas `ImageData` is un-premultiplied, which
 * is the same convention the colour pipeline uses, so the two line up with no
 * conversion. `gain` scales the light, not the coverage.
 */
export function oklabToDisplayRgba(c: Oklab, gain = 1): Rgba {
  const rgb = clampedRgb(oklabToLinearRgb(c));
  return [
    srgbByte(rgb.r * gain),
    srgbByte(rgb.g * gain),
    srgbByte(rgb.b * gain),
    Math.round(clamp01(rgb.alpha) * 255),
  ];
}

/** Bytes as sent to the strip (linear) to bytes for a canvas (sRGB). */
export function ledBytesToDisplay(r: number, g: number, b: number): Rgb {
  const byte = (v: number) => Math.round(clamp01(linearToSrgb(v / 255)) * 255);
  return [byte(r), byte(g), byte(b)];
}
