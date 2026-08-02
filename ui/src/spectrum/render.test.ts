import { describe, expect, it } from "vitest";

import { sourceIndex } from "./render";

const N = 150;
const map = (mirror: boolean, reverse: boolean) =>
  Array.from({ length: N }, (_, i) => sourceIndex(i, N, mirror, reverse));

describe("strip layout", () => {
  it("is the identity when neither transform is on", () => {
    expect(map(false, false)).toEqual(Array.from({ length: N }, (_, i) => i));
  });

  it("reverses end to end", () => {
    const m = map(false, true);
    expect(m[0]).toBe(N - 1);
    expect(m[N - 1]).toBe(0);
  });

  // The two cases the feature was specified by: mirrored puts bass at both
  // ends and treble in the middle, and adding reverse swaps that round.
  it("mirrors the low end to both ends of the strip", () => {
    const m = map(true, false);
    expect(m[0]).toBe(0);
    expect(m[N - 1]).toBe(0);
    expect(m[Math.floor((N - 1) / 2)]).toBeGreaterThanOrEqual(N - 2);
  });

  it("mirrored and reversed puts the low end in the middle", () => {
    const m = map(true, true);
    expect(m[0]).toBe(N - 1);
    expect(m[N - 1]).toBe(N - 1);
    expect(m[Math.floor((N - 1) / 2)]).toBeLessThanOrEqual(1);
  });

  /**
   * An even-length strip has no LED at its centre, so the fold lands on the two
   * either side of it and the very top of the range is approached rather than
   * hit. An odd-length strip has a true centre and reaches it exactly. Pinned
   * because it is the one place mirroring is not exact, and a future change
   * that quietly loses more than an index should fail here.
   */
  it("reaches the far end of the range at the fold", () => {
    expect(Math.max(...map(true, false))).toBe(N - 2);
    const odd = Array.from({ length: 151 }, (_, i) => sourceIndex(i, 151, true, false));
    expect(Math.max(...odd)).toBe(150);
    expect(odd[75]).toBe(150);
  });

  it("mirrors symmetrically about the centre", () => {
    const m = map(true, false);
    for (let i = 0; i < N; i++) expect(m[i]).toBe(m[N - 1 - i]);
  });

  it("stays in range for every combination and length", () => {
    for (const n of [1, 2, 3, 149, 150, 600]) {
      for (const mirror of [false, true]) {
        for (const reverse of [false, true]) {
          for (let i = 0; i < n; i++) {
            const s = sourceIndex(i, n, mirror, reverse);
            expect(s).toBeGreaterThanOrEqual(0);
            expect(s).toBeLessThanOrEqual(n - 1);
          }
        }
      }
    }
  });
});
