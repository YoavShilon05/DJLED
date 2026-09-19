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
 * Keyframes carry opacity as well as colour, so the field is four channels. The
 * two are not interpolated the same way — see {@link ColorSurface.sample}.
 *
 * A keyframe may also carry a radius, past which it contributes nothing. That
 * is a different question from sigma, which is how two keyframes that both
 * reach a point share it. The consequence is that the field need not be
 * covered: where nothing reaches, the sample is transparent black.
 *
 * Must stay numerically identical to the Rust, or the editor previews something
 * the strip will not do.
 */

import { hexToOklab, type Oklab } from "./oklab";

export interface Keyframe {
  x: number;
  y: number;
  /** sRGB hex, either "#ff2000" or "#ff2000cc" with an opacity byte. */
  color: string;
  /**
   * How far this keyframe reaches, as a distance in the unit square. Absent —
   * which is how the engine writes an unconfined keyframe — means everywhere.
   */
  radius?: number | null;
}

export interface SurfaceConfig {
  keyframes: Keyframe[];
  sigma: number;
}

/**
 * Smallest area of effect that still means something, and the floor a radius is
 * clamped to. Matches `MIN_RADIUS` in the Rust.
 */
export const MIN_RADIUS = 1e-3;

/**
 * The outer fraction of a radius spent fading out. Matches `EDGE` in the Rust;
 * a disagreement here is a colour field the editor draws softer or harder than
 * the wall will.
 */
const EDGE = 0.35;

/** Presence at `t`, the distance from a keyframe as a fraction of its radius. */
function edgeTaper(t: number): number {
  const u = Math.min((1 - t) / EDGE, 1);
  return u * u * (3 - 2 * u);
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
  /** Precomputed `1 / r` for a confined keyframe, `null` for one that reaches
   *  everywhere. */
  reach: number | null;
}

export class ColorSurface {
  private points: Compiled[];
  private falloff: number;

  constructor(cfg: SurfaceConfig) {
    this.points = cfg.keyframes.map((k) => ({
      x: k.x,
      y: k.y,
      color: hexToOklab(k.color),
      // Floored rather than allowed to be zero: `1 / 0` then multiplied by a
      // zero distance is a NaN, and a NaN colour is a pixel nobody can explain.
      reach: k.radius == null ? null : 1 / Math.max(k.radius, MIN_RADIUS),
    }));
    const sigma = Math.max(cfg.sigma, 1e-3);
    this.falloff = 1 / (2 * sigma * sigma);
  }

  /**
   * Colour and opacity at a point in the unit square.
   *
   * Opacity interpolates on the plain Gaussian weights, so it is a smooth field
   * for the same reasons colour is. Colour, though, is weighted by `w · α`: a
   * fully transparent keyframe has to pull the result toward *transparent*, not
   * toward its own invisible colour. Weighting colour by `w` alone would let an
   * invisible green keyframe tint everything near it.
   *
   * Where no keyframe reaches — because there are none, or because every one of
   * them is confined elsewhere — the answer is transparent black, so a hole in
   * the field lets the layer below through rather than covering it.
   */
  sample(x: number, y: number): Oklab {
    x = x < 0 ? 0 : x > 1 ? 1 : x;
    y = y < 0 ? 0 : y > 1 ? 1 : y;

    // `weight` normalises opacity; `cover` normalises colour. `present` is how
    // much of anything reaches here at all — see below.
    let weight = 0;
    let cover = 0;
    let l = 0;
    let a = 0;
    let b = 0;
    let present = 0;
    let nearest = -1;
    let nearestD2 = Infinity;

    for (let i = 0; i < this.points.length; i++) {
      const p = this.points[i];
      const dx = x - p.x;
      const dy = y - p.y;
      const d2 = dx * dx + dy * dy;

      let taper: number;
      if (p.reach === null) {
        // Only unconfined keyframes stand in for the fallback below. A confined
        // one may not colour a point it does not reach; that is the feature.
        if (d2 < nearestD2) {
          nearestD2 = d2;
          nearest = i;
        }
        taper = 1;
      } else {
        const t = Math.sqrt(d2) * p.reach;
        if (t >= 1) continue;
        taper = edgeTaper(t);
      }

      if (taper > present) present = taper;

      const w = Math.exp(-d2 * this.falloff) * taper;
      const wa = w * p.color.alpha;
      weight += w;
      cover += wa;
      l += wa * p.color.l;
      a += wa * p.color.a;
      b += wa * p.color.b;
    }

    // With a small sigma and a point far from every keyframe, every weight can
    // underflow to zero. Falling back to the nearest keeps the field defined
    // everywhere rather than punching a black hole in it — but where there is no
    // unconfined keyframe to fall back to, the baseline is the answer.
    if (weight <= Number.MIN_VALUE) {
      return nearest >= 0 ? this.points[nearest].color : { l: 0, a: 0, b: 0, alpha: 0 };
    }

    // Normalisation is also what would silently undo the taper: with one
    // keyframe in reach it appears in both `cover` and `weight` and cancels, so
    // opacity would hold its authored value right to the edge and then fall off
    // a cliff. `present` is the taper before normalisation, and the maximum
    // rather than the sum, so two overlapping areas of effect are covered where
    // either one covers and not covered twice.
    const alpha = (present * cover) / weight;

    // Every keyframe within reach is fully transparent, so there is no colour to
    // average — only the absence of one.
    if (cover <= Number.MIN_VALUE) return { l: 0, a: 0, b: 0, alpha };

    return { l: l / cover, a: a / cover, b: b / cover, alpha };
  }
}

/** Clone a config, so edits never mutate state React is holding. */
export function cloneSurface(cfg: SurfaceConfig): SurfaceConfig {
  return { sigma: cfg.sigma, keyframes: cfg.keyframes.map((k) => ({ ...k })) };
}
