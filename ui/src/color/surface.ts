/**
 * The 2D keyframe colour surface — a port of `engine/src/color/surface.rs`.
 *
 * Colour is authored over the unit square: x is position along the strip (bass
 * left, treble right), y is that band's intensity.
 *
 * Interpolation is normalised Gaussian weighting. Of the obvious candidates it
 * is the only one that is smooth everywhere, needs no matrix solve, cannot be
 * made singular by two keyframes landing on top of each other, and is a convex
 * combination — so results stay inside the hull of the authored colours. RBF
 * overshoots and goes ill-conditioned; inverse-distance weighting leaves a flat
 * plateau at every control point.
 *
 * Must stay numerically identical to the Rust, or the editor previews something
 * the strip will not do.
 */

import { hexToOklab, type Oklab } from "./oklab";

export interface Keyframe {
  x: number;
  y: number;
  /** sRGB hex, e.g. "#ff2000". */
  color: string;
}

export interface SurfaceConfig {
  keyframes: Keyframe[];
  sigma: number;
}

/** Matches `SurfaceConfig::default()` in the Rust. */
export const DEFAULT_SURFACE: SurfaceConfig = {
  keyframes: [
    { x: 0.0, y: 0.0, color: "#1a0000" },
    { x: 0.0, y: 1.0, color: "#ff2000" },
    { x: 0.5, y: 0.0, color: "#001a08" },
    { x: 0.5, y: 1.0, color: "#40ff60" },
    { x: 1.0, y: 0.0, color: "#000820" },
    { x: 1.0, y: 1.0, color: "#40c0ff" },
  ],
  sigma: 0.25,
};

interface Compiled {
  x: number;
  y: number;
  color: Oklab;
}

export class ColorSurface {
  private points: Compiled[];
  private falloff: number;

  constructor(cfg: SurfaceConfig) {
    this.points = cfg.keyframes.map((k) => ({
      x: k.x,
      y: k.y,
      color: hexToOklab(k.color),
    }));
    const sigma = Math.max(cfg.sigma, 1e-3);
    this.falloff = 1 / (2 * sigma * sigma);
  }

  sample(x: number, y: number): Oklab {
    if (this.points.length === 0) return { l: 0, a: 0, b: 0 };

    x = x < 0 ? 0 : x > 1 ? 1 : x;
    y = y < 0 ? 0 : y > 1 ? 1 : y;

    let total = 0;
    let l = 0;
    let a = 0;
    let b = 0;
    let nearest = 0;
    let nearestD2 = Infinity;

    for (let i = 0; i < this.points.length; i++) {
      const p = this.points[i];
      const dx = x - p.x;
      const dy = y - p.y;
      const d2 = dx * dx + dy * dy;
      if (d2 < nearestD2) {
        nearestD2 = d2;
        nearest = i;
      }

      const w = Math.exp(-d2 * this.falloff);
      total += w;
      l += w * p.color.l;
      a += w * p.color.a;
      b += w * p.color.b;
    }

    // With a small sigma and a point far from every keyframe, every weight can
    // underflow to zero. Falling back to the nearest keeps the field defined
    // everywhere rather than punching a black hole in it.
    if (total <= Number.MIN_VALUE) return this.points[nearest].color;

    return { l: l / total, a: a / total, b: b / total };
  }
}

/** Clone a config, so edits never mutate state React is holding. */
export function cloneSurface(cfg: SurfaceConfig): SurfaceConfig {
  return { sigma: cfg.sigma, keyframes: cfg.keyframes.map((k) => ({ ...k })) };
}
