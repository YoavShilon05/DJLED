/**
 * What one spectrum layer paints, per LED.
 *
 * The engine cannot answer this — thresholds, LED sectors, the intensity curve
 * and the whole layer stack are not in the wire protocol — so the editor works
 * it out locally. That is the only way the controls do anything visible without
 * touching the backend, and when the engine grows these fields this module
 * becomes the reference the two sides are checked against.
 *
 * Colour and brightness are returned *separately*, as a `Pixel`. The old
 * single-layer version multiplied the gain straight into the colour, which is
 * correct when there is nothing underneath; with a stack it would throw away
 * the coverage the compositor needs to know a dark band is absent rather than
 * black. See `color/blend.ts`.
 */

import { BLACK_PIXEL, type Pixel } from "../color/blend";
import { oklabToLinearRgb } from "../color/oklab";
import { ColorSurface } from "../color/surface";
import { evalCurve } from "../config/curve";
import { toSurface, type LedKeyframe, type SpectrumState } from "../config/layers";
import type { EqCurve } from "../config/eq";
import { DB_MAX, DB_MIN, clamp, hzToNorm, normToHz } from "../config/scales";
import { levelToDb, type SpectrumFrame } from "./paint";

const DB_SPAN = DB_MAX - DB_MIN;

/**
 * The EQ, for the offline preview only.
 *
 * When the engine is connected its levels already have the EQ in them — it is
 * applied there between the AGC and the range map, which is where it belongs —
 * so applying it again here would double it. This exists so the curve still
 * does something visible with nothing running.
 *
 * It is an approximation of the engine's version, in two ways worth knowing:
 * the gain is a fraction of the analyser's dB window (hence `dbSpan`, reported
 * by the engine), and a band already at zero is left alone, because the editor
 * cannot see how far below the floor it actually sits and a boost applied to a
 * floored level would make an idle strip glow.
 */
export function applyEq(frame: SpectrumFrame, curve: EqCurve, dbSpan: number): SpectrumFrame {
  if (curve.isFlat) return frame;
  const span = dbSpan > 0 ? dbSpan : DB_SPAN;
  return {
    centers: frame.centers,
    levels: frame.levels.map((level, i) => {
      const hz = frame.centers[i];
      if (hz === undefined || level <= 0) return level;
      return clamp(level + curve.gainAt(hz) / span, 0, 1);
    }),
  };
}

/**
 * Which logical LED an output position shows.
 *
 * Both transforms are spatial, not spectral — they rearrange where colours land
 * on the wall and change nothing about the analysis — so they are applied at
 * the very end, to the *composited* strip, and are deliberately invisible to
 * every graph. That is also why they live on the master rather than on a layer:
 * they describe how the strip is mounted, and two layers disagreeing about
 * which end is which is not an effect anyone wants.
 *
 * Mirror folds the whole range into each half, so both ends of the strip show
 * the low end and the centre shows the high end. Reverse then flips what that
 * lookup returns, which is what puts bass in the middle when both are on.
 */
export function sourceIndex(i: number, n: number, mirror: boolean, reverse: boolean): number {
  if (n <= 1) return 0;
  let u = i / (n - 1);
  if (mirror) u = u < 0.5 ? u * 2 : (1 - u) * 2;
  if (reverse) u = 1 - u;
  return Math.round(u * (n - 1));
}

/**
 * The frequency an LED displays.
 *
 * Sectors are defined by index, so the keyframes are ordered by LED here even
 * though the editor keeps them ordered by frequency — a sector may run
 * downwards in frequency if that is how it was authored. Interpolation is
 * linear on the *log* axis, which is what makes "LED 0 at 20 Hz, LED 50 at
 * 2 kHz" spread evenly across the first 50 LEDs rather than piling seven
 * octaves into the last few.
 *
 * `null` for an LED outside every sector: the layer does not address it, so it
 * is transparent there — which is what lets one spectrum layer cover part of a
 * strip and another cover the rest.
 */
export function ledFrequency(keyframes: LedKeyframe[], led: number): number | null {
  const sorted = [...keyframes].sort((a, b) => a.led - b.led);
  if (sorted.length === 0) return null;
  if (led < sorted[0].led || led > sorted[sorted.length - 1].led) return null;

  for (let i = 0; i < sorted.length - 1; i++) {
    const a = sorted[i];
    const b = sorted[i + 1];
    if (led > b.led) continue;
    const span = b.led - a.led;
    const t = span === 0 ? 0 : (led - a.led) / span;
    return normToHz(hzToNorm(a.hz) + t * (hzToNorm(b.hz) - hzToNorm(a.hz)));
  }
  return sorted[sorted.length - 1].hz;
}

/** The analyser's level at an arbitrary frequency, interpolated on the log axis. */
export function levelAt(frame: SpectrumFrame, hz: number): number {
  const { levels, centers } = frame;
  const n = Math.min(levels.length, centers.length);
  if (n === 0) return 0;
  if (hz <= centers[0]) return levels[0];
  if (hz >= centers[n - 1]) return levels[n - 1];

  for (let i = 1; i < n; i++) {
    if (hz > centers[i]) continue;
    const t =
      (hzToNorm(hz) - hzToNorm(centers[i - 1])) /
      (hzToNorm(centers[i]) - hzToNorm(centers[i - 1]) || 1);
    return levels[i - 1] + t * (levels[i] - levels[i - 1]);
  }
  return levels[n - 1];
}

/** Brightness for a level, after the threshold, the clamp and the curve. */
export function brightnessFor(state: SpectrumState, level: number): number {
  const db = levelToDb(level);
  if (db <= state.threshold) return 0;
  const window = state.clamp - state.threshold;
  if (window <= 0) return 1;
  return evalCurve(state.curve, clamp((db - state.threshold) / window, 0, 1));
}

/**
 * One spectrum layer's contribution to every LED.
 *
 * Note what alpha is: the brightness the threshold/clamp/curve chain produced.
 * That is the whole reason a reactive layer stacks sensibly — where a band is
 * below threshold the layer is *absent*, not black, so whatever is beneath it
 * survives.
 */
export function spectrumPixels(
  state: SpectrumState,
  frame: SpectrumFrame,
  ledCount: number,
): Pixel[] {
  const surface = new ColorSurface(toSurface(state));
  const out: Pixel[] = new Array(ledCount);

  for (let i = 0; i < ledCount; i++) {
    const hz = ledFrequency(state.ledKeyframes, i);
    if (hz === null) {
      out[i] = BLACK_PIXEL;
      continue;
    }
    const level = levelAt(frame, hz);
    const alpha = brightnessFor(state, level);
    if (alpha <= 0) {
      out[i] = BLACK_PIXEL;
      continue;
    }
    // The surface's y is the same normalised level the plot's dB axis shows,
    // so a band is coloured by exactly the field pixel its bar reaches.
    out[i] = { rgb: oklabToLinearRgb(surface.sample(hzToNorm(hz), level)), alpha };
  }
  return out;
}
