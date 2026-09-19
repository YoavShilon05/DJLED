/**
 * A layer's colour field as a function of time — a port of
 * `engine/src/color/timeline.rs`.
 *
 * The surface answers "what colour is this position, at this level". A timeline
 * adds "*when*", by authoring several whole fields and interpolating between
 * them. Each is a {@link TimelineKey}: a moment in the loop, and the field the
 * layer has reached by then. Positions, colours, opacities, areas of effect and
 * the blend radius all travel between one key and the next.
 *
 * Whole fields rather than a track per property because the thing being
 * authored is a *graph*: the editor draws one field at a time and every gesture
 * on it edits that field as a whole, so a key that **is** the graph at a moment
 * needs no second editor and no second mental model.
 *
 * Identity between two keys is position in the list. A
 * {@link import("./surface").Keyframe} carries no id — the wire format is
 * deliberately just a point and a colour, so a preset stays readable — and the
 * editor adds and removes a colour on every key at once precisely so the nth
 * keyframe means the same thing in all of them.
 *
 * # The clock
 *
 * The phase is a function of the wall clock, seconds since the Unix epoch,
 * modulo the length. So this file and the engine agree on which instant of the
 * loop is live without it ever crossing the wire — they read the same machine's
 * clock — and neither a reconnect, a preset switch nor a restart lurches a show
 * that is already up.
 *
 * Must stay numerically identical to the Rust; `reference.json` carries
 * timeline cases and both sides assert against them.
 */

import { DEFAULT_SURFACE, type SurfaceConfig } from "./surface";

/** What a layer's field has become by one moment of the loop. */
export interface TimelineKey {
  /** The editor's handle on this key, never interpreted by the engine. */
  id: string;
  /**
   * Where in the loop this field is reached, as a fraction of the length.
   *
   * Normalised rather than in seconds, so changing the length stretches the
   * whole animation instead of clipping the end off it — which is what someone
   * matching a loop to a tempo means by making it shorter.
   */
  at: number;
  surface: SurfaceConfig;
}

/**
 * A layer's timeline. No keys means a layer that does not animate, which is
 * what every show written before this existed is.
 */
export interface Timeline {
  /** Off holds the field at the first key rather than dropping the keys. */
  enabled: boolean;
  /** Loop length in seconds. Per layer, and independent of every other. */
  length: number;
  keys: TimelineKey[];
}

/** Matches `MIN_LENGTH` / `MAX_LENGTH` in the Rust. */
export const MIN_LENGTH = 0.05;
export const MAX_LENGTH = 600;

/** Matches `Timeline::default()`. Eight seconds, and nothing animating. */
export const DEFAULT_LENGTH = 8;

export function emptyTimeline(): Timeline {
  return { enabled: true, length: DEFAULT_LENGTH, keys: [] };
}

/** Loop length in seconds, made safe to divide by. */
export function timelineLength(tl: Timeline): number {
  if (!Number.isFinite(tl.length)) return DEFAULT_LENGTH;
  return Math.min(MAX_LENGTH, Math.max(MIN_LENGTH, tl.length));
}

/** Whether this timeline actually moves. One key is a still field, and so is a
 *  timeline switched off. */
export function animates(tl: Timeline): boolean {
  return tl.enabled && tl.keys.length >= 2;
}

/**
 * The key the loop starts from — the earliest, not merely the first in the
 * list. Also the field shown when the timeline is switched off, and the one a
 * layer's stored `surface` mirrors for peers that do not know about timelines.
 */
export function startOf(tl: Timeline): SurfaceConfig | null {
  let best: TimelineKey | null = null;
  for (const key of tl.keys) {
    if (!Number.isFinite(key.at)) continue;
    if (best === null || key.at < best.at) best = key;
  }
  return best === null ? null : best.surface;
}

/** Where in the loop a moment in time falls, 0..1. */
export function phaseOf(tl: Timeline, seconds: number): number {
  const length = timelineLength(tl);
  const phase = (seconds % length) / length;
  return phase < 0 ? phase + 1 : phase;
}

/**
 * Seconds since the Unix epoch — the clock every layer's loop is a function of,
 * and the same one `epoch_seconds` reads in the engine.
 */
export function nowSeconds(): number {
  return Date.now() / 1000;
}

/**
 * The same timeline with every key's field projected onto the top row.
 *
 * The whole-timeline form of `flattened` in `surface.rs`: a layer listening to
 * nothing is sampled at a flat full level, so only one row of its field is ever
 * read. A still colour that animates is exactly the combination that needs it.
 */
export function flattenTimeline(tl: Timeline): Timeline {
  return {
    enabled: tl.enabled,
    length: tl.length,
    keys: tl.keys.map((k) => ({
      ...k,
      surface: { sigma: k.surface.sigma, keyframes: k.surface.keyframes.map((f) => ({ ...f, y: 1 })) },
    })),
  };
}

/** A key holding the stock palette, for a timeline being started from nothing. */
export function defaultKey(id: string, at: number): TimelineKey {
  return { id, at, surface: { sigma: DEFAULT_SURFACE.sigma, keyframes: DEFAULT_SURFACE.keyframes.map((k) => ({ ...k })) } };
}
