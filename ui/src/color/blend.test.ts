import { describe, expect, it } from "vitest";

import { blendPixel, type Pixel } from "./blend";
import type { LinearRgb } from "./oklab";
import type { BlendMode } from "../config/layers";

const MODES: BlendMode[] = ["normal", "add", "multiply"];

const rgb = (r: number, g: number, b: number): LinearRgb => ({ r, g, b });
const px = (c: LinearRgb, alpha: number): Pixel => ({ rgb: c, alpha });

const BLACK = rgb(0, 0, 0);
const RED = rgb(1, 0, 0);
const BLUE = rgb(0, 0, 1);
const GREY = rgb(0.5, 0.5, 0.5);

describe("blend modes", () => {
  /**
   * The property the whole layer model rests on. A reactive layer is dark
   * across most of the strip most of the time, and it has to be *absent* there
   * rather than black — otherwise stacking anything under it would be pointless
   * and `normal` would be unusable.
   */
  it("leaves what is beneath untouched at zero coverage", () => {
    for (const mode of MODES) {
      expect(blendPixel(GREY, px(RED, 0), mode)).toEqual(GREY);
    }
  });

  it("replaces what is beneath at full coverage under normal", () => {
    expect(blendPixel(GREY, px(RED, 1), "normal")).toEqual(RED);
  });

  it("is a plain sum under add", () => {
    expect(blendPixel(rgb(0.25, 0, 0), px(rgb(0.5, 0, 0), 1), "add")).toEqual(rgb(0.75, 0, 0));
  });

  it("is a plain product under multiply at full coverage", () => {
    expect(blendPixel(rgb(0.5, 0.5, 0.5), px(rgb(0.5, 1, 0), 1), "multiply")).toEqual(
      rgb(0.25, 0.5, 0),
    );
  });

  it("scales the contribution by coverage, not the colour", () => {
    // Half coverage of full red over black is half the light, and the same
    // under add as under normal — which is what makes opacity mean "how much of
    // this layer" rather than "a darker version of it".
    expect(blendPixel(BLACK, px(RED, 0.5), "add")).toEqual(rgb(0.5, 0, 0));
    expect(blendPixel(BLACK, px(RED, 0.5), "normal")).toEqual(rgb(0.5, 0, 0));
  });

  /**
   * Pinned because it is the reason the layer list can say so: reordering two
   * `add` layers is a no-op, and only `normal` makes stack position visible.
   * If a future mode change breaks this, the guidance in the UI becomes wrong.
   */
  it("is order-independent for add", () => {
    const a = px(RED, 0.7);
    const b = px(BLUE, 0.4);
    const ab = blendPixel(blendPixel(BLACK, a, "add"), b, "add");
    const ba = blendPixel(blendPixel(BLACK, b, "add"), a, "add");
    expect(ab).toEqual(ba);
  });

  it("is order-dependent for normal", () => {
    const a = px(RED, 1);
    const b = px(BLUE, 1);
    const ab = blendPixel(blendPixel(BLACK, a, "normal"), b, "normal");
    const ba = blendPixel(blendPixel(BLACK, b, "normal"), a, "normal");
    expect(ab).not.toEqual(ba);
  });

  /**
   * Individually commutative modes still do not commute with *each other*,
   * which is the non-obvious half of "order matters".
   */
  it("is order-dependent once modes are mixed", () => {
    const base = rgb(0.5, 0.5, 0.5);
    const add = px(rgb(0.3, 0.3, 0.3), 1);
    const mul = px(rgb(0.5, 0.5, 0.5), 1);

    const addThenMul = blendPixel(blendPixel(base, add, "add"), mul, "multiply");
    const mulThenAdd = blendPixel(blendPixel(base, mul, "multiply"), add, "add");

    expect(addThenMul.r).toBeCloseTo(0.4, 10);
    expect(mulThenAdd.r).toBeCloseTo(0.55, 10);
  });
});
