import { describe, expect, it } from "vitest";

import { EqCurve, bandGainAt, type EqBand, type EqType } from "./eq";

const FS = 48_000;

function band(type: EqType, hz: number, gain: number, q: number): EqBand {
  return { id: `${type}-${hz}`, type, hz, gain, q };
}

/**
 * These assert closed-form properties of the RBJ biquad, not values copied out
 * of a previous run — a transcription error in the coefficients fails them
 * rather than being baked in.
 */
describe("eq response", () => {
  it("puts a bell's full gain exactly at its centre frequency", () => {
    // |H(w0)| = A^2 for the peaking filter, which is the requested gain in dB.
    for (const gain of [-18, -6, 3, 12]) {
      expect(bandGainAt(band("peak", 1000, gain, 1.4), 1000, FS)).toBeCloseTo(gain, 6);
    }
  });

  it("leaves a bell's gain behind well away from centre", () => {
    // A bell's skirt is asymptotic, so this is "negligible", not "zero": at
    // 40 Hz a 12 dB Q=2 bell at 1 kHz still contributes a few thousandths.
    const b = band("peak", 1000, 12, 2);
    expect(Math.abs(bandGainAt(b, 40, FS))).toBeLessThan(0.05);
    expect(Math.abs(bandGainAt(b, 18_000, FS))).toBeLessThan(0.2);
  });

  it("widens a bell as Q falls", () => {
    const at = (q: number) => bandGainAt(band("peak", 1000, 12, q), 500, FS);
    expect(at(0.3)).toBeGreaterThan(at(1));
    expect(at(1)).toBeGreaterThan(at(6));
  });

  it("shelves one side and not the other", () => {
    const low = band("lowShelf", 500, 9, 0.707);
    expect(bandGainAt(low, 25, FS)).toBeCloseTo(9, 1);
    expect(bandGainAt(low, 15_000, FS)).toBeCloseTo(0, 1);

    const high = band("highShelf", 2000, -9, 0.707);
    expect(bandGainAt(high, 15_000, FS)).toBeCloseTo(-9, 1);
    expect(bandGainAt(high, 25, FS)).toBeCloseTo(0, 1);
  });

  it("passes at -3 dB through a Butterworth cutoff", () => {
    // |H(w0)| = Q for both pass types, so Q = 1/sqrt(2) is the -3.01 dB point.
    const q = Math.SQRT1_2;
    expect(bandGainAt(band("lowPass", 1000, 0, q), 1000, FS)).toBeCloseTo(-3.0103, 3);
    expect(bandGainAt(band("highPass", 1000, 0, q), 1000, FS)).toBeCloseTo(-3.0103, 3);
  });

  it("cuts on the far side of a pass filter", () => {
    const low = band("lowPass", 1000, 0, Math.SQRT1_2);
    expect(bandGainAt(low, 100, FS)).toBeCloseTo(0, 2);
    expect(bandGainAt(low, 8000, FS)).toBeLessThan(-30);

    const high = band("highPass", 1000, 0, Math.SQRT1_2);
    expect(bandGainAt(high, 8000, FS)).toBeCloseTo(0, 2);
    expect(bandGainAt(high, 100, FS)).toBeLessThan(-30);
  });

  it("adds cascaded bands in dB", () => {
    const a = band("peak", 200, 6, 1);
    const b = band("peak", 5000, -4, 1);
    const curve = new EqCurve([a, b], FS);
    for (const hz of [50, 200, 1000, 5000, 16_000]) {
      expect(curve.gainAt(hz)).toBeCloseTo(bandGainAt(a, hz, FS) + bandGainAt(b, hz, FS), 9);
    }
  });

  it("is flat with no bands", () => {
    const curve = new EqCurve([], FS);
    expect(curve.isFlat).toBe(true);
    expect(curve.gainAt(1000)).toBe(0);
  });

  it("stays finite for a band pushed past Nyquist", () => {
    // The frequency axis reaches 20 kHz, which is above Nyquist at 32 kHz.
    expect(Number.isFinite(bandGainAt(band("peak", 20_000, 12, 4), 20_000, 32_000))).toBe(true);
    expect(Number.isFinite(bandGainAt(band("lowPass", 19_000, 0, 1), 20_000, 32_000))).toBe(true);
  });
});
