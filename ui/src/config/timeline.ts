/**
 * The timeline, as the editor authors it.
 *
 * A layer's colour field can be a *loop* of fields rather than one: each
 * {@link EditorKey} is a moment, and the graph between two of them is
 * interpolated — positions, colours, opacities and areas of effect all travel.
 * The engine runs the same loop off the same wall clock, so what the plot shows
 * and what the wall does are the same instant.
 *
 * # Where the first key lives
 *
 * The key at the start of the loop **is** the layer's own
 * {@link EditorLayer.colorKeyframes} and {@link EditorLayer.blend}, and
 * {@link EditorTimeline.keys} holds only the ones after it. Three things fall
 * out of that and all three are the reason:
 *
 * - A layer with no keys is byte-for-byte the still layer it always was. There
 *   is no migration, no second code path, and no timeline object to author
 *   before a colour can be placed.
 * - The wire's `surface` — which a peer that knows nothing about timelines
 *   reads — is a *derivation* of the first key rather than a second copy of it
 *   that could go stale. See `toEngineConfig`.
 * - A loop always has a field at phase 0. The first key cannot be deleted or
 *   dragged off the start, which is exactly the guarantee an animation with a
 *   wrap-around needs.
 *
 * # The set of colours belongs to the layer; where they sit belongs to the key
 *
 * Adding or deleting a colour keyframe applies to *every* key at once, keeping
 * the nth keyframe the same thing in all of them — which is the correspondence
 * the engine's blend is built on, since a keyframe carries no id over the wire.
 * Moving one, recolouring it or changing its area of effect touches only the
 * key being edited. {@link mergeKeyEdit} is where that split is enforced, and
 * it is the only place the editor decides which of the two an edit was.
 */

import { hexToOklab, oklabToHex } from "../color/oklab";
import { UNCONFINED } from "../color/surface";
import {
  DEFAULT_LENGTH,
  MAX_LENGTH,
  MIN_LENGTH,
  type Timeline,
  type TimelineKey,
} from "../color/timeline";
// Types only. The editor owns the layer and this owns the keys inside it, so a
// value import either way would be a cycle between the two halves of one model.
import type { ColorKeyframe, EditorLayer } from "./editor";
import { DB_MAX, DB_MIN, clamp, dbToNorm, hzToNorm, normToDb, normToHz } from "./scales";

/** One authored moment after the start of the loop. */
export interface EditorKey {
  id: string;
  /** Where in the loop, as a fraction of the length. Never 0 — that is the
   *  layer's own field. */
  at: number;
  colorKeyframes: ColorKeyframe[];
  blend: number;
}

export interface EditorTimeline {
  /** Off holds the field at the start of the loop rather than dropping the
   *  keys, so an animation can be auditioned against its own first frame. */
  enabled: boolean;
  /** Loop length in seconds. Per layer and independent of every other. */
  length: number;
  /** After the first, which is the layer's own field. Kept sorted by `at`. */
  keys: EditorKey[];
}

/** The id that names the key at the start of the loop — the layer's own field,
 *  which has no {@link EditorKey} of its own to carry one. */
export const BASE_KEY = "";

/** Keys sit strictly inside the loop: 0 belongs to the layer's own field, and
 *  1 is the same instant as 0 once it wraps. */
export const KEY_MIN = 0.01;
export const KEY_MAX = 0.99;

export { MAX_LENGTH, MIN_LENGTH };

export function emptyEditorTimeline(): EditorTimeline {
  return { enabled: true, length: DEFAULT_LENGTH, keys: [] };
}

/** A key's position in the loop, made safe to blend across. */
export function clampKey(at: number): number {
  if (!Number.isFinite(at)) return KEY_MIN;
  return clamp(at, KEY_MIN, KEY_MAX);
}

/** One key as everything that draws or edits it sees it, first key included. */
export interface KeyView {
  id: string;
  at: number;
  colorKeyframes: ColorKeyframe[];
  blend: number;
  /** The key at the start of the loop, which is the layer's own field. */
  base: boolean;
}

/** Every key of a layer, in loop order, starting with the layer's own field. */
export function keysOf(layer: EditorLayer): KeyView[] {
  const base: KeyView = {
    id: BASE_KEY,
    at: 0,
    colorKeyframes: layer.colorKeyframes,
    blend: layer.blend,
    base: true,
  };
  const rest = [...layer.timeline.keys]
    .sort((a, b) => a.at - b.at)
    .map((k) => ({ ...k, base: false }));
  return [base, ...rest];
}

/** Whether this layer's field moves. One key is a still field, and so is a
 *  timeline switched off. */
export function isAnimated(layer: EditorLayer): boolean {
  return layer.timeline.enabled && layer.timeline.keys.length > 0;
}

/** The key edits land on. Never null: an id naming nothing falls back to the
 *  start of the loop, the same way a missing layer id falls back to the top. */
export function keyOf(layer: EditorLayer, id: string): KeyView {
  const keys = keysOf(layer);
  return keys.find((k) => k.id === id) ?? keys[0];
}

/**
 * The layer as the graph should draw it: every property of the layer, with the
 * colour field of one key.
 *
 * A whole layer rather than a pair of fields so that the plot, the gizmos, the
 * canvas and the popovers keep reading `layer.colorKeyframes` and know nothing
 * about keys. Selecting a key changes which field they are handed, and nothing
 * else about them.
 */
export function fieldLayer(layer: EditorLayer, keyId: string): EditorLayer {
  const key = keyOf(layer, keyId);
  if (key.base) return layer;
  return { ...layer, colorKeyframes: key.colorKeyframes, blend: key.blend };
}

/**
 * Fold an edit made against {@link fieldLayer} back into the real layer.
 *
 * The colour field belongs to the key that was being edited; everything else
 * belongs to the layer. The exception is the *set* of colours: a keyframe added
 * or removed is added to or removed from every key, because the correspondence
 * between two keys is position in the list and the engine's blend is built on
 * it. An added one lands at the same place in every key until it is moved in
 * one of them, which is also what makes adding a colour mid-loop look like
 * adding a colour rather than like a colour that blinks.
 */
export function mergeKeyEdit(
  layer: EditorLayer,
  keyId: string,
  edited: EditorLayer,
): EditorLayer {
  const key = keyOf(layer, keyId);
  const before = key.colorKeyframes;
  const after = edited.colorKeyframes;

  const kept = new Set(after.map((k) => k.id));
  const had = new Set(before.map((k) => k.id));
  const removed = before.filter((k) => !kept.has(k.id)).map((k) => k.id);
  const added = after.filter((k) => !had.has(k.id));

  // Applied to the edited key too, so a set change and a field change arriving
  // in the same edit both land — the popover's delete button is exactly that.
  const propagate = (frames: ColorKeyframe[]): ColorKeyframe[] => {
    if (removed.length === 0 && added.length === 0) return frames;
    const survivors = frames.filter((k) => !removed.includes(k.id));
    const present = new Set(survivors.map((k) => k.id));
    return [...survivors, ...added.filter((k) => !present.has(k.id)).map((k) => ({ ...k }))];
  };

  const keys = layer.timeline.keys.map((k) =>
    k.id === keyId
      ? { ...k, colorKeyframes: after, blend: edited.blend }
      : { ...k, colorKeyframes: propagate(k.colorKeyframes) },
  );

  return {
    ...edited,
    // The layer's own field is the first key, so an edit to any other key must
    // not carry the edited field back onto it — only the set change does.
    colorKeyframes: key.base ? after : propagate(layer.colorKeyframes),
    blend: key.base ? edited.blend : layer.blend,
    timeline: { ...layer.timeline, keys },
  };
}

/**
 * Add a key at `at`, holding the field the loop already shows there.
 *
 * Seeded from the interpolated field rather than from a copy of the nearest
 * key, so dropping a key changes nothing about what the loop currently does —
 * it only gives the next edit somewhere to land. That is the behaviour every
 * animation tool has and the one that makes a timeline safe to subdivide
 * halfway through a set.
 */
export function addKey(layer: EditorLayer, at: number, id: string): EditorLayer {
  const phase = clamp(at, KEY_MIN, KEY_MAX);
  const field = fieldAt(layer, phase);
  const keys = [...layer.timeline.keys, { id, at: phase, ...field }].sort((a, b) => a.at - b.at);
  return { ...layer, timeline: { ...layer.timeline, keys } };
}

/** Drop a key. The one at the start of the loop is the layer's own field and
 *  cannot be dropped; removing the last of the others stops the animation. */
export function removeKey(layer: EditorLayer, id: string): EditorLayer {
  if (id === BASE_KEY) return layer;
  return {
    ...layer,
    timeline: { ...layer.timeline, keys: layer.timeline.keys.filter((k) => k.id !== id) },
  };
}

/** Move a key along the loop. */
export function moveKey(layer: EditorLayer, id: string, at: number): EditorLayer {
  if (id === BASE_KEY) return layer;
  const keys = layer.timeline.keys
    .map((k) => (k.id === id ? { ...k, at: clamp(at, KEY_MIN, KEY_MAX) } : k))
    .sort((a, b) => a.at - b.at);
  return { ...layer, timeline: { ...layer.timeline, keys } };
}

export function withTimeline(layer: EditorLayer, patch: Partial<EditorTimeline>): EditorLayer {
  return { ...layer, timeline: { ...layer.timeline, ...patch } };
}

/** Loop length in seconds, made safe to divide by. Matches the engine's clamp. */
export function safeLength(length: number): number {
  if (!Number.isFinite(length)) return DEFAULT_LENGTH;
  return Math.min(MAX_LENGTH, Math.max(MIN_LENGTH, length));
}

/** Where in the loop a moment in time falls, 0..1. */
export function phaseAt(layer: EditorLayer, seconds: number): number {
  const length = safeLength(layer.timeline.length);
  const phase = (seconds % length) / length;
  return phase < 0 ? phase + 1 : phase;
}

/**
 * The field the loop shows at `phase`, in the editor's own units.
 *
 * A separate interpolation from the engine's, and deliberately not a
 * replacement for it: the engine's is what reaches the wall and this only ever
 * *seeds* a new key. Positions travel on the normalised axes, so they match;
 * colour crosses Oklab and comes back through an eight-bit hex, so a key
 * inserted mid-fade can differ from the fade by a value nobody can see, and
 * that is worth it for a key whose colour is a hex string like every other.
 */
export function fieldAt(
  layer: EditorLayer,
  phase: number,
): { colorKeyframes: ColorKeyframe[]; blend: number } {
  const keys = keysOf(layer);
  if (keys.length < 2 || !layer.timeline.enabled) {
    return { colorKeyframes: keys[0].colorKeyframes.map(clone), blend: keys[0].blend };
  }

  // The bracketing pair, wrapping past the last key back to the first, exactly
  // as `ColorSurface.seek` does.
  let a = 0;
  while (a + 1 < keys.length && keys[a + 1].at <= phase) a += 1;
  const b = (a + 1) % keys.length;

  const span = wrap(keys[b].at - keys[a].at);
  const travelled = wrap(phase - keys[a].at);
  const t = span > 1e-6 ? clamp(travelled / span, 0, 1) : 0;

  const from = keys[a];
  const to = keys[b];
  const byId = new Map(to.colorKeyframes.map((k) => [k.id, k]));

  return {
    blend: lerp(from.blend, to.blend, t),
    colorKeyframes: from.colorKeyframes.map((k) => {
      const end = byId.get(k.id);
      if (!end) return clone(k);
      return {
        id: k.id,
        hz: normToHz(lerp(hzToNorm(k.hz), hzToNorm(end.hz), t)),
        db: clamp(normToDb(lerp(dbToNorm(k.db), dbToNorm(end.db), t)), DB_MIN, DB_MAX),
        color: mixHex(k.color, end.color, t),
        radius: mixRadius(k.radius, end.radius, t),
      };
    }),
  };
}

/**
 * The wire form: the layer's own field first, then the rest.
 *
 * Empty when nothing animates, which is what keeps a show that predates
 * timelines rendering byte for byte as it did. The first key is written from
 * the layer's own field rather than stored beside it, so the two can never
 * disagree.
 */
export function toEngineTimeline(
  layer: EditorLayer,
  surfaceOfKey: (key: KeyView) => TimelineKey["surface"],
): Timeline {
  const keys = keysOf(layer);
  return {
    enabled: layer.timeline.enabled,
    length: safeLength(layer.timeline.length),
    keys:
      keys.length < 2
        ? []
        : keys.map((k) => ({ id: k.base ? "start" : k.id, at: k.at, surface: surfaceOfKey(k) })),
  };
}

function clone(k: ColorKeyframe): ColorKeyframe {
  return { ...k };
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function wrap(delta: number): number {
  return delta >= 0 ? delta : delta + 1;
}

/** Two colours part of the way apart, through Oklab — so red to blue passes
 *  through the purples rather than through grey. */
function mixHex(from: string, to: string, t: number): string {
  const a = hexToOklab(from);
  const b = hexToOklab(to);
  return oklabToHex({
    l: lerp(a.l, b.l, t),
    a: lerp(a.a, b.a, t),
    b: lerp(a.b, b.b, t),
    alpha: lerp(a.alpha, b.alpha, t),
  });
}

/**
 * Two areas of effect part of the way apart.
 *
 * `null` is "everywhere" and an interpolation cannot be halfway to it, so it
 * stands in as {@link UNCONFINED} — which is also the top of the radius slider,
 * and is what the engine does. Mirrors `mix` in `color/surface.ts`.
 */
function mixRadius(from: number | null, to: number | null, t: number): number | null {
  if (from === null && to === null) return null;
  const r = lerp(from ?? UNCONFINED, to ?? UNCONFINED, t);
  return r >= UNCONFINED ? null : r;
}
