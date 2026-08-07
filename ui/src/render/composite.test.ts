import { describe, expect, it } from "vitest";

import { compositeLayers, staticPixels, type RenderInput } from "./composite";
import type { SpectrumFrame } from "../spectrum/paint";
import {
  makeLayer,
  type SpectrumLayer,
  type StaticLayer,
} from "../config/layers";

const LEDS = 16;

/** Nothing playing: every band on the floor. */
const SILENCE: SpectrumFrame = {
  levels: new Array(24).fill(0),
  centers: Array.from({ length: 24 }, (_, i) => 20 * Math.pow(1000, i / 23)),
};

/** Everything playing flat out. */
const FULL: SpectrumFrame = { ...SILENCE, levels: new Array(24).fill(1) };

function input(audio: SpectrumFrame): RenderInput {
  return { audio, midi: SILENCE, ledCount: LEDS, time: 0 };
}

function staticLayer(patch: Partial<StaticLayer> = {}): StaticLayer {
  const layer = makeLayer("static", LEDS) as StaticLayer;
  return {
    ...layer,
    blendMode: "normal",
    state: { ...layer.state, level: 1, from: 0, to: LEDS - 1 },
    ...patch,
  };
}

function spectrumLayer(patch: Partial<SpectrumLayer> = {}): SpectrumLayer {
  const layer = makeLayer("spectrum", LEDS) as SpectrumLayer;
  return { ...layer, blendMode: "add", ...patch };
}

describe("static layer", () => {
  it("is transparent outside its span", () => {
    const pixels = staticPixels({ ...staticLayer().state, from: 4, to: 8 }, LEDS);
    expect(pixels[0].alpha).toBe(0);
    expect(pixels[3].alpha).toBe(0);
    expect(pixels[4].alpha).toBeGreaterThan(0);
    expect(pixels[8].alpha).toBeGreaterThan(0);
    expect(pixels[9].alpha).toBe(0);
  });

  it("spreads the whole ramp across its span, however short", () => {
    const state = { ...staticLayer().state, from: 2, to: 5 };
    const pixels = staticPixels(state, LEDS);
    // First and last LED of the span are the two ends of the ramp, so a short
    // span shows the whole gradient rather than a slice of it.
    expect(pixels[2].rgb).not.toEqual(pixels[5].rgb);
  });
});

describe("compositing", () => {
  /**
   * The claim the layer model is built on, end to end: a silent spectrum layer
   * contributes nothing, so whatever is beneath survives untouched. If this
   * breaks, every stack containing a reactive layer goes black between
   * transients.
   */
  it("leaves the layer beneath intact where a reactive layer is silent", () => {
    const wash = staticLayer();
    const alone = compositeLayers([wash], input(SILENCE));
    const stacked = compositeLayers([wash, spectrumLayer()], input(SILENCE));
    expect(stacked).toEqual(alone);
  });

  it("adds light where the reactive layer is lit", () => {
    const wash = staticLayer();
    const alone = compositeLayers([wash], input(SILENCE));
    const stacked = compositeLayers([wash, spectrumLayer()], input(FULL));
    // Somewhere on the strip the stack is strictly brighter than the wash.
    const brighter = stacked.some((c, i) => c.r + c.g + c.b > alone[i].r + alone[i].g + alone[i].b);
    expect(brighter).toBe(true);
  });

  /**
   * Opacity scales coverage, not colour. At zero a layer is gone rather than
   * black, which is the same invariant as silence but reached by the other
   * control — and the one that makes an opacity slider safe to drag to the end.
   */
  it("removes a layer entirely at zero opacity", () => {
    const wash = staticLayer();
    const alone = compositeLayers([wash], input(FULL));
    const stacked = compositeLayers([wash, spectrumLayer({ opacity: 0 })], input(FULL));
    expect(stacked).toEqual(alone);
  });

  it("is unchanged by reordering two additive layers", () => {
    const a = spectrumLayer({ blendMode: "add" });
    const b = staticLayer({ blendMode: "add" });
    const ab = compositeLayers([a, b], input(FULL));
    const ba = compositeLayers([b, a], input(FULL));
    for (let i = 0; i < LEDS; i++) {
      expect(ab[i].r).toBeCloseTo(ba[i].r, 10);
      expect(ab[i].g).toBeCloseTo(ba[i].g, 10);
      expect(ab[i].b).toBeCloseTo(ba[i].b, 10);
    }
  });

  it("starts from black, so an empty stack is a dark strip", () => {
    const flat = compositeLayers([], input(FULL));
    expect(flat).toHaveLength(LEDS);
    for (const c of flat) expect(c).toEqual({ r: 0, g: 0, b: 0 });
  });
});
