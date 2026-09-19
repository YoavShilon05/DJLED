/**
 * Everything the editor holds, and the one place it is translated for the engine.
 *
 * The whole config crosses the wire as a single `ShowConfig`; `toEngineConfig`
 * and `fromEngineConfig` are the only two places that shape is known, so a
 * protocol change lands here rather than being spread across the editor.
 *
 * # The stack
 *
 * A show is a list of layers, bottom first, and that order *is* the compositing
 * order — dragging a row in the editor reorders this array and nothing else.
 * Everything that shapes a look belongs to a layer: its colour keyframes, its
 * sectors, its EQ, its decay, its thresholds, and what it listens to.
 *
 * Two things stay on the show rather than on a layer, and for the same reason
 * in both cases — they are not part of any one layer's appearance:
 *
 * - **Gizmo visibility**, which is about reading the plot, not about the strip.
 * - **The active layer**, which is where edits land. Kept here so it survives a
 *   reload alongside the rest.
 *
 * Master brightness is neither: it is the power budget for the whole strip and
 * lives with the engine.
 */

import { DEFAULT_SURFACE, type SurfaceConfig } from "../color/surface";
import type { InputSource, LayerConfig, MidiConfig, ShowConfig } from "../engine";
import type { CurveConfig } from "./curve";
import type { EqBand } from "./eq";
import { DEFAULT_HIGH_NOTE, DEFAULT_LOW_NOTE, noteRange } from "./notes";
import { DB_MAX, DB_MIN, dbToNorm, hzToNorm, normToDb, normToHz } from "./scales";

/** A colour authored at a point on the spectrum plot. */
export interface ColorKeyframe {
  id: string;
  hz: number;
  db: number;
  /**
   * sRGB hex, either `#ff2000` or `#ff2000cc` with an opacity byte.
   *
   * Opacity rides in the colour rather than in a field of its own, so it travels
   * the wire, the presets and the colour picker as one value. Six digits means
   * opaque, which is what keeps stored configs from before the channel existed
   * loading unchanged.
   *
   * With a stack under it, opacity is the difference between a layer that lets
   * the one below show through and one that paints over it. Black is not the
   * same as transparent: black covers.
   */
  color: string;
  /**
   * How far this colour reaches, as a distance in the unit square — the same
   * space {@link EditorLayer.blend} is measured in, so the two are comparable
   * even though they answer different questions. `null` is unconfined, which is
   * what every keyframe was before this existed.
   *
   * Blend radius decides how two keyframes that both reach a point share it;
   * this decides whether a keyframe is in that conversation at all. Sigma is one
   * number for the whole layer, so it cannot confine one colour without
   * sharpening every other one at the same time.
   *
   * Past every keyframe's reach the field is transparent black, so confining
   * colours leaves holes the layers below show through rather than black.
   */
  radius: number | null;
}

/**
 * Area of effect, from just-a-point to past the diagonal of the unit square.
 *
 * The top of the range is ‖(1, 1)‖ rounded up: at that reach a keyframe in one
 * corner touches the opposite one, so anything larger is the same field twice.
 */
export const RADIUS_MIN = 0.02;
export const RADIUS_MAX = 1.45;

/** What un-ticking "infinite" starts from. About a third of the field, which is
 *  visibly an area of effect without being one that covers everything. */
export const RADIUS_DEFAULT = 0.35;

/**
 * A frequency pinned to an LED index. Consecutive keyframes define a sector:
 * LEDs between two of them display the frequencies between them, spread
 * linearly along the log axis.
 *
 * Per layer, so sectors are also how a layer is confined to part of the wall.
 * A position no sector reaches is not painted by that layer at all — which is
 * different from being painted black, and only visible once something is
 * underneath.
 */
export interface LedKeyframe {
  id: string;
  led: number;
  hz: number;
}

export interface GizmoFlags {
  thresholds: boolean;
  colorKeyframes: boolean;
  ledKeyframes: boolean;
  curve: boolean;
  eq: boolean;
}

/** One layer of the stack, as the editor authors it. */
export interface EditorLayer {
  id: string;
  name: string;
  /** Off skips the layer entirely — the engine does not even analyse it. */
  enabled: boolean;
  /** Master opacity, 0..1. Fades toward what is underneath, not toward black. */
  opacity: number;
  /** What this layer listens to. Layers naming the same device share one open
   *  handle in the engine, so a five-layer show on one output is one capture. */
  source: InputSource;
  colorKeyframes: ColorKeyframe[];
  ledKeyframes: LedKeyframe[];
  /** Applied to this layer's spectrum before anything else looks at it. */
  eq: EqBand[];
  /** Play this layer back to front. */
  reverse: boolean;
  /** Fold this layer's range into each half of the strip. */
  mirror: boolean;
  /** dB below which a band is dark. */
  threshold: number;
  /** dB at and above which a band is at full brightness. */
  clamp: number;
  curve: CurveConfig;
  /** Per-frame level retention. 0 snaps, 1 never falls. */
  decay: number;
  /** Samples captured before each transform. */
  sampleLength: number;
  /** How MIDI notes land on the axis, when this layer is driven by MIDI. */
  midi: MidiConfig;
  /** How sharply colour keyframes hand over to each other — the surface's
   *  sigma. One number for the whole layer, which is why confining a single
   *  colour is {@link ColorKeyframe.radius} and not this. */
  blend: number;
}

export interface EditorConfig {
  /** Bottom first. Never empty; see {@link sanitiseLayers}. */
  layers: EditorLayer[];
  /** Where edits land. Falls back to the top layer when it names nothing real. */
  activeLayerId: string;
  gizmos: GizmoFlags;
}

/**
 * Analysis hop, in samples. Matches the engine's `MIN_HOP`..`MAX_HOP`; anything
 * outside is clamped there rather than rejected.
 *
 * Not an FFT window length — the engine draws every band from the smallest
 * transform that can still resolve it, five tiers from 8192 down to 128, so
 * there is no single window to set. This is how often those transforms run.
 */
export const SAMPLE_LENGTHS = [64, 128, 256, 512, 1024, 2048];

/** Layers a show may hold. Not a technical limit — a stack past this stops
 *  being something anyone can reason about, and every layer is an analyser. */
export const MAX_LAYERS = 12;

let counter = 0;
export function nextId(prefix: string): string {
  counter += 1;
  return `${prefix}-${Date.now().toString(36)}-${counter}`;
}

/** The system default output, which is what a new layer listens to. */
export const DEFAULT_SOURCE: InputSource = { id: null, kind: "loopback", channel: null };

/**
 * A layer seeded from the surface the engine ships with, so a fresh editor
 * previews what the strip is already doing rather than a blank field.
 */
export function defaultLayer(name: string): EditorLayer {
  return {
    id: nextId("layer"),
    name,
    enabled: true,
    opacity: 1,
    source: { ...DEFAULT_SOURCE },
    colorKeyframes: DEFAULT_SURFACE.keyframes.map((k) => ({
      id: nextId("ck"),
      hz: normToHz(k.x),
      db: normToDb(k.y),
      color: k.color,
      radius: k.radius ?? null,
    })),
    ledKeyframes: [
      { id: nextId("led"), led: 0, hz: 20 },
      { id: nextId("led"), led: 149, hz: 20_000 },
    ],
    // Flat. The EQ is a correction layer, so having it do nothing until asked is
    // the only honest default.
    eq: [],
    reverse: false,
    mirror: false,
    threshold: -62,
    clamp: -6,
    curve: { type: "linear", p1: { x: 0.25, y: 0.1 }, p2: { x: 0.25, y: 1 } },
    // Both match the engine's own defaults, so a fresh editor connecting does not
    // immediately push a change and rebuild the analyser.
    decay: 0.82,
    sampleLength: 256,
    // An 88-key piano, stretched across the whole strip, with a semitone of glow
    // either side of each note. Matches the engine's own defaults.
    midi: { lowNote: DEFAULT_LOW_NOTE, highNote: DEFAULT_HIGH_NOTE, spread: 1, sustain: true },
    blend: DEFAULT_SURFACE.sigma,
  };
}

/**
 * A layer to stack on top of an existing one.
 *
 * Deliberately *not* a copy of the default: dropping a second full-strip opaque
 * palette on top of the first would hide everything under it, and the first
 * thing anyone does with a new layer is wonder where their old one went. So a
 * new layer starts transparent at the bottom of its field and coloured at the
 * top — it lights where there is signal and shows the layer below where there
 * is not, which is the behaviour the stack exists for.
 */
export function newLayer(name: string): EditorLayer {
  const base = defaultLayer(name);
  return {
    ...base,
    colorKeyframes: [
      { id: nextId("ck"), hz: 20, db: DB_MIN, color: "#00000000", radius: null },
      { id: nextId("ck"), hz: 20_000, db: DB_MIN, color: "#00000000", radius: null },
      { id: nextId("ck"), hz: 20, db: DB_MAX, color: "#ffffff", radius: null },
      { id: nextId("ck"), hz: 20_000, db: DB_MAX, color: "#ffffff", radius: null },
    ],
  };
}

export const DEFAULT_CONFIG: EditorConfig = (() => {
  const layer = defaultLayer("Layer 1");
  return {
    layers: [layer],
    activeLayerId: layer.id,
    gizmos: {
      thresholds: true,
      colorKeyframes: true,
      ledKeyframes: true,
      curve: true,
      eq: true,
    },
  };
})();

/** The layer edits currently land on. Never null: a stack always has a layer,
 *  and an id naming nothing falls back to the top of the stack. */
export function activeLayer(config: EditorConfig): EditorLayer {
  return (
    config.layers.find((l) => l.id === config.activeLayerId) ??
    config.layers[config.layers.length - 1]
  );
}

/** Replace one layer, leaving the rest of the stack and its order alone. */
export function withLayer(config: EditorConfig, layer: EditorLayer): EditorConfig {
  return { ...config, layers: config.layers.map((l) => (l.id === layer.id ? layer : l)) };
}

/**
 * Colour keyframes in the form the renderer samples: normalised over the unit
 * square, with x on the log frequency axis and y the band's level.
 */
export function toSurface(layer: EditorLayer): SurfaceConfig {
  return {
    sigma: layer.blend,
    keyframes: layer.colorKeyframes.map((k) => ({
      x: hzToNorm(k.hz),
      y: dbToNorm(k.db),
      color: k.color,
      // Left off entirely when unconfined, which is how the engine writes it —
      // so a show authored here and one round-tripped through the engine are
      // the same JSON rather than differing by a field full of nulls.
      ...(k.radius === null ? {} : { radius: k.radius }),
    })),
  };
}

function toEngineLayer(layer: EditorLayer): LayerConfig {
  return {
    id: layer.id,
    name: layer.name,
    enabled: layer.enabled,
    opacity: layer.opacity,
    source: layer.source,
    surface: toSurface(layer),
    eq: layer.eq,
    ledKeyframes: layer.ledKeyframes,
    reverse: layer.reverse,
    mirror: layer.mirror,
    threshold: layer.threshold,
    clamp: layer.clamp,
    curve: layer.curve,
    decay: layer.decay,
    sampleLength: layer.sampleLength,
    midi: layer.midi,
  };
}

/** Everything the engine acts on. Gizmo visibility stays here, as it should. */
export function toEngineConfig(config: EditorConfig): ShowConfig {
  return { layers: config.layers.map(toEngineLayer) };
}

function fromEngineLayer(layer: LayerConfig): EditorLayer {
  const base = defaultLayer(layer.name || "Layer");
  return {
    ...base,
    id: layer.id || base.id,
    name: layer.name || base.name,
    enabled: layer.enabled !== false,
    opacity: typeof layer.opacity === "number" ? layer.opacity : 1,
    source: layer.source ?? { ...DEFAULT_SOURCE },
    blend: layer.surface.sigma,
    colorKeyframes: layer.surface.keyframes.map((k) => ({
      id: nextId("ck"),
      hz: normToHz(k.x),
      db: normToDb(k.y),
      color: k.color,
      radius: typeof k.radius === "number" ? k.radius : null,
    })),
    // Ids are the editor's own bookkeeping and never cross the wire, so they
    // are minted fresh rather than expected back.
    eq: layer.eq.map((b) => ({ ...b, id: nextId("eq") })),
    ledKeyframes: layer.ledKeyframes.map((k) => ({ ...k, id: nextId("led") })),
    reverse: layer.reverse,
    mirror: layer.mirror,
    threshold: layer.threshold,
    clamp: layer.clamp,
    curve: layer.curve,
    decay: layer.decay,
    sampleLength: layer.sampleLength,
    midi: { ...base.midi, ...layer.midi },
  };
}

/** Adopt what the engine is running, leaving editor-only settings untouched. */
export function fromEngineConfig(config: EditorConfig, show: ShowConfig): EditorConfig {
  const layers = (show.layers ?? []).map(fromEngineLayer);
  if (layers.length === 0) return config;
  return {
    ...config,
    layers,
    // Whatever was selected is unlikely to exist in a stack the editor did not
    // author, so selection falls to the top rather than to nothing.
    activeLayerId: layers.some((l) => l.id === config.activeLayerId)
      ? config.activeLayerId
      : layers[layers.length - 1].id,
  };
}

const STORAGE_KEY = "djled.editor";

/**
 * The shape stored before layers existed: one layer's worth of fields at the
 * top level. Kept only so a stored preset can be migrated on load.
 */
type StoredV1 = Partial<Omit<EditorLayer, "id" | "name" | "enabled" | "opacity" | "source">> & {
  gizmos?: Partial<GizmoFlags>;
};

export function loadConfig(): EditorConfig {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_CONFIG;
    const parsed = JSON.parse(raw) as Partial<EditorConfig> & StoredV1;

    // A stored config from before the stack has its single layer spread across
    // the top level. Folding it into one layer is what keeps a saved look from
    // being silently replaced by the default the day layers landed.
    const layers = Array.isArray(parsed.layers)
      ? parsed.layers.map(sanitiseLayer)
      : [sanitiseLayer({ ...parsed, name: "Layer 1" } as Partial<EditorLayer>)];

    const stack = sanitiseLayers(layers);
    return {
      layers: stack,
      activeLayerId: stack.some((l) => l.id === parsed.activeLayerId)
        ? (parsed.activeLayerId as string)
        : stack[stack.length - 1].id,
      gizmos: { ...DEFAULT_CONFIG.gizmos, ...parsed.gizmos },
    };
  } catch {
    return DEFAULT_CONFIG;
  }
}

/**
 * A stored layer, made safe to draw with.
 *
 * Anything missing or structurally wrong falls back field by field rather than
 * discarding the layer: a hand-edited or stale preset must not brick the editor.
 */
function sanitiseLayer(stored: Partial<EditorLayer>): EditorLayer {
  const base = defaultLayer(stored.name?.trim() || "Layer");
  return {
    ...base,
    ...stored,
    id: typeof stored.id === "string" && stored.id ? stored.id : base.id,
    name: stored.name?.trim() || base.name,
    enabled: stored.enabled !== false,
    opacity: clamp01(stored.opacity, 1),
    source: sanitiseSource(stored.source),
    // Empty is a legitimate colour field — a layer that paints nothing — so like
    // the EQ this only guards against the field being absent or the wrong shape.
    // A stored keyframe missing a radius is one written before areas of effect
    // existed, and unconfined is what it meant.
    colorKeyframes: Array.isArray(stored.colorKeyframes)
      ? stored.colorKeyframes.map((k) => ({ ...k, radius: k.radius ?? null }))
      : base.colorKeyframes,
    // Sectors are not the same: a layer with no sectors reaches no LED at all,
    // and the engine would substitute a spanning layout anyway.
    ledKeyframes:
      Array.isArray(stored.ledKeyframes) && stored.ledKeyframes.length
        ? stored.ledKeyframes
        : base.ledKeyframes,
    eq: Array.isArray(stored.eq) ? stored.eq : base.eq,
    curve: { ...base.curve, ...stored.curve },
    threshold: clampDb(stored.threshold, base.threshold),
    clamp: clampDb(stored.clamp, base.clamp),
    midi: sanitiseMidi(stored.midi, base.midi),
  };
}

/**
 * A stack, made safe to render.
 *
 * Two guarantees the rest of the editor leans on: there is always at least one
 * layer, and no two share an id — ids are how a row finds its live telemetry,
 * and a duplicate would have two rows fighting over one plot.
 */
function sanitiseLayers(layers: EditorLayer[]): EditorLayer[] {
  const seen = new Set<string>();
  const out = layers.slice(0, MAX_LAYERS).map((layer) => {
    const id = seen.has(layer.id) ? nextId("layer") : layer.id;
    seen.add(id);
    return id === layer.id ? layer : { ...layer, id };
  });
  return out.length ? out : [defaultLayer("Layer 1")];
}

function sanitiseSource(stored: InputSource | undefined): InputSource {
  const kind = stored?.kind;
  return {
    id: typeof stored?.id === "string" ? stored.id : null,
    kind: kind === "input" || kind === "midi" || kind === "loopback" ? kind : "loopback",
    channel: typeof stored?.channel === "number" ? stored.channel : null,
  };
}

export function saveConfig(config: EditorConfig): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  } catch {
    // Private-mode quota failures are not worth interrupting an edit over.
  }
}

export function clearConfig(): void {
  localStorage.removeItem(STORAGE_KEY);
}

export function hasStoredConfig(): boolean {
  return localStorage.getItem(STORAGE_KEY) !== null;
}

/**
 * A stored MIDI section, made safe to draw with.
 *
 * The range goes through the same clamp the engine applies, so the editor never
 * draws an axis the strip is not using — an inverted or one-note range would
 * otherwise render as a divide by zero rather than as the octave the engine
 * quietly widened it to.
 */
function sanitiseMidi(stored: Partial<MidiConfig> | undefined, fallback: MidiConfig): MidiConfig {
  const merged = { ...fallback, ...stored };
  const [lowNote, highNote] = noteRange(merged.lowNote, merged.highNote);
  const spread = Number.isFinite(merged.spread) ? Math.min(12, Math.max(0, merged.spread)) : 1;
  return { lowNote, highNote, spread, sustain: merged.sustain !== false };
}

function clampDb(value: unknown, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(DB_MAX, Math.max(DB_MIN, value));
}

function clamp01(value: unknown, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(1, Math.max(0, value));
}
