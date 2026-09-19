import { describe, expect, it } from "vitest";

import { DB_MAX, DB_MIN } from "../config/scales";
import { computeLayout, dbOfY, xOfHz, yOfDb } from "./layout";

const WIDTH = 900;
const HEIGHT = 400;

describe("the plot geometry", () => {
  const l = computeLayout(WIDTH, HEIGHT);

  it("puts the top and bottom of the dB axis on the plot's edges", () => {
    expect(yOfDb(l, DB_MAX)).toBe(l.plot.y);
    expect(yOfDb(l, DB_MIN)).toBe(l.plot.y + l.plot.h);
  });

  it("round-trips a dB through a pixel", () => {
    for (const db of [DB_MIN, -62, -30, -6, DB_MAX]) {
      expect(dbOfY(l, yOfDb(l, db))).toBeCloseTo(db, 6);
    }
  });

  it("leaves room beside the plot for the rail", () => {
    expect(l.rail.w).toBeGreaterThan(0);
    expect(l.plot.x + l.plot.w).toBeLessThanOrEqual(l.rail.x);
    expect(l.flat).toBe(false);
  });
});

/**
 * A layer listening to nothing has no level, so the plot has no vertical axis
 * to be. The collapse lives entirely in `yOfDb` and `dbOfY` on purpose — every
 * gizmo, hit test, drag offset and popover anchor already goes through those
 * two, so a flat plot places all of them correctly without any of them knowing
 * there is a second kind of plot. These pin that, because the day one of them
 * starts doing its own arithmetic is the day handles drift off the lane.
 */
describe("a flat plot", () => {
  const l = computeLayout(WIDTH, 96, true);

  it("puts every dB in the middle of the lane", () => {
    const middle = l.plot.y + l.plot.h / 2;
    for (const db of [DB_MIN, -62, -30, DB_MAX]) expect(yOfDb(l, db)).toBe(middle);
  });

  /**
   * A constant, so a drag cannot read a level out of a plot that has none and
   * quietly write it back over the one the layer was authored with.
   */
  it("reads the same dB at every height", () => {
    for (const y of [l.plot.y, l.plot.y + l.plot.h / 2, l.plot.y + l.plot.h]) {
      expect(dbOfY(l, y)).toBe(DB_MAX);
    }
  });

  it("gives the rail's width back to the plot", () => {
    expect(l.rail.w).toBe(0);
    expect(l.plot.w).toBeGreaterThan(computeLayout(WIDTH, 96).plot.w);
  });

  /** The horizontal axis is untouched: it is the one axis a still layer has. */
  it("keeps the frequency axis exactly where it was", () => {
    const tall = computeLayout(WIDTH, HEIGHT, true);
    for (const hz of [20, 250, 1000, 20_000]) {
      expect(xOfHz(l, hz)).toBeCloseTo(xOfHz(tall, hz), 6);
    }
  });
});
