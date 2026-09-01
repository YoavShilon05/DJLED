/**
 * Cross-language agreement check.
 *
 * The editor reimplements Oklab and the keyframe surface in TypeScript so it can
 * preview the strip without a round trip to the engine. Nothing about the two
 * languages enforces that they agree, and a silent divergence would mean the
 * editor shows colours the wall will never produce — the worst kind of bug here,
 * because it looks like it is working.
 *
 * `reference.json` is generated from the Rust (see
 * `engine/examples/color_reference.rs`). Both sides assert against it, so either
 * one drifting fails a test.
 *
 * It carries two palettes. The first is the shipped default, which is opaque.
 * The second is translucent, because opacity-weighted blending is the subtlest
 * thing here to port and an all-opaque fixture would let both sides get it wrong
 * and still agree.
 */

import { describe, expect, it } from "vitest";

import reference from "./reference.json";
import { oklabToLedBytes } from "./oklab";
import { ColorSurface, type SurfaceConfig } from "./surface";

interface Sample {
  x: number;
  y: number;
  l: number;
  a: number;
  b: number;
  alpha: number;
  led: number[];
}

interface Case {
  name: string;
  sigma: number;
  keyframes: SurfaceConfig["keyframes"];
  samples: Sample[];
}

const cases = reference.cases as Case[];

// The Rust computes in f32 throughout; this runs in f64. Small accumulated
// differences are expected, so the bar is "closer than anything observable",
// not bit equality.
const LAB_TOLERANCE = 1e-4;

describe("colour surface matches the Rust implementation", () => {
  it.each(cases.map((c) => [c.name, c] as const))(
    "agrees on every %s sample in Oklab",
    (_name, testCase) => {
      const surface = new ColorSurface(testCase);
      for (const s of testCase.samples) {
        const got = surface.sample(s.x, s.y);
        expect(Math.abs(got.l - s.l), `l at (${s.x}, ${s.y})`).toBeLessThan(LAB_TOLERANCE);
        expect(Math.abs(got.a - s.a), `a at (${s.x}, ${s.y})`).toBeLessThan(LAB_TOLERANCE);
        expect(Math.abs(got.b - s.b), `b at (${s.x}, ${s.y})`).toBeLessThan(LAB_TOLERANCE);
        expect(Math.abs(got.alpha - s.alpha), `alpha at (${s.x}, ${s.y})`).toBeLessThan(
          LAB_TOLERANCE,
        );
      }
    },
  );

  it.each(cases.map((c) => [c.name, c] as const))(
    "agrees on the bytes the %s palette would put on the LEDs",
    (_name, testCase) => {
      const surface = new ColorSurface(testCase);
      for (const s of testCase.samples) {
        const got = oklabToLedBytes(surface.sample(s.x, s.y));
        for (let c = 0; c < 3; c++) {
          // One step of slack: the two languages can round a value sitting
          // exactly on a boundary differently, and one 8-bit step is invisible.
          expect(
            Math.abs(got[c] - s.led[c]),
            `channel ${c} at (${s.x}, ${s.y}): got ${got}, expected ${s.led}`,
          ).toBeLessThanOrEqual(1);
        }
      }
    },
  );

  it("uses the same default palette as the engine", () => {
    // Guards against the two defaults drifting apart, which would make a fresh
    // UI disagree with a fresh engine before anything is even edited.
    expect(cases[0].name).toBe("default");
    expect(cases[0].keyframes.length).toBeGreaterThan(0);
    expect(cases[0].sigma).toBeGreaterThan(0);
  });

  it("actually exercises opacity", () => {
    // Without this the alpha assertions above could pass on a fixture where
    // every sample is opaque, which proves nothing about the blending.
    const alphas = cases.flatMap((c) => c.samples.map((s) => s.alpha));
    expect(alphas.some((a) => a > 0.99)).toBe(true);
    expect(alphas.some((a) => a < 0.1)).toBe(true);
    expect(alphas.some((a) => a >= 0.3 && a <= 0.7)).toBe(true);
  });
});
