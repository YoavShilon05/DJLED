/**
 * The 1D ramp a static layer paints.
 *
 * Deliberately not the same machinery as the colour surface. The surface is 2D
 * and its whole job is to be smooth in every direction, which is why it uses
 * Gaussian weighting and why a keyframe influences the field well away from
 * itself. A ramp is 1D and wants the opposite: a stop is a colour *at* a
 * position, and two stops close together should give a hard edge rather than
 * averaging into a wash. So this is piecewise linear, and interpolation happens
 * in Oklab for the same reason it does everywhere else.
 */

import { hexToOklab, type Oklab } from "./oklab";
import type { GradientStop } from "../config/layers";

interface Compiled {
  pos: number;
  color: Oklab;
}

const BLACK: Oklab = { l: 0, a: 0, b: 0 };

export class Gradient {
  private stops: Compiled[];

  constructor(stops: GradientStop[]) {
    this.stops = [...stops]
      .sort((a, b) => a.pos - b.pos)
      .map((s) => ({ pos: s.pos, color: hexToOklab(s.color) }));
  }

  /**
   * Colour at a position, clamped at both ends.
   *
   * Holding the end colours rather than fading to black past the last stop: the
   * layer's span already says where it stops painting, and a ramp that dimmed
   * towards its own edges would make that span impossible to place accurately.
   */
  sample(p: number): Oklab {
    const n = this.stops.length;
    if (n === 0) return BLACK;
    if (n === 1) return this.stops[0].color;

    if (p <= this.stops[0].pos) return this.stops[0].color;
    if (p >= this.stops[n - 1].pos) return this.stops[n - 1].color;

    for (let i = 1; i < n; i++) {
      const b = this.stops[i];
      if (p > b.pos) continue;
      const a = this.stops[i - 1];
      const span = b.pos - a.pos;
      // Coincident stops are a hard edge, not a division by zero.
      const t = span <= 0 ? 1 : (p - a.pos) / span;
      return {
        l: a.color.l + (b.color.l - a.color.l) * t,
        a: a.color.a + (b.color.a - a.color.a) * t,
        b: a.color.b + (b.color.b - a.color.b) * t,
      };
    }
    return this.stops[n - 1].color;
  }
}
