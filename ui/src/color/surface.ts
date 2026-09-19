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
import { animates, startOf, timelineLength, type Timeline } from "./timeline";

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
 * Largest area of effect worth distinguishing from no limit at all. Matches
 * `UNCONFINED` in the Rust, and is the same number the editor's radius slider
 * tops out at.
 *
 * The diagonal of the unit square is √2, so a keyframe reaching this far covers
 * the field from any corner. It exists because a radius is nullable and an
 * interpolation cannot be halfway to `null`: across a span where one end is
 * confined and the other is not, "everywhere" stands in as this distance.
 */
export const UNCONFINED = 1.45;

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

/**
 * One authored keyframe with its colour parsed, before a radius has become the
 * reciprocal {@link Compiled} samples with.
 *
 * Separate because interpolation happens in authored units: lerping `1 / r`
 * would make an area of effect open fast and close slowly for no reason anyone
 * asked for.
 */
interface Authored {
  x: number;
  y: number;
  color: Oklab;
  radius: number | null;
}

/** The whole field at one authored moment of a loop. */
interface Moment {
  /** Where in the loop, 0..1. */
  at: number;
  /** The blend radius here, interpolated as sigma rather than as the falloff it
   *  becomes — for the same reason a radius is. */
  sigma: number;
  /** Index-matched across every moment, padded to the longest. `null` where
   *  this keyframe is not authored here — see {@link ColorSurface.seek}. */
  points: (Authored | null)[];
}

function compile(p: Authored): Compiled {
  return {
    x: p.x,
    y: p.y,
    color: p.color,
    // Floored rather than allowed to be zero: `1 / 0` then multiplied by a zero
    // distance is a NaN, and a NaN colour is a pixel nobody can explain.
    reach: p.radius == null ? null : 1 / Math.max(p.radius, MIN_RADIUS),
  };
}

function authored(cfg: SurfaceConfig): Authored[] {
  return cfg.keyframes.map((k) => ({
    x: k.x,
    y: k.y,
    color: hexToOklab(k.color),
    radius: k.radius ?? null,
  }));
}

function falloffOf(sigma: number): number {
  const s = Math.max(sigma, 1e-3);
  return 1 / (2 * s * s);
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

/** A difference of two phases, measured forwards around the loop. */
function wrapped(delta: number): number {
  return delta >= 0 ? delta : delta + 1;
}

/** The same keyframe with its opacity scaled — how one authored at one end of a
 *  span and absent from the other crosses it. */
function faded(p: Authored, by: number): Authored {
  return { ...p, color: { ...p.color, alpha: p.color.alpha * by } };
}

/**
 * One keyframe part of the way from where it is authored at one key to where it
 * is authored at the next.
 *
 * Colour interpolates in Oklab, which is the whole reason colours live in Oklab
 * here: red crossing to blue passes through the purples rather than through the
 * muddy grey a linear-RGB lerp gives.
 *
 * The radius is the one value with no obvious midpoint, because "everywhere" is
 * not a distance. {@link UNCONFINED} stands in for it, so an area of effect
 * opening up grows smoothly to the size of the field and only then stops being
 * a limit at all.
 */
function mix(p: Authored, q: Authored, t: number): Authored {
  let radius: number | null = null;
  if (p.radius !== null || q.radius !== null) {
    const r = lerp(p.radius ?? UNCONFINED, q.radius ?? UNCONFINED, t);
    radius = r >= UNCONFINED ? null : r;
  }
  return {
    x: lerp(p.x, q.x, t),
    y: lerp(p.y, q.y, t),
    color: {
      l: lerp(p.color.l, q.color.l, t),
      a: lerp(p.color.a, q.color.a, t),
      b: lerp(p.color.b, q.color.b, t),
      alpha: lerp(p.color.alpha, q.color.alpha, t),
    },
    radius,
  };
}

export class ColorSurface {
  /** The field as it stands right now — the whole of it for a still surface. */
  private points: Compiled[];
  private falloff: number;
  /** Empty unless this surface animates. Sorted by {@link Moment.at}. */
  private moments: Moment[] = [];
  /** Loop length in seconds. Only read when `moments` has something in it. */
  private length = 0;

  constructor(cfg: SurfaceConfig) {
    this.points = authored(cfg).map(compile);
    this.falloff = falloffOf(cfg.sigma);
  }

  /**
   * The field a layer paints with: its timeline where it has one, and its still
   * surface where it does not.
   *
   * Not two alternatives the caller picks between — the keys are authoritative
   * wherever there are any, and a layer's stored surface is only the view of
   * them a peer that does not know about timelines gets. Preferring it would
   * leave the editor and the engine arguing about which field is live.
   */
  static forLayer(surface: SurfaceConfig, timeline: Timeline): ColorSurface {
    const start = startOf(timeline);
    if (start === null) return new ColorSurface(surface);
    // A timeline switched off, or one with a single key, is a still field — the
    // one the loop would have started from.
    if (!animates(timeline)) return new ColorSurface(start);
    return ColorSurface.animated(timeline);
  }

  /**
   * Compile a timeline for sampling.
   *
   * Every key is padded to the longest one's keyframe count, so the blend in
   * {@link ColorSurface.seek} is an index walk with no bookkeeping. The keys are
   * sorted here rather than trusted: a hand-edited preset is under no obligation
   * to be in order, and an out-of-order key would play the loop backwards.
   */
  static animated(timeline: Timeline): ColorSurface {
    const keys = [...timeline.keys].sort((a, b) => a.at - b.at);
    const width = keys.reduce((n, k) => Math.max(n, k.surface.keyframes.length), 0);

    const surface = new ColorSurface(keys[0].surface);
    surface.moments = keys.map((k) => {
      const points: (Authored | null)[] = authored(k.surface);
      while (points.length < width) points.push(null);
      return {
        at: Number.isFinite(k.at) ? Math.min(1, Math.max(0, k.at)) : 0,
        sigma: k.surface.sigma,
        points,
      };
    });
    surface.length = timelineLength(timeline);
    return surface;
  }

  /** Whether this surface moves on its own. A still one ignores
   *  {@link ColorSurface.seek} entirely. */
  animates(): boolean {
    return this.moments.length >= 2;
  }

  /**
   * Move the field to where the loop has reached at `seconds` — seconds since
   * the Unix epoch, which is what makes this and the engine agree on the
   * instant without it crossing the wire.
   *
   * Everything authored interpolates: position, colour, opacity, area of effect
   * and the blend radius. The one case that is not a plain lerp is a keyframe
   * authored at one end of the span and not the other, which fades its opacity
   * out across the span. The editor never produces that — it adds and removes a
   * colour on every key at once — but a hand-edited preset can, and a pop on
   * the wall is a worse answer than a fade.
   */
  seek(seconds: number): void {
    if (!this.animates()) return;

    const n = this.moments.length;
    const length = Math.max(this.length, 1e-3);
    let phase = (seconds % length) / length;
    if (phase < 0) phase += 1;

    // The bracketing pair, wrapping past the last key back to the first — which
    // is the whole of what makes this a loop rather than a ramp. A phase before
    // the first key is still inside that wrap.
    let a: number;
    let b: number;
    if (phase < this.moments[0].at) {
      a = n - 1;
      b = 0;
    } else {
      a = 0;
      while (a + 1 < n && this.moments[a + 1].at <= phase) a += 1;
      b = (a + 1) % n;
    }

    // Both spans are measured around the loop, so the one that crosses the wrap
    // is positive rather than a negative span playing its segment backwards.
    const span = wrapped(this.moments[b].at - this.moments[a].at);
    const travelled = wrapped(phase - this.moments[a].at);
    const t = span > 1e-6 ? Math.min(1, Math.max(0, travelled / span)) : 0;

    const from = this.moments[a];
    const to = this.moments[b];
    this.falloff = falloffOf(lerp(from.sigma, to.sigma, t));

    this.points.length = 0;
    for (let i = 0; i < from.points.length; i++) {
      const start = from.points[i];
      const end = to.points[i];
      if (start !== null && end !== null) this.points.push(compile(mix(start, end, t)));
      else if (start !== null) this.points.push(compile(faded(start, 1 - t)));
      else if (end !== null) this.points.push(compile(faded(end, t)));
    }
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
