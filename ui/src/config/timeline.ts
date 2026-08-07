/**
 * Snapshot animation.
 *
 * A key is a whole copy of a layer's state at a moment on its loop, and the
 * state actually in force is the tween between the two keys surrounding the
 * playhead. There is no per-parameter track and nothing to arm: scrub, edit,
 * and the edit lands in a key at that time.
 *
 * Two consequences worth knowing:
 *
 *  - **The loop is a ring, not a line.** The segment after the last key wraps to
 *    the first, so an animation returns to where it started without the author
 *    duplicating a key at the end.
 *  - **Items are matched by id, and an unmatched one holds its own value.** A
 *    colour keyframe that exists in only one of the two surrounding keys stays
 *    put for that whole segment rather than fading in from nowhere or popping
 *    mid-way. Ids were already stable editor bookkeeping; this is what they buy.
 */

import { hexToOklab, oklabToHex } from "../color/oklab";
import type { CurveConfig, Point } from "./curve";
import type { EqBand } from "./eq";
import type {
  ColorKeyframe,
  GradientStop,
  Layer,
  LayerKey,
  LayerState,
  LedKeyframe,
  SpectrumState,
  StaticState,
} from "./layers";
import { nextId } from "./layers";
import { clamp } from "./scales";

/**
 * How close a click has to land to count as the same key. Also the auto-key
 * tolerance — an edit inside this window rewrites the key rather than laying
 * down a second one a few milliseconds away.
 */
export const KEY_SNAP_SECONDS = 0.05;

const lerp = (x: number, y: number, t: number) => x + (y - x) * t;

/** Positive modulo — `-0.5 % 8` is `-0.5` in JS, which is not a phase. */
export function phaseOf(time: number, loopSeconds: number): number {
  if (!(loopSeconds > 0)) return 0;
  return ((time % loopSeconds) + loopSeconds) % loopSeconds;
}

interface Segment<S extends LayerState> {
  a: LayerKey<S>;
  b: LayerKey<S>;
  /** 0..1 between the two. */
  t: number;
}

/** The two keys either side of a phase, wrapping across the loop boundary. */
export function segmentAt<S extends LayerState>(
  keys: Array<LayerKey<S>>,
  phase: number,
  loopSeconds: number,
): Segment<S> | null {
  const sorted = sortKeys(keys);
  const n = sorted.length;
  if (n === 0) return null;
  if (n === 1) return { a: sorted[0], b: sorted[0], t: 0 };

  let i = -1;
  for (let k = 0; k < n; k++) if (sorted[k].t <= phase) i = k;

  // Before the first key: the tail of the segment that wraps around the end.
  if (i === -1) {
    const a = sorted[n - 1];
    const b = sorted[0];
    const span = loopSeconds - a.t + b.t;
    return { a, b, t: span > 0 ? clamp((loopSeconds - a.t + phase) / span, 0, 1) : 0 };
  }

  const a = sorted[i];
  const b = sorted[(i + 1) % n];
  const span = i === n - 1 ? loopSeconds - a.t + b.t : b.t - a.t;
  return { a, b, t: span > 0 ? clamp((phase - a.t) / span, 0, 1) : 0 };
}

/** Generic over the element rather than the state, so a mixed array still sorts. */
export function sortKeys<T extends { t: number }>(keys: T[]): T[] {
  return [...keys].sort((x, y) => x.t - y.t);
}

/**
 * The state a layer is actually in at a moment on the shared clock.
 *
 * With no keys this is just the layer's own state, which is what keeps an
 * un-animated layer free of any timeline machinery.
 */
export function stateAt(layer: Layer, time: number): LayerState {
  if (layer.keys.length === 0) return layer.state;
  const phase = phaseOf(time, layer.loopSeconds);

  if (layer.kind === "spectrum") {
    const seg = segmentAt(layer.keys, phase, layer.loopSeconds);
    return seg ? lerpSpectrum(seg.a.state, seg.b.state, seg.t) : layer.state;
  }
  const seg = segmentAt(layer.keys, phase, layer.loopSeconds);
  return seg ? lerpStatic(seg.a.state, seg.b.state, seg.t) : layer.state;
}

/** Narrowing helpers, so callers do not repeat the union check at every use. */
export function spectrumStateAt(layer: Layer, time: number): SpectrumState | null {
  return layer.kind === "spectrum" ? (stateAt(layer, time) as SpectrumState) : null;
}

export function staticStateAt(layer: Layer, time: number): StaticState | null {
  return layer.kind === "static" ? (stateAt(layer, time) as StaticState) : null;
}

/**
 * Apply an edit made while the playhead was at `time`.
 *
 * With no keys the edit is just the layer's state. With keys it auto-keys: the
 * key under the playhead is rewritten, or one is created there. `state` is kept
 * in step either way, so deleting every key leaves the layer looking exactly as
 * it did rather than reverting to whatever it held before it was animated.
 */
export function writeStateAt(layer: Layer, time: number, next: LayerState): Layer {
  if (layer.keys.length === 0) {
    return { ...layer, state: next } as Layer;
  }
  const phase = phaseOf(time, layer.loopSeconds);
  const hit = layer.keys.find((k) => Math.abs(k.t - phase) <= KEY_SNAP_SECONDS);
  const keys = hit
    ? layer.keys.map((k) => (k.id === hit.id ? { ...k, state: next } : k))
    : sortKeys([...layer.keys, { id: nextId("key"), t: phase, state: next }] as Array<
        LayerKey<LayerState>
      >);
  return { ...layer, state: next, keys } as Layer;
}

/** Capture the current state as a key at `time`, replacing one already there. */
export function addKeyAt(layer: Layer, time: number): Layer {
  const phase = phaseOf(time, layer.loopSeconds);
  const current = stateAt(layer, time);
  const hit = layer.keys.find((k) => Math.abs(k.t - phase) <= KEY_SNAP_SECONDS);
  if (hit) {
    return {
      ...layer,
      keys: layer.keys.map((k) => (k.id === hit.id ? { ...k, state: current } : k)),
    } as Layer;
  }
  return {
    ...layer,
    keys: sortKeys([...layer.keys, { id: nextId("key"), t: phase, state: current }] as Array<
      LayerKey<LayerState>
    >),
  } as Layer;
}

export function removeKey(layer: Layer, keyId: string): Layer {
  return { ...layer, keys: layer.keys.filter((k) => k.id !== keyId) } as Layer;
}

export function moveKey(layer: Layer, keyId: string, t: number): Layer {
  const at = clamp(t, 0, layer.loopSeconds);
  return {
    ...layer,
    keys: sortKeys(
      layer.keys.map((k) => (k.id === keyId ? { ...k, t: at } : k)),
    ) as Array<LayerKey<LayerState>>,
  } as Layer;
}

/**
 * Widened to the base state rather than generic: callers hold a `Layer`, whose
 * `keys` is a union of two array types, and only the key's `t` matters here.
 */
export function keyNear(
  keys: Array<LayerKey<LayerState>>,
  phase: number,
): LayerKey<LayerState> | null {
  return keys.find((k) => Math.abs(k.t - phase) <= KEY_SNAP_SECONDS) ?? null;
}

/* ------------------------------------------------------------------ tweening */

/**
 * Interpolate a colour through Oklab.
 *
 * The same reason the surface does: sRGB drags a gradient through mud and HSV
 * bands the hue. An animated colour crosses far more of the space than a static
 * one, so this matters more here, not less.
 */
function lerpHex(from: string, to: string, t: number): string {
  if (from === to) return from;
  const c0 = hexToOklab(from);
  const c1 = hexToOklab(to);
  return oklabToHex({
    l: lerp(c0.l, c1.l, t),
    a: lerp(c0.a, c1.a, t),
    b: lerp(c0.b, c1.b, t),
  });
}

/** Match by id, lerp what pairs up, and let anything unpaired hold its value. */
function mergeById<T extends { id: string }>(
  from: T[],
  to: T[],
  t: number,
  lerpOne: (x: T, y: T, t: number) => T,
): T[] {
  const byId = new Map(to.map((i) => [i.id, i]));
  const seen = new Set(from.map((i) => i.id));
  const out = from.map((x) => {
    const y = byId.get(x.id);
    return y ? lerpOne(x, y, t) : x;
  });
  for (const y of to) if (!seen.has(y.id)) out.push(y);
  return out;
}

function lerpPoint(x: Point, y: Point, t: number): Point {
  return { x: lerp(x.x, y.x, t), y: lerp(x.y, y.y, t) };
}

/**
 * A curve's *type* is an enum, so it steps at the key rather than tweening —
 * there is no halfway between "ease in" and "bézier". Its control points do
 * tween, which is what makes a hand-authored bézier animate.
 */
function lerpCurve(from: CurveConfig, to: CurveConfig, t: number): CurveConfig {
  return {
    type: t >= 1 ? to.type : from.type,
    p1: lerpPoint(from.p1, to.p1, t),
    p2: lerpPoint(from.p2, to.p2, t),
  };
}

function lerpColorKeyframe(x: ColorKeyframe, y: ColorKeyframe, t: number): ColorKeyframe {
  return {
    ...x,
    hz: lerp(x.hz, y.hz, t),
    db: lerp(x.db, y.db, t),
    color: lerpHex(x.color, y.color, t),
  };
}

function lerpLedKeyframe(x: LedKeyframe, y: LedKeyframe, t: number): LedKeyframe {
  return { ...x, hz: lerp(x.hz, y.hz, t), led: Math.round(lerp(x.led, y.led, t)) };
}

function lerpEqBand(x: EqBand, y: EqBand, t: number): EqBand {
  return {
    ...x,
    // Same reasoning as the curve type: a filter is one shape or another.
    type: t >= 1 ? y.type : x.type,
    hz: lerp(x.hz, y.hz, t),
    gain: lerp(x.gain, y.gain, t),
    q: lerp(x.q, y.q, t),
  };
}

function lerpStop(x: GradientStop, y: GradientStop, t: number): GradientStop {
  return { ...x, pos: lerp(x.pos, y.pos, t), color: lerpHex(x.color, y.color, t) };
}

export function lerpSpectrum(from: SpectrumState, to: SpectrumState, t: number): SpectrumState {
  return {
    source: t >= 1 ? to.source : from.source,
    colorKeyframes: mergeById(from.colorKeyframes, to.colorKeyframes, t, lerpColorKeyframe),
    ledKeyframes: mergeById(from.ledKeyframes, to.ledKeyframes, t, lerpLedKeyframe),
    eq: mergeById(from.eq, to.eq, t, lerpEqBand),
    threshold: lerp(from.threshold, to.threshold, t),
    clamp: lerp(from.clamp, to.clamp, t),
    curve: lerpCurve(from.curve, to.curve, t),
    blendRadius: lerp(from.blendRadius, to.blendRadius, t),
  };
}

export function lerpStatic(from: StaticState, to: StaticState, t: number): StaticState {
  return {
    stops: mergeById(from.stops, to.stops, t, lerpStop),
    from: Math.round(lerp(from.from, to.from, t)),
    to: Math.round(lerp(from.to, to.to, t)),
    level: lerp(from.level, to.level, t),
  };
}
