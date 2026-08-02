/**
 * Everything the editor holds, and the one place it is translated for the engine.
 *
 * The whole config crosses the wire as a single `ShowConfig`; `toEngineConfig`
 * and `fromEngineConfig` are the only two places that shape is known, so a
 * protocol change lands here rather than being spread across the editor.
 */

import { DEFAULT_SURFACE, type SurfaceConfig } from "../color/surface";
import type { ShowConfig } from "../engine";
import type { CurveConfig } from "./curve";
import type { EqBand } from "./eq";
import { DB_MAX, DB_MIN, dbToNorm, hzToNorm, normToDb, normToHz } from "./scales";

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

export interface GizmoFlags {
  thresholds: boolean;
  colorKeyframes: boolean;
  ledKeyframes: boolean;
  curve: boolean;
  eq: boolean;
}

export interface EditorConfig {
  colorKeyframes: ColorKeyframe[];
  ledKeyframes: LedKeyframe[];
  /** Applied to the spectrum before anything else looks at it. */
  eq: EqBand[];
  /** Play the strip back to front. */
  reverse: boolean;
  /** Fold the whole range into each half of the strip. */
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
  /** Reach of each colour keyframe's influence — the surface's sigma. */
  blend: number;
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

let counter = 0;
export function nextId(prefix: string): string {
  counter += 1;
  return `${prefix}-${Date.now().toString(36)}-${counter}`;
}

/**
 * Seeded from the surface the engine ships with, so a fresh editor previews
 * what the strip is already doing rather than a blank field.
 */
export const DEFAULT_CONFIG: EditorConfig = {
  colorKeyframes: DEFAULT_SURFACE.keyframes.map((k) => ({
    id: nextId("ck"),
    hz: normToHz(k.x),
    db: normToDb(k.y),
    color: k.color,
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
  blend: DEFAULT_SURFACE.sigma,
  gizmos: { thresholds: true, colorKeyframes: true, ledKeyframes: true, curve: true, eq: true },
};

/**
 * Colour keyframes in the form the renderer samples: normalised over the unit
 * square, with x on the log frequency axis and y the band's level.
 */
export function toSurface(config: EditorConfig): SurfaceConfig {
  return {
    sigma: config.blend,
    keyframes: config.colorKeyframes.map((k) => ({
      x: hzToNorm(k.hz),
      y: dbToNorm(k.db),
      color: k.color,
    })),
  };
}

/** Everything the engine acts on. Gizmo visibility stays here, as it should. */
export function toEngineConfig(config: EditorConfig): ShowConfig {
  return {
    surface: toSurface(config),
    eq: config.eq,
    ledKeyframes: config.ledKeyframes,
    reverse: config.reverse,
    mirror: config.mirror,
    threshold: config.threshold,
    clamp: config.clamp,
    curve: config.curve,
    decay: config.decay,
    sampleLength: config.sampleLength,
  };
}

/** Adopt what the engine is running, leaving editor-only settings untouched. */
export function fromEngineConfig(config: EditorConfig, show: ShowConfig): EditorConfig {
  return {
    ...config,
    blend: show.surface.sigma,
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
    reverse: show.reverse,
    mirror: show.mirror,
    threshold: show.threshold,
    clamp: show.clamp,
    curve: show.curve,
    decay: show.decay,
    sampleLength: show.sampleLength,
  };
}

const STORAGE_KEY = "djled.editor";

export function loadConfig(): EditorConfig {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_CONFIG;
    const parsed = JSON.parse(raw) as Partial<EditorConfig>;
    // A hand-edited or stale preset must not brick the editor, so anything
    // missing or structurally wrong falls back field by field.
    return {
      ...DEFAULT_CONFIG,
      ...parsed,
      colorKeyframes: Array.isArray(parsed.colorKeyframes) && parsed.colorKeyframes.length
        ? parsed.colorKeyframes
        : DEFAULT_CONFIG.colorKeyframes,
      ledKeyframes: Array.isArray(parsed.ledKeyframes) && parsed.ledKeyframes.length
        ? parsed.ledKeyframes
        : DEFAULT_CONFIG.ledKeyframes,
      // Empty is a legitimate EQ, so unlike the keyframes this only guards
      // against the field being absent or the wrong shape.
      eq: Array.isArray(parsed.eq) ? parsed.eq : DEFAULT_CONFIG.eq,
      curve: { ...DEFAULT_CONFIG.curve, ...parsed.curve },
      gizmos: { ...DEFAULT_CONFIG.gizmos, ...parsed.gizmos },
      threshold: clampDb(parsed.threshold, DEFAULT_CONFIG.threshold),
      clamp: clampDb(parsed.clamp, DEFAULT_CONFIG.clamp),
    };
  } catch {
    return DEFAULT_CONFIG;
  }
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

function clampDb(value: unknown, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(DB_MAX, Math.max(DB_MIN, value));
}
