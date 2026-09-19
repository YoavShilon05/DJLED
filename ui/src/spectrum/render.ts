/**
 * What the strip would look like under the current stack.
 *
 * A port of the fold in `engine/src/color/render.rs`, and it has to stay one:
 * this is what the Preview strip draws, and the Engine strip beside it is what
 * the wall is really doing. If the two disagree the editor is lying.
 *
 * # How a layer contributes
 *
 * Each layer is sampled at every LED on *its own* axis — its own sectors, its
 * own reverse and mirror, its own thresholds, its own colour field — and the
 * results are folded bottom to top with source-over in linear light. Two rules
 * make that fold mean what the editor says it means:
 *
 * - **The intensity curve scales light, not coverage.** A band the threshold
 *   has closed is painted with its field's colour at zero intensity — black, in
 *   the stock palette — at whatever opacity that keyframe carries. So an opaque
 *   quiet band blocks, exactly like an opaque bright one. Black is a colour,
 *   not an absence; a layer that should let the one below show through in its
 *   quiet regions says so by authoring opacity there.
 * - **An LED no sector reaches is not painted at all**, rather than painted
 *   black. Sectors are how a layer is confined to part of the wall, and a layer
 *   confined to the first fifty LEDs must not blank the other hundred.
 *
 * With one layer both reduce to what this module did before the stack existed,
 * because the backdrop is an unlit strip and over black the two are
 * indistinguishable.
 *
 * A layer listening to nothing takes the same path with two of its stages taken
 * out of the way: its level is a flat full one and its field is read along the
 * single row that implies, so there is nothing for a threshold, a clamp or a
 * curve to do. That mirrors what `LiveStack::visuals` hands the engine's
 * renderer, and has to — this is the strip preview and that is the wall.
 *
 * # Time
 *
 * A layer's field can be a loop rather than a single field, and the instant of
 * that loop is taken from the wall clock — the same clock the engine reads, so
 * the two land on the same instant with nothing exchanged. `now` defaults to
 * it and is a parameter only so a test can name an instant.
 *
 * Nothing here is memoised against it. The preview is already rebuilt on every
 * frame the engine publishes, and offline on every frame the demo spectrum
 * moves, so a loop animates without a clock of its own being put on the React
 * frame path — which is the one thing this module must not cost.
 */

import { linearRgbToDisplay, type Rgb } from "../color/display";
import { CLEAR, clampedRgb, oklabToLinearRgb, over, type LinearRgb } from "../color/oklab";
import { ColorSurface } from "../color/surface";
import { nowSeconds } from "../color/timeline";
import { evalCurve } from "../config/curve";
import {
  isStatic,
  toRenderSurface,
  toRenderTimeline,
  type EditorConfig,
  type EditorLayer,
  type LedKeyframe,
} from "../config/editor";
import type { EqCurve } from "../config/eq";
import { DB_MAX, DB_MIN, clamp, hzToNorm, normToHz } from "../config/scales";
import { levelToDb, type SpectrumFrame } from "./paint";

const BLACK_RGB: Rgb = [0, 0, 0];

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
 * on the wall and change nothing about the analysis — so they are applied here
 * at the very end and are deliberately invisible to the graph.
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
 * `null` for an LED outside every sector: this layer does not address it, so it
 * contributes nothing there and whatever is beneath shows through.
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

/**
 * Brightness for a level, after the threshold, the clamp and the curve.
 *
 * Full for a layer with no source, whatever its stored window says. Those
 * controls are not shown for such a layer and do not act on it in the engine
 * either — reading them here would let a threshold left at the top of the axis
 * black out a layer whose whole point is that it does not react.
 */
export function brightnessFor(layer: EditorLayer, level: number): number {
  if (isStatic(layer)) return 1;
  const db = levelToDb(level);
  if (db <= layer.threshold) return 0;
  const window = layer.clamp - layer.threshold;
  if (window <= 0) return 1;
  return evalCurve(layer.curve, clamp((db - layer.threshold) / window, 0, 1));
}

/**
 * One layer's contribution at every LED, un-premultiplied and *not* composited
 * onto anything.
 *
 * Separated out because it is the honest unit: a layer on its own has an
 * opacity per LED, and only the fold decides what that opacity reveals.
 */
export function layerCoverage(
  layer: EditorLayer,
  frame: SpectrumFrame,
  ledCount: number,
  now = nowSeconds(),
): LinearRgb[] {
  // The loop wherever the layer has one, and its still field where it does not.
  // Sought before any position is sampled, exactly as the engine's run loop
  // seeks before it renders — sampling first would show every frame one behind.
  const surface = ColorSurface.forLayer(toRenderSurface(layer), toRenderTimeline(layer)).cycling(
    layer.cycle,
  );
  surface.seek(now);
  const out: LinearRgb[] = new Array(ledCount);
  const opacity = clamp(layer.opacity, 0, 1);
  // A layer with no source has no spectrum to read; it sits at a flat full
  // level, which is the row its flattened field was projected onto.
  const still = isStatic(layer);

  for (let i = 0; i < ledCount; i++) {
    const led = sourceIndex(i, ledCount, layer.mirror, layer.reverse);
    const hz = ledFrequency(layer.ledKeyframes, led);
    if (hz === null) {
      // Not addressed by this layer. Contributing black here would blank every
      // layer beneath it; "outside my sectors" is not a colour.
      out[i] = CLEAR;
      continue;
    }

    const level = still ? 1 : levelAt(frame, hz);
    // The surface's y is the same normalised level the plot's dB axis shows,
    // so a band is coloured by exactly the field pixel its bar reaches.
    const rgb = clampedRgb(oklabToLinearRgb(surface.sample(hzToNorm(hz), level)));
    const gain = brightnessFor(layer, level);

    // The gain scales light and leaves coverage alone. That is what makes a
    // quiet opaque band block rather than fade — it is painted black, and black
    // covers.
    out[i] = {
      r: rgb.r * gain,
      g: rgb.g * gain,
      b: rgb.b * gain,
      alpha: rgb.alpha * opacity,
    };
  }
  return out;
}

/**
 * How to get the spectrum for one layer.
 *
 * A function rather than an array because two layers can be listening to two
 * different devices on two different grids — 48 audio bands under 88 semitones
 * of MIDI — and each has to be read on its own axis before the two meet in
 * strip space. The caller is the only thing that knows which live frame belongs
 * to which layer, or what to show for a layer that has none yet.
 */
export type LayerFrames = (layer: EditorLayer, index: number) => SpectrumFrame;

/** The whole stack, composited, as the strip would show it. */
export function renderStack(
  config: EditorConfig,
  frames: LayerFrames,
  ledCount: number,
  masterBrightness = 1,
  now = nowSeconds(),
): Rgb[] {
  const stack: LinearRgb[] = new Array(ledCount).fill(CLEAR);

  config.layers.forEach((layer, index) => {
    // A layer nobody can see is skipped rather than composited at zero, which
    // is the same result for a good deal less work.
    if (!layer.enabled || layer.opacity <= 0) return;
    const contribution = layerCoverage(layer, frames(layer, index), ledCount, now);
    for (let i = 0; i < ledCount; i++) {
      if (contribution[i].alpha <= 0) continue;
      stack[i] = over(contribution[i], stack[i]);
    }
  });

  // Composite the finished stack onto the unlit strip, then scale linear light
  // by the master — which is where a brightness belongs, since the LED is
  // driven linearly.
  const out: Rgb[] = new Array(ledCount);
  for (let i = 0; i < ledCount; i++) {
    out[i] = masterBrightness <= 0 ? BLACK_RGB : linearRgbToDisplay(stack[i], masterBrightness);
  }
  return out;
}

/**
 * One layer on its own, over an unlit strip.
 *
 * The stack answers "what will the wall do"; this answers "what is *this* row
 * contributing", which is the question the layer list is asking. It is the same
 * coverage the fold above uses, composited onto black instead of onto the
 * layers below — so a row whose sectors reach only the first fifty LEDs reads
 * as fifty lit pixels and a hundred dark ones, rather than as a mystery.
 *
 * Deliberately not part of `renderStack`: the thumbnails are drawn at a
 * fraction of the strip's length and a fraction of its rate, and folding that
 * compromise into the fold the preview depends on would be the wrong trade.
 */
export function renderLayer(
  layer: EditorLayer,
  frame: SpectrumFrame,
  ledCount: number,
  masterBrightness = 1,
  now = nowSeconds(),
): Rgb[] {
  const coverage = layerCoverage(layer, frame, ledCount, now);
  const out: Rgb[] = new Array(ledCount);
  for (let i = 0; i < ledCount; i++) {
    out[i] = masterBrightness <= 0 ? BLACK_RGB : linearRgbToDisplay(coverage[i], masterBrightness);
  }
  return out;
}
