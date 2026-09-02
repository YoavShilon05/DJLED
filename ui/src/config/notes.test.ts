import { describe, expect, it } from "vitest";

import {
  DEFAULT_HIGH_NOTE,
  DEFAULT_LOW_NOTE,
  MIN_NOTE_SPAN,
  noteName,
  noteRange,
  noteTicks,
  noteToHz,
  noteToNorm,
} from "./notes";
import { F_MAX, F_MIN, hzToNorm } from "./scales";

const LOW = DEFAULT_LOW_NOTE;
const HIGH = DEFAULT_HIGH_NOTE;

describe("note names", () => {
  it("puts middle C at C4", () => {
    expect(noteName(60)).toBe("C4");
    expect(noteName(21)).toBe("A0");
    expect(noteName(108)).toBe("C8");
    expect(noteName(69)).toBe("A4");
  });
});

describe("the stretched axis", () => {
  it("puts the ends of the range at the ends of the plot", () => {
    expect(noteToNorm(LOW, LOW, HIGH)).toBe(0);
    expect(noteToNorm(HIGH, LOW, HIGH)).toBe(1);
    expect(noteToHz(LOW, LOW, HIGH)).toBeCloseTo(F_MIN, 4);
    expect(noteToHz(HIGH, LOW, HIGH)).toBeCloseTo(F_MAX, 0);
  });

  /**
   * The property the whole mapping rests on: equal steps in pitch are equal
   * steps of strip. Without it a chord would bunch up at one end.
   */
  it("spaces semitones evenly", () => {
    const step = noteToNorm(LOW + 1, LOW, HIGH);
    for (let note = LOW; note < HIGH; note++) {
      const d = noteToNorm(note + 1, LOW, HIGH) - noteToNorm(note, LOW, HIGH);
      expect(d).toBeCloseTo(step, 6);
    }
  });

  /**
   * Narrowing magnifies rather than crops — the same note moves. This is the
   * behaviour the panel's hint promises, and the one thing about a stretched
   * axis that is not obvious from the two numbers defining it.
   */
  it("re-spreads the same note when the range narrows", () => {
    expect(noteToNorm(60, LOW, HIGH)).toBeCloseTo((60 - LOW) / (HIGH - LOW), 6);
    expect(noteToNorm(60, 48, 72)).toBeCloseTo(0.5, 6);
  });

  /** The editor must draw the range the engine will actually use, or the axis
   *  labels would describe a strip nobody is looking at. */
  it("clamps a hostile range the way the engine does", () => {
    expect(noteRange(100, 20)).toEqual([20, 100]);

    const [lo, hi] = noteRange(60, 61);
    expect(hi - lo).toBe(MIN_NOTE_SPAN);

    const [top, topHigh] = noteRange(127, 127);
    expect(topHigh).toBeLessThanOrEqual(127);
    expect(topHigh - top).toBeGreaterThanOrEqual(MIN_NOTE_SPAN);

    expect(noteRange(NaN, NaN)).toEqual([LOW, HIGH]);
  });
});

describe("note ticks", () => {
  it("labels octaves, and thins them on a full keyboard", () => {
    const full = noteTicks(LOW, HIGH);
    expect(full.length).toBeGreaterThan(2);
    expect(full.length).toBeLessThan(10);
    expect(full.map((t) => t.label)).toContain("C4");

    // Two octaves label every C rather than every other one.
    expect(noteTicks(48, 72).map((t) => t.label)).toEqual(["C3", "C4", "C5"]);
  });

  it("names both ends when they are not on a C", () => {
    const labels = noteTicks(LOW, HIGH).map((t) => t.label);
    expect(labels).toContain("A0");
  });

  it("places every tick inside the plot", () => {
    for (const tick of noteTicks(LOW, HIGH)) {
      const n = hzToNorm(tick.hz);
      expect(n).toBeGreaterThanOrEqual(0);
      expect(n).toBeLessThanOrEqual(1);
    }
  });
});
