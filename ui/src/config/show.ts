/**
 * The document: a stack of layers, the master settings that describe the wall
 * itself, and the view state that describes only this browser.
 *
 * This is also the one place the engine's wire shape is known. That matters
 * more than it used to — the engine still runs a single analyser and a single
 * `ShowConfig`, so `toEngineConfig` has to pick *one* spectrum layer to send.
 * Compositing is therefore local-only for now, which is exactly the split the
 * README already describes for thresholds and sectors: the editor works it out,
 * and this module is the reference the engine will be checked against when it
 * grows a stack of its own.
 */

import type { ShowConfig } from "../engine";
import {
  DEFAULT_GIZMOS,
  DEFAULT_LED_COUNT,
  defaultSpectrumState,
  makeLayer,
  nextId,
  toSurface,
  type GizmoFlags,
  type Layer,
  type LayerKey,
  type SpectrumState,
} from "./layers";
import { DB_MAX, DB_MIN, clamp, normToDb, normToHz } from "./scales";
import { stateAt } from "./timeline";

/**
 * Analysis hop, in samples. Matches the engine's `MIN_HOP`..`MAX_HOP`; anything
 * outside is clamped there rather than rejected.
 *
 * Not an FFT window length — the engine draws every band from the smallest
 * transform that can still resolve it, five tiers from 8192 down to 128, so
 * there is no single window to set. This is how often those transforms run.
 */
export const SAMPLE_LENGTHS = [64, 128, 256, 512, 1024, 2048];

/**
 * Settings that describe the wall rather than any one layer.
 *
 * Reverse and mirror are here and not on a layer deliberately: they are
 * properties of how the strip is physically mounted, and two layers disagreeing
 * about which end is which is not an effect anyone wants. Decay and frame hop
 * are here because there is one analyser — per-layer values would need one
 * analyser each.
 */
export interface MasterConfig {
  /** Play the strip back to front. */
  reverse: boolean;
  /** Fold the whole range into each half of the strip. */
  mirror: boolean;
  /** Per-frame level retention. 0 snaps, 1 never falls. */
  decay: number;
  /** Samples captured before each transform. */
  sampleLength: number;
}

/** Never sent anywhere and never animated — it describes this browser only. */
export interface ViewConfig {
  gizmos: GizmoFlags;
  /** Whether the timeline shows every layer's lane or only the focused one. */
  allLanes: boolean;
}

export interface ShowDoc {
  /** Bottom of the stack first, so index order is compositing order. */
  layers: Layer[];
  focusedId: string | null;
  /** Non-null hides every other layer, without touching their enabled flags. */
  soloId: string | null;
  master: MasterConfig;
  view: ViewConfig;
  /** Transport length in seconds. Layers loop within it at their own lengths. */
  duration: number;
}

export const DEFAULT_MASTER: MasterConfig = {
  reverse: false,
  mirror: false,
  // Both match the engine's own defaults, so a fresh editor connecting does not
  // immediately push a change and rebuild the analyser.
  decay: 0.82,
  sampleLength: 256,
};

export const DEFAULT_DURATION = 8;

export function defaultDoc(ledCount = DEFAULT_LED_COUNT): ShowDoc {
  const spectrum = makeLayer("spectrum", ledCount);
  return {
    layers: [spectrum],
    focusedId: spectrum.id,
    soloId: null,
    master: { ...DEFAULT_MASTER },
    view: { gizmos: { ...DEFAULT_GIZMOS }, allLanes: false },
    duration: DEFAULT_DURATION,
  };
}

/** The layer the engine is being driven from, or null if the stack has none. */
export function drivingLayer(doc: ShowDoc): Layer | null {
  return doc.layers.find((l) => l.kind === "spectrum" && l.enabled && l.state.source === "audio")
    ?? doc.layers.find((l) => l.kind === "spectrum")
    ?? null;
}

/**
 * Everything the engine acts on, from the one layer it can represent.
 *
 * The protocol carries a single surface, so a stack of two spectrum layers
 * cannot cross the wire yet and the strip will show only this one. The local
 * `Preview` shows the true composite, which is why the two can legitimately
 * disagree while the backend is still single-layer — the panel says so.
 */
export function toEngineConfig(doc: ShowDoc, time: number): ShowConfig | null {
  const layer = drivingLayer(doc);
  if (!layer || layer.kind !== "spectrum") return null;
  const state = stateAt(layer, time) as SpectrumState;
  return {
    surface: toSurface(state),
    eq: state.eq,
    ledKeyframes: state.ledKeyframes,
    reverse: doc.master.reverse,
    mirror: doc.master.mirror,
    threshold: state.threshold,
    clamp: state.clamp,
    curve: state.curve,
    decay: doc.master.decay,
    sampleLength: doc.master.sampleLength,
  };
}

/** Adopt what the engine is running into the driving layer and the master. */
export function fromEngineConfig(doc: ShowDoc, show: ShowConfig): ShowDoc {
  const layer = drivingLayer(doc);
  const master: MasterConfig = {
    reverse: show.reverse,
    mirror: show.mirror,
    decay: show.decay,
    sampleLength: show.sampleLength,
  };
  if (!layer || layer.kind !== "spectrum") return { ...doc, master };

  const state: SpectrumState = {
    ...layer.state,
    blendRadius: show.surface.sigma,
    colorKeyframes: show.surface.keyframes.map((k) => ({
      id: nextId("ck"),
      hz: normToHz(k.x),
      db: normToDb(k.y),
      color: k.color,
    })),
    // Ids are the editor's own bookkeeping and never cross the wire, so they
    // are minted fresh rather than expected back.
    eq: show.eq.map((b) => ({ ...b, id: nextId("eq") })),
    ledKeyframes: show.ledKeyframes.map((k) => ({ ...k, id: nextId("led") })),
    threshold: show.threshold,
    clamp: show.clamp,
    curve: show.curve,
  };

  return {
    ...doc,
    master,
    // Cast because spreading a discriminated union re-widens its members; the
    // kind guard above is what actually makes this sound.
    layers: doc.layers.map((l) => (l.id === layer.id ? ({ ...l, state } as Layer) : l)),
  };
}

/* ------------------------------------------------------------- persistence */

const STORAGE_KEY = "djled.show";
/** The pre-layers editor. Read once, to migrate, and never written again. */
const LEGACY_KEY = "djled.editor";

export function loadDoc(ledCount = DEFAULT_LED_COUNT): ShowDoc {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return reviveDoc(JSON.parse(raw) as Partial<ShowDoc>, ledCount);

    const legacy = localStorage.getItem(LEGACY_KEY);
    if (legacy) return migrateLegacy(JSON.parse(legacy) as Record<string, unknown>, ledCount);
  } catch {
    // A hand-edited or stale document must not brick the editor.
  }
  return defaultDoc(ledCount);
}

export function saveDoc(doc: ShowDoc): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(doc));
  } catch {
    // Private-mode quota failures are not worth interrupting an edit over.
  }
}

export function clearDoc(): void {
  localStorage.removeItem(STORAGE_KEY);
  localStorage.removeItem(LEGACY_KEY);
}

export function hasStoredDoc(): boolean {
  return localStorage.getItem(STORAGE_KEY) !== null || localStorage.getItem(LEGACY_KEY) !== null;
}

/**
 * Field-by-field fallback rather than a schema check.
 *
 * A document that has lost one setting should open with that setting at its
 * default, not refuse to open — the alternative loses an authored palette over
 * a missing boolean.
 */
function reviveDoc(parsed: Partial<ShowDoc>, ledCount: number): ShowDoc {
  const base = defaultDoc(ledCount);
  const layers = Array.isArray(parsed.layers) && parsed.layers.length
    ? parsed.layers.map((l) => reviveLayer(l, ledCount))
    : base.layers;
  const focusedId = layers.some((l) => l.id === parsed.focusedId)
    ? (parsed.focusedId as string)
    : layers[0].id;
  return {
    layers,
    focusedId,
    soloId: layers.some((l) => l.id === parsed.soloId) ? (parsed.soloId as string) : null,
    master: { ...base.master, ...parsed.master },
    view: {
      gizmos: { ...DEFAULT_GIZMOS, ...parsed.view?.gizmos },
      allLanes: parsed.view?.allLanes ?? false,
    },
    duration: positive(parsed.duration, base.duration),
  };
}

function reviveLayer(raw: Layer, ledCount: number): Layer {
  const fallback = makeLayer(raw?.kind === "static" ? "static" : "spectrum", ledCount);
  const common = {
    id: typeof raw?.id === "string" ? raw.id : fallback.id,
    name: typeof raw?.name === "string" ? raw.name : fallback.name,
    enabled: raw?.enabled ?? true,
    blendMode: raw?.blendMode ?? fallback.blendMode,
    opacity: clamp(Number(raw?.opacity ?? 1), 0, 1),
    loopSeconds: positive(raw?.loopSeconds, fallback.loopSeconds),
  };

  if (raw?.kind === "static" && fallback.kind === "static") {
    const state = { ...fallback.state, ...raw.state };
    return {
      ...common,
      kind: "static",
      state: {
        ...state,
        stops: Array.isArray(state.stops) && state.stops.length ? state.stops : fallback.state.stops,
      },
      keys: Array.isArray(raw.keys) ? raw.keys : [],
    };
  }

  const spectrumFallback = defaultSpectrumState(ledCount);
  const state = { ...spectrumFallback, ...(raw?.state as Partial<SpectrumState>) };
  return {
    ...common,
    kind: "spectrum",
    state: {
      ...state,
      colorKeyframes: Array.isArray(state.colorKeyframes) && state.colorKeyframes.length
        ? state.colorKeyframes
        : spectrumFallback.colorKeyframes,
      ledKeyframes: Array.isArray(state.ledKeyframes) && state.ledKeyframes.length
        ? state.ledKeyframes
        : spectrumFallback.ledKeyframes,
      // Empty is a legitimate EQ, so unlike the keyframes this only guards
      // against the field being absent or the wrong shape.
      eq: Array.isArray(state.eq) ? state.eq : [],
      curve: { ...spectrumFallback.curve, ...state.curve },
      threshold: clampDb(state.threshold, spectrumFallback.threshold),
      clamp: clampDb(state.clamp, spectrumFallback.clamp),
    },
    // The `fallback.kind` half of the branch above stops TS narrowing `raw` to
    // the spectrum member here, so the cast says what the branch already knows.
    keys: Array.isArray(raw?.keys) ? (raw.keys as Array<LayerKey<SpectrumState>>) : [],
  };
}

/**
 * The pre-layers editor was exactly one spectrum layer plus the master, so it
 * migrates without asking anything: a saved palette survives the redesign.
 */
function migrateLegacy(old: Record<string, unknown>, ledCount: number): ShowDoc {
  const doc = defaultDoc(ledCount);
  const layer = doc.layers[0];
  if (layer.kind !== "spectrum") return doc;

  const state: SpectrumState = {
    ...layer.state,
    colorKeyframes: asArray(old.colorKeyframes, layer.state.colorKeyframes),
    ledKeyframes: asArray(old.ledKeyframes, layer.state.ledKeyframes),
    eq: Array.isArray(old.eq) ? (old.eq as SpectrumState["eq"]) : [],
    threshold: clampDb(old.threshold, layer.state.threshold),
    clamp: clampDb(old.clamp, layer.state.clamp),
    curve: { ...layer.state.curve, ...(old.curve as object) },
    blendRadius: positive(old.blend, layer.state.blendRadius),
  };

  return {
    ...doc,
    layers: [{ ...layer, state } as Layer],
    master: {
      reverse: Boolean(old.reverse),
      mirror: Boolean(old.mirror),
      decay: typeof old.decay === "number" ? old.decay : DEFAULT_MASTER.decay,
      sampleLength:
        typeof old.sampleLength === "number" ? old.sampleLength : DEFAULT_MASTER.sampleLength,
    },
    view: { gizmos: { ...DEFAULT_GIZMOS, ...(old.gizmos as object) }, allLanes: false },
  };
}

function asArray<T>(value: unknown, fallback: T[]): T[] {
  return Array.isArray(value) && value.length ? (value as T[]) : fallback;
}

function positive(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : fallback;
}

function clampDb(value: unknown, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return clamp(value, DB_MIN, DB_MAX);
}
