import { describe, expect, it } from "vitest";

import {
  DEFAULT_CONFIG,
  defaultLayer,
  nextId,
  type EditorConfig,
  type EditorLayer,
} from "../config/editor";
import { DB_MAX, DB_MIN } from "../config/scales";
import { renderLayer, renderStack } from "./render";
import type { SpectrumFrame } from "./paint";

const LEDS = 150;

/** Full level everywhere, so a render measures the fold and nothing else. */
const LOUD: SpectrumFrame = {
  levels: Array.from({ length: 48 }, () => 1),
  centers: Array.from({ length: 48 }, (_, i) => 20 * Math.pow(1000, i / 47)),
};

const SILENT: SpectrumFrame = { levels: LOUD.levels.map(() => 0), centers: LOUD.centers };

/** A layer of one flat colour, so the field shape contributes nothing. */
function flat(color: string, opacity = 1): EditorLayer {
  const base = defaultLayer("flat");
  return {
    ...base,
    id: nextId("layer"),
    opacity,
    // Threshold below the bottom of the axis, so the intensity stage is out of
    // the way entirely — including at silence, which the blocking test needs.
    threshold: DB_MIN - 20,
    clamp: DB_MIN - 19,
    blend: 0.5,
    colorKeyframes: [
      { id: nextId("ck"), hz: 20, db: DB_MIN, color, radius: null },
      { id: nextId("ck"), hz: 20_000, db: DB_MAX, color, radius: null },
    ],
  };
}

function show(...layers: EditorLayer[]): EditorConfig {
  return { ...DEFAULT_CONFIG, layers, activeLayerId: layers[layers.length - 1].id };
}

const render = (config: EditorConfig, frame: SpectrumFrame = LOUD) =>
  renderStack(config, () => frame, LEDS);

describe("the layer stack", () => {
  /** The claim the feature was asked for: red under green is green. */
  it("lets an opaque layer override the one below", () => {
    const composited = render(show(flat("#ff2000"), flat("#40ff60")));
    expect(composited).toEqual(render(show(flat("#40ff60"))));
  });

  /**
   * Layer opacity is the Photoshop slider: half of it lands halfway between the
   * two layers, not halfway to black. Over an unlit strip the two look the
   * same, which is exactly why this needs a layer underneath to test at all.
   */
  it("lerps toward what is underneath at partial opacity", () => {
    const red = render(show(flat("#ff2000")));
    const green = render(show(flat("#40ff60")));
    const blended = render(show(flat("#ff2000"), flat("#40ff60", 0.5)));

    for (let i = 0; i < LEDS; i++) {
      for (let c = 0; c < 3; c++) {
        // Compared in sRGB bytes, which is what both sides are, so the midpoint
        // is only approximate — the encode is not linear. A generous bound
        // still separates "blended" from "dimmed to black".
        const lo = Math.min(red[i][c], green[i][c]);
        const hi = Math.max(red[i][c], green[i][c]);
        expect(blended[i][c]).toBeGreaterThanOrEqual(lo - 1);
        expect(blended[i][c]).toBeLessThanOrEqual(hi + 1);
      }
    }
    expect(blended).not.toEqual(red);
    expect(blended).not.toEqual(green);
  });

  it("shows what is underneath through a fully transparent layer", () => {
    expect(render(show(flat("#ff2000"), flat("#40ff60", 0)))).toEqual(
      render(show(flat("#ff2000"))),
    );
  });

  /**
   * Black blocks and transparent does not, even though the two are
   * indistinguishable over an unlit strip. This is the distinction the whole
   * design rests on.
   */
  it("blocks with opaque black and passes through transparent black", () => {
    const blocked = render(show(flat("#ff2000"), flat("#000000")));
    expect(blocked.every((px) => px.every((c) => c === 0))).toBe(true);

    expect(render(show(flat("#ff2000"), flat("#00000000")))).toEqual(
      render(show(flat("#ff2000"))),
    );
  });

  /**
   * The same distinction on the intensity axis. A band the threshold has closed
   * is painted black, so an opaque layer blocks in its quiet regions — and
   * authoring opacity at the bottom of the field is how you ask for the other
   * behaviour.
   */
  it("blocks under the threshold when the field is opaque there", () => {
    const closed = (color: string): EditorLayer => ({
      ...flat(color),
      threshold: -20,
      clamp: -6,
    });

    const opaque = renderStack(show(flat("#ff2000"), closed("#40ff60")), () => SILENT, LEDS);
    expect(opaque.every((px) => px.every((c) => c === 0))).toBe(true);

    const faded = renderStack(show(flat("#ff2000"), closed("#40ff6000")), () => SILENT, LEDS);
    expect(faded.some((px) => px.some((c) => c > 0))).toBe(true);
  });

  /**
   * A layer confined to part of the strip must leave the rest of the stack
   * alone. Contributing black outside its sectors would turn any layer that
   * uses them into a full-strip blackout.
   */
  it("confines a sectored layer without blanking the rest", () => {
    const top: EditorLayer = {
      ...flat("#40ff60"),
      ledKeyframes: [
        { id: nextId("led"), led: 75, hz: 20 },
        { id: nextId("led"), led: 149, hz: 20_000 },
      ],
    };
    const composited = render(show(flat("#ff2000"), top));
    const red = render(show(flat("#ff2000")));

    expect(composited[0]).toEqual(red[0]);
    expect(composited[LEDS - 1]).not.toEqual(red[LEDS - 1]);
  });

  /** A hidden layer contributes nothing, which is what the eye toggle promises. */
  it("skips a disabled layer entirely", () => {
    const hidden: EditorLayer = { ...flat("#40ff60"), enabled: false };
    expect(render(show(flat("#ff2000"), hidden))).toEqual(render(show(flat("#ff2000"))));
  });

  /**
   * Two layers can be reading two different devices, so their level arrays can
   * be different lengths and sit on different axes. Nothing may index one by
   * the length of the other.
   */
  it("reads each layer on its own axis", () => {
    const notes: SpectrumFrame = {
      levels: Array.from({ length: 88 }, () => 1),
      centers: Array.from({ length: 88 }, (_, i) => 20 * Math.pow(1000, i / 87)),
    };
    const bottom = flat("#ff2000");
    const top = flat("#40ff60", 0.5);

    const out = renderStack(
      show(bottom, top),
      (layer) => (layer.id === top.id ? notes : LOUD),
      LEDS,
    );
    expect(out).toHaveLength(LEDS);
    expect(out.some((px) => px.some((c) => c > 0))).toBe(true);
  });

  /** Master brightness scales the whole stack, not one layer of it. */
  it("scales the composited result by master brightness", () => {
    const config = show(flat("#ff2000"), flat("#40ff60", 0.5));
    const full = renderStack(config, () => LOUD, LEDS, 1);
    const half = renderStack(config, () => LOUD, LEDS, 0.5);

    for (let i = 0; i < LEDS; i++) {
      for (let c = 0; c < 3; c++) expect(half[i][c]).toBeLessThanOrEqual(full[i][c]);
    }
    expect(renderStack(config, () => LOUD, LEDS, 0).every((px) => px.every((c) => c === 0))).toBe(
      true,
    );
  });
});

describe("a layer on its own", () => {
  /**
   * The thumbnail beside each row and the preview strip above have to agree
   * about what one layer looks like, or the list is pointing at the wrong row.
   *
   * They agree by construction — both fold the same `layerCoverage` — and this
   * is what pins that they still do: an opaque layer over an unlit strip is
   * the whole stack, so the two renders are the same bytes, not merely similar
   * ones.
   */
  it("renders the same bytes a one-layer stack does", () => {
    const only = flat("#ff2000");
    expect(renderLayer(only, LOUD, LEDS)).toEqual(render(show(only)));
  });

  /**
   * The thumbnails are drawn at a fraction of the strip's length, so the fold
   * has to survive being asked for a different number of LEDs — a sector
   * authored in LED indices is the part most able to fall over here.
   */
  it("answers at whatever length it is asked for", () => {
    expect(renderLayer(flat("#40ff60"), LOUD, 48)).toHaveLength(48);
  });

  /** Master brightness scales light, so zero is a black strip rather than a
   *  transparent one — an unlit LED, which is what the picture is of. */
  it("goes black at zero brightness", () => {
    const dark = renderLayer(flat("#40ff60"), LOUD, LEDS, 0);
    expect(dark.every(([r, g, b]) => r === 0 && g === 0 && b === 0)).toBe(true);
  });

});
