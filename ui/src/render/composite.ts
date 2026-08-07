/**
 * The stack, flattened.
 *
 * One strip, several layers painting it. Each layer renders a full strip's
 * worth of `Pixel` and the fold runs bottom-up, so a layer's blend mode
 * describes how it meets everything already accumulated beneath it. Master
 * geometry and brightness are applied once, at the very end, to the result.
 *
 * Order of operations is load-bearing:
 *
 *   layers → fold (linear RGB) → mirror/reverse → master brightness → sRGB
 *
 * Mirror and reverse come *after* the fold because they describe the wall, not
 * any one layer; brightness comes after them because it is a dimmer, not a
 * colour decision; and the sRGB encode is last because everything before it is
 * light and only the final byte is a screen value.
 */

import { BLACK_LINEAR, blendPixel, type Pixel } from "../color/blend";
import { linearToDisplay, type Rgb } from "../color/display";
import { Gradient } from "../color/gradient";
import { oklabToLinearRgb, type LinearRgb } from "../color/oklab";
import type { Layer, SpectrumState, StaticState } from "../config/layers";
import { visibleLayers } from "../config/layers";
import type { ShowDoc } from "../config/show";
import { stateAt } from "../config/timeline";
import { sourceIndex, spectrumPixels } from "../spectrum/render";
import type { SpectrumFrame } from "../spectrum/paint";

/**
 * What the layers are rendered against.
 *
 * `midi` is separate from `audio` rather than being a second `SpectrumFrame`
 * chosen by the caller, so a stack holding one of each renders both correctly
 * in the same pass.
 */
export interface RenderInput {
  audio: SpectrumFrame;
  midi: SpectrumFrame;
  ledCount: number;
  /** Seconds on the shared transport clock. */
  time: number;
}

/** One static layer's contribution: a ramp across its own span, nothing outside. */
export function staticPixels(state: StaticState, ledCount: number): Pixel[] {
  const ramp = new Gradient(state.stops);
  const from = Math.max(0, Math.min(state.from, state.to));
  const to = Math.min(ledCount - 1, Math.max(state.from, state.to));
  const span = to - from;

  const out: Pixel[] = new Array(ledCount);
  for (let i = 0; i < ledCount; i++) {
    if (i < from || i > to) {
      out[i] = { rgb: BLACK_LINEAR, alpha: 0 };
      continue;
    }
    const p = span <= 0 ? 0 : (i - from) / span;
    out[i] = { rgb: oklabToLinearRgb(ramp.sample(p)), alpha: state.level };
  }
  return out;
}

/** A layer's pixels at a moment, with its animation already resolved. */
export function layerPixels(layer: Layer, input: RenderInput): Pixel[] {
  const state = stateAt(layer, input.time);
  if (layer.kind === "spectrum") {
    const spectrum = state as SpectrumState;
    const frame = spectrum.source === "midi" ? input.midi : input.audio;
    return spectrumPixels(spectrum, frame, input.ledCount);
  }
  return staticPixels(state as StaticState, input.ledCount);
}

/**
 * Scale a layer's coverage by its opacity.
 *
 * On the alpha rather than the colour, which is the whole point of keeping the
 * two apart: dropping a layer to 40% should let more of what is underneath
 * through, not paint a darker version of the same thing over it.
 */
function withOpacity(pixels: Pixel[], opacity: number): Pixel[] {
  if (opacity >= 1) return pixels;
  return pixels.map((p) => ({ rgb: p.rgb, alpha: p.alpha * opacity }));
}

/** The flattened stack, in linear light, before any master transform. */
export function compositeLayers(layers: Layer[], input: RenderInput): LinearRgb[] {
  const out: LinearRgb[] = new Array(input.ledCount).fill(BLACK_LINEAR);

  for (const layer of layers) {
    const pixels = withOpacity(layerPixels(layer, input), layer.opacity);
    for (let i = 0; i < input.ledCount; i++) {
      out[i] = blendPixel(out[i], pixels[i], layer.blendMode);
    }
  }
  return out;
}

/**
 * The whole show at a moment, as bytes for the screen.
 *
 * Solo is resolved here rather than by muting layers, so a soloed layer leaves
 * every other layer's `enabled` flag untouched and clearing solo restores the
 * mix exactly.
 */
export function compositeShow(
  doc: ShowDoc,
  input: RenderInput,
  masterBrightness = 1,
): Rgb[] {
  const layers = visibleLayers(doc.layers, doc.soloId);
  const flat = compositeLayers(layers, input);
  return applyMaster(flat, doc, masterBrightness);
}

/** Mirror, reverse and the master dimmer, applied to a flattened strip. */
export function applyMaster(flat: LinearRgb[], doc: ShowDoc, brightness: number): Rgb[] {
  const n = flat.length;
  const out: Rgb[] = new Array(n);
  for (let i = 0; i < n; i++) {
    const src = flat[sourceIndex(i, n, doc.master.mirror, doc.master.reverse)];
    out[i] = linearToDisplay(src, brightness);
  }
  return out;
}

/** One layer on its own, for the preview under its editor. */
export function previewLayer(layer: Layer, input: RenderInput, brightness = 1): Rgb[] {
  const pixels = withOpacity(layerPixels(layer, input), layer.opacity);
  return pixels.map((p) => linearToDisplay(blendPixel(BLACK_LINEAR, p, "normal"), brightness));
}
