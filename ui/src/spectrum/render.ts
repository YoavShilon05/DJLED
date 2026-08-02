/**
 * What the strip would look like under the current config.
 *
 * The engine cannot answer this yet — thresholds, LED sectors and the intensity
 * curve are not in the wire protocol — so the editor works it out locally. That
 * is the only way the new controls do anything visible without touching the
 * backend, and when the engine grows these fields this module becomes the
 * reference the two sides are checked against.
 */

import { oklabToDisplay, type Rgb } from "../color/display";
import { ColorSurface } from "../color/surface";
import { evalCurve } from "../config/curve";
import { toSurface, type EditorConfig, type LedKeyframe } from "../config/editor";
import { clamp, hzToNorm, normToHz } from "../config/scales";
import { levelToDb, type SpectrumFrame } from "./paint";

const BLACK: Rgb = [0, 0, 0];

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
 * `null` for an LED outside every sector: it is not addressed, so it is dark.
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
export function brightnessFor(config: EditorConfig, level: number): number {
  const db = levelToDb(level);
  if (db <= config.threshold) return 0;
  const window = config.clamp - config.threshold;
  if (window <= 0) return 1;
  return evalCurve(config.curve, clamp((db - config.threshold) / window, 0, 1));
}

export function renderStrip(
  config: EditorConfig,
  frame: SpectrumFrame,
  ledCount: number,
  masterBrightness = 1,
): Rgb[] {
  const surface = new ColorSurface(toSurface(config));
  const out: Rgb[] = new Array(ledCount);

  for (let i = 0; i < ledCount; i++) {
    const hz = ledFrequency(config.ledKeyframes, i);
    if (hz === null) {
      out[i] = BLACK;
      continue;
    }
    const level = levelAt(frame, hz);
    const gain = brightnessFor(config, level) * masterBrightness;
    // The surface's y is the same normalised level the plot's dB axis shows,
    // so a band is coloured by exactly the field pixel its bar reaches.
    out[i] = gain <= 0 ? BLACK : oklabToDisplay(surface.sample(hzToNorm(hz), level), gain);
  }
  return out;
}
