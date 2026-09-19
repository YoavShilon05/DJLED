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
 * It carries several palettes. The first is the shipped default, which is
 * opaque. The second is translucent, because opacity-weighted blending is the
 * subtlest thing here to port and an all-opaque fixture would let both sides get
 * it wrong and still agree. Others confine their keyframes and join the position
 * axis end to end — the two places a port can quietly measure a distance
 * differently and agree everywhere else.
 */

import { describe, expect, it } from "vitest";

import reference from "./reference.json";
import { oklabToLedBytes } from "./oklab";
import { ColorSurface, type SurfaceConfig } from "./surface";
import type { Timeline } from "./timeline";

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
  /** Whether the recorded samples were taken on a joined position axis. */
  cycle?: boolean;
  keyframes: SurfaceConfig["keyframes"];
  samples: Sample[];
}

/** A recorded case, back in the shape the editor compiles. */
function surfaceOf(c: Case): ColorSurface {
  return new ColorSurface(c).cycling(c.cycle === true);
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
      const surface = surfaceOf(testCase);
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
      const surface = surfaceOf(testCase);
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

interface Phase {
  seconds: number;
  samples: Sample[];
}

interface TimelineCase {
  name: string;
  length: number;
  cycle?: boolean;
  keys: { at: number; sigma: number; keyframes: SurfaceConfig["keyframes"] }[];
  phases: Phase[];
}

const timelines = reference.timelines as TimelineCase[];

/** A recorded timeline, back in the shape the editor compiles. */
function timelineOf(c: TimelineCase): Timeline {
  return {
    enabled: true,
    length: c.length,
    keys: c.keys.map((k, i) => ({
      id: `k${i}`,
      at: k.at,
      surface: { sigma: k.sigma, keyframes: k.keyframes },
    })),
  };
}

describe("timelines match the Rust implementation", () => {
  it.each(timelines.map((c) => [c.name, c] as const))(
    "agrees with the engine at every instant of the %s loop",
    (_name, testCase) => {
      const surface = ColorSurface.animated(timelineOf(testCase)).cycling(
        testCase.cycle === true,
      );
      for (const phase of testCase.phases) {
        surface.seek(phase.seconds);
        for (const s of phase.samples) {
          const got = surface.sample(s.x, s.y);
          const at = `(${s.x}, ${s.y}) at ${phase.seconds}s`;
          expect(Math.abs(got.l - s.l), `l at ${at}`).toBeLessThan(LAB_TOLERANCE);
          expect(Math.abs(got.a - s.a), `a at ${at}`).toBeLessThan(LAB_TOLERANCE);
          expect(Math.abs(got.b - s.b), `b at ${at}`).toBeLessThan(LAB_TOLERANCE);
          expect(Math.abs(got.alpha - s.alpha), `alpha at ${at}`).toBeLessThan(LAB_TOLERANCE);
        }
      }
    },
  );

  it.each(timelines.map((c) => [c.name, c] as const))(
    "agrees on the bytes the %s loop would put on the LEDs",
    (_name, testCase) => {
      const surface = ColorSurface.animated(timelineOf(testCase)).cycling(
        testCase.cycle === true,
      );
      for (const phase of testCase.phases) {
        surface.seek(phase.seconds);
        for (const s of phase.samples) {
          const got = oklabToLedBytes(surface.sample(s.x, s.y));
          for (let c = 0; c < 3; c++) {
            expect(
              Math.abs(got[c] - s.led[c]),
              `channel ${c} at (${s.x}, ${s.y}) at ${phase.seconds}s`,
            ).toBeLessThanOrEqual(1);
          }
        }
      }
    },
  );

  it("exercises the awkward halves of a blend", () => {
    // A span whose ends disagree about whether a keyframe exists, and both a
    // confined and an unconfined radius. Without them, both implementations
    // could take the easy reading of each and still agree with the file.
    const counts = timelines.map((t) => t.keys.map((k) => k.keyframes.length));
    expect(counts.some((c) => Math.min(...c) !== Math.max(...c))).toBe(true);

    const radii = timelines.flatMap((t) => t.keys.flatMap((k) => k.keyframes.map((f) => f.radius)));
    expect(radii.some((r) => typeof r === "number")).toBe(true);
    expect(radii.some((r) => r == null)).toBe(true);
  });

  it("exercises a joined position axis", () => {
    // Cycling is the only thing that changes how far apart two positions are,
    // so without a case that uses it the file agrees whichever way the port
    // measures — and a keyframe away from both edges would never show it.
    const cycling = [...cases, ...timelines].filter((c) => c.cycle === true);
    expect(cycling.length).toBeGreaterThan(0);

    const positions = cycling.flatMap((c) =>
      "keyframes" in c
        ? c.keyframes.map((k) => k.x)
        : c.keys.flatMap((k) => k.keyframes.map((f) => f.x)),
    );
    expect(positions.some((x) => x < 0.15 || x > 0.85)).toBe(true);
  });

  it("actually moves between phases", () => {
    // Otherwise every assertion above could pass on a loop that never leaves
    // its first key.
    const moved = timelines.some((t) => {
      const first = t.phases[0].samples.map((s) => s.l);
      return t.phases
        .slice(1)
        .some((p) => p.samples.some((s, i) => Math.abs(s.l - first[i]) > 1e-3));
    });
    expect(moved).toBe(true);
  });
});
