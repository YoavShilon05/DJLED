/**
 * The layer stack.
 *
 * One strip, several things painting it. Each layer renders a full strip's worth
 * of colour plus an alpha, and the compositor folds them bottom-up — so what a
 * layer *is* stays separate from how it *combines*, and adding a kind never
 * touches the blending.
 *
 * Two rules hold the model together:
 *
 *  1. **A layer's `state` is the only animatable part.** Everything the timeline
 *     tweens lives in `SpectrumState` or `StaticState`; everything else (name,
 *     blend mode, loop length) is layer chrome and holds still. That is what
 *     lets a snapshot be a plain structural clone rather than a field list.
 *  2. **Nothing here knows about pixels.** Rendering lives in `render/`, so this
 *     module is safe to import from anywhere including tests.
 */

import { DEFAULT_SURFACE } from "../color/surface";
import type { SurfaceConfig } from "../color/surface";
import type { CurveConfig } from "./curve";
import type { EqBand } from "./eq";
import { dbToNorm, hzToNorm, normToDb, normToHz } from "./scales";

let counter = 0;
export function nextId(prefix: string): string {
  counter += 1;
  return `${prefix}-${Date.now().toString(36)}-${counter}`;
}

/** A colour authored at a point on the spectrum plot. */
export interface ColorKeyframe {
  id: string;
  hz: number;
  db: number;
  /** sRGB hex, e.g. `#ff2000`. */
  color: string;
}

/**
 * A frequency pinned to an LED index. Consecutive keyframes define a sector:
 * LEDs between two of them display the frequencies between them, spread
 * linearly along the log axis.
 */
export interface LedKeyframe {
  id: string;
  led: number;
  hz: number;
}

/** A colour authored at a position along a static layer's span. */
export interface GradientStop {
  id: string;
  /** 0..1 across the layer's LED span. */
  pos: number;
  /** sRGB hex. */
  color: string;
}

/**
 * How a layer combines with everything beneath it, already flattened.
 *
 * Compositing happens in linear RGB rather than Oklab, deliberately: `add` is
 * two lamps pointed at one wall and `multiply` is a gel over a lamp, and both
 * are physics. Oklab stays where perceptual uniformity is actually wanted —
 * interpolating *within* a layer's gradient.
 *
 * `tint` is deliberately absent. It needs a partner layer rather than the
 * flattened stack (hue sampled from an accumulation that contains a reactive
 * layer is undefined wherever that layer is dark), which means a clip
 * relationship in the layer list. Not yet.
 */
export type BlendMode = "normal" | "add" | "multiply";

export const BLEND_OPTIONS: Array<{ value: BlendMode; label: string }> = [
  { value: "normal", label: "Normal" },
  { value: "add", label: "Add" },
  { value: "multiply", label: "Multiply" },
];

/**
 * Where a spectrum layer's x axis comes from.
 *
 * Not two layer kinds, because it is not two axes. MIDI note n is
 * `440·2^((n−69)/12)` Hz, so note number *is* the log-frequency axis relabelled
 * — note 21 lands at 27.5 Hz and note 127 at 12.5 kHz, both inside the plot's
 * existing 20 Hz–20 kHz range. Velocity normalises the same way a level does.
 * So the colour surface, the LED mapping and the intensity curve all transfer
 * across a source change untouched, and only the ruler changes.
 */
export type SpectrumSource = "audio" | "midi";

/** Everything the timeline may tween on a spectrum layer. */
export interface SpectrumState {
  source: SpectrumSource;
  colorKeyframes: ColorKeyframe[];
  ledKeyframes: LedKeyframe[];
  /** Applied to the spectrum before anything else looks at it. */
  eq: EqBand[];
  /** dB below which a band is dark. */
  threshold: number;
  /** dB at and above which a band is at full brightness. */
  clamp: number;
  curve: CurveConfig;
  /** Reach of each colour keyframe's influence — the surface's sigma. */
  blendRadius: number;
}

/** Everything the timeline may tween on a static layer. */
export interface StaticState {
  stops: GradientStop[];
  /** First LED the gradient covers. */
  from: number;
  /** Last LED the gradient covers, inclusive. */
  to: number;
  /** The layer's own alpha before the stack opacity — 0 is invisible. */
  level: number;
}

export type LayerState = SpectrumState | StaticState;

/** A whole-layer snapshot at a point on the loop. */
export interface LayerKey<S extends LayerState> {
  id: string;
  /** Seconds from the start of this layer's loop. */
  t: number;
  state: S;
}

interface LayerCommon {
  id: string;
  name: string;
  /** Off means it contributes nothing; it still holds its state and its keys. */
  enabled: boolean;
  blendMode: BlendMode;
  /** 0..1, multiplied into the layer's alpha. */
  opacity: number;
  /**
   * Loop length in seconds. Phase is `time % loopSeconds` against the shared
   * transport clock rather than a clock of its own, which is what keeps a 2 s
   * layer and an 8 s layer aligned instead of drifting apart.
   */
  loopSeconds: number;
}

export interface SpectrumLayer extends LayerCommon {
  kind: "spectrum";
  state: SpectrumState;
  keys: Array<LayerKey<SpectrumState>>;
}

export interface StaticLayer extends LayerCommon {
  kind: "static";
  state: StaticState;
  keys: Array<LayerKey<StaticState>>;
}

export type Layer = SpectrumLayer | StaticLayer;
export type LayerKind = Layer["kind"];

export const LAYER_KIND_LABEL: Record<LayerKind, string> = {
  spectrum: "Spectrum",
  static: "Static colour",
};

/** Which overlays the focused spectrum editor draws. View state, never tweened. */
export interface GizmoFlags {
  thresholds: boolean;
  colorKeyframes: boolean;
  ledKeyframes: boolean;
  curve: boolean;
  eq: boolean;
}

export const DEFAULT_GIZMOS: GizmoFlags = {
  thresholds: true,
  colorKeyframes: true,
  ledKeyframes: true,
  curve: true,
  eq: true,
};

/** Default LED count, until the engine reports its own. */
export const DEFAULT_LED_COUNT = 150;

/**
 * Seeded from the surface the engine ships with, so a fresh editor previews
 * what the strip is already doing rather than a blank field.
 */
export function defaultSpectrumState(ledCount = DEFAULT_LED_COUNT): SpectrumState {
  return {
    source: "audio",
    colorKeyframes: DEFAULT_SURFACE.keyframes.map((k) => ({
      id: nextId("ck"),
      hz: normToHz(k.x),
      db: normToDb(k.y),
      color: k.color,
    })),
    ledKeyframes: [
      { id: nextId("led"), led: 0, hz: 20 },
      { id: nextId("led"), led: Math.max(0, ledCount - 1), hz: 20_000 },
    ],
    // Flat. The EQ is a correction layer, so having it do nothing until asked is
    // the only honest default.
    eq: [],
    threshold: -62,
    clamp: -6,
    curve: { type: "linear", p1: { x: 0.25, y: 0.1 }, p2: { x: 0.25, y: 1 } },
    blendRadius: DEFAULT_SURFACE.sigma,
  };
}

export function defaultStaticState(ledCount = DEFAULT_LED_COUNT): StaticState {
  return {
    stops: [
      { id: nextId("gs"), pos: 0, color: "#2000ff" },
      { id: nextId("gs"), pos: 0.5, color: "#c400ff" },
      { id: nextId("gs"), pos: 1, color: "#ff2000" },
    ],
    from: 0,
    to: Math.max(0, ledCount - 1),
    // Well below full: a static layer is usually a wash under something, and a
    // wash at full brightness is a wall of colour rather than a bed.
    level: 0.35,
  };
}

export const DEFAULT_LOOP_SECONDS = 8;

export function makeLayer(kind: LayerKind, ledCount = DEFAULT_LED_COUNT): Layer {
  const common = {
    id: nextId("layer"),
    enabled: true,
    opacity: 1,
    loopSeconds: DEFAULT_LOOP_SECONDS,
  };
  if (kind === "static") {
    return {
      ...common,
      kind: "static",
      name: "Static colour",
      // Normal, not add: a static layer is usually the bed everything else sits
      // on, and adding it to nothing is the same thing with a worse name.
      blendMode: "normal",
      state: defaultStaticState(ledCount),
      keys: [],
    };
  }
  return {
    ...common,
    kind: "spectrum",
    name: "Spectrum",
    blendMode: "add",
    state: defaultSpectrumState(ledCount),
    keys: [],
  };
}

/**
 * Colour keyframes in the form the renderer samples: normalised over the unit
 * square, with x on the log frequency axis and y the band's level.
 */
export function toSurface(state: SpectrumState): SurfaceConfig {
  return {
    sigma: state.blendRadius,
    keyframes: state.colorKeyframes.map((k) => ({
      x: hzToNorm(k.hz),
      y: dbToNorm(k.db),
      color: k.color,
    })),
  };
}

/** Stops in ascending position, which is the only order the ramp can be read in. */
export function sortStops(stops: GradientStop[]): GradientStop[] {
  return [...stops].sort((a, b) => a.pos - b.pos);
}

/** A layer is only meaningfully soloed against others, so this lives on the list. */
export function visibleLayers(layers: Layer[], soloId: string | null): Layer[] {
  if (soloId) return layers.filter((l) => l.id === soloId);
  return layers.filter((l) => l.enabled);
}

export function findLayer(layers: Layer[], id: string | null): Layer | null {
  return layers.find((l) => l.id === id) ?? null;
}

/** Replace one layer, leaving the rest of the array identity-stable. */
export function replaceLayer(layers: Layer[], id: string, next: Layer): Layer[] {
  return layers.map((l) => (l.id === id ? next : l));
}

export function moveLayer(layers: Layer[], id: string, delta: number): Layer[] {
  const i = layers.findIndex((l) => l.id === id);
  if (i < 0) return layers;
  const j = i + delta;
  if (j < 0 || j >= layers.length) return layers;
  const out = [...layers];
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}
