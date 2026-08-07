/**
 * How one layer combines with everything beneath it.
 *
 * **Compositing happens in linear RGB, not Oklab.** That is not an oversight:
 * `add` is two lamps pointed at one wall and `multiply` is a gel in front of
 * one, and both of those are physics rather than perception. A WS2812 is driven
 * in linear terms too, so a stack composited linearly predicts the wall.
 * Oklab keeps the job it is good at — interpolating *within* a layer, where the
 * question is what looks like an even gradient rather than what a photon does.
 *
 * **Brightness is alpha.** A layer that renders black at an LED is not painting
 * black there, it is absent there. Without that, `normal` would be useless for
 * anything reactive: a spectrum layer is dark across most of the strip most of
 * the time, and an opaque black would wipe out every layer under it between
 * transients. Each renderer therefore returns colour and coverage separately,
 * and it is the *coverage* that a layer's opacity scales.
 */

import type { LinearRgb } from "./oklab";
import type { BlendMode } from "../config/layers";

/** A layer's contribution at one LED: unpremultiplied colour, plus coverage. */
export interface Pixel {
  rgb: LinearRgb;
  /** 0..1. Zero means this layer is not present at this LED at all. */
  alpha: number;
}

export const BLACK_PIXEL: Pixel = { rgb: { r: 0, g: 0, b: 0 }, alpha: 0 };

export const BLACK_LINEAR: LinearRgb = { r: 0, g: 0, b: 0 };

const lerp = (x: number, y: number, t: number) => x + (y - x) * t;

/**
 * Fold one layer onto the accumulated result beneath it.
 *
 * Every mode is written so that `alpha === 0` is exactly `dst`, which is what
 * makes an absent layer free rather than merely cheap.
 */
export function blendPixel(dst: LinearRgb, src: Pixel, mode: BlendMode): LinearRgb {
  const a = src.alpha;
  if (a <= 0) return dst;

  switch (mode) {
    // Coverage-weighted replacement. The layer wins where it is lit and shows
    // what is under it where it is not.
    case "normal":
      return {
        r: lerp(dst.r, src.rgb.r, a),
        g: lerp(dst.g, src.rgb.g, a),
        b: lerp(dst.b, src.rgb.b, a),
      };

    // Light adds. Left unclipped here so a chain of layers accumulates honestly;
    // the clip belongs at the encode, where the gamut actually ends.
    case "add":
      return {
        r: dst.r + src.rgb.r * a,
        g: dst.g + src.rgb.g * a,
        b: dst.b + src.rgb.b * a,
      };

    // A gel: the product is what gets through, and alpha says how much of the
    // gel is actually in the way.
    case "multiply":
      return {
        r: lerp(dst.r, dst.r * src.rgb.r, a),
        g: lerp(dst.g, dst.g * src.rgb.g, a),
        b: lerp(dst.b, dst.b * src.rgb.b, a),
      };
  }
}

/** One line each, for the inspector — what the mode does, not how. */
export const BLEND_HINT: Record<BlendMode, string> = {
  normal: "Wins where it is lit, shows what is beneath where it is dark.",
  add: "Adds its light to everything beneath, the way two lamps do.",
  multiply: "Filters what is beneath, like a coloured gel over a lamp.",
};
