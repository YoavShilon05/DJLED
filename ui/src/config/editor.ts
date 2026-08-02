/**
 * Everything the editor holds, and the one place it is translated for the engine.
 *
 * Only the colour surface and the master brightness exist in the wire protocol
 * today. The rest — thresholds, LED regions, the intensity curve, decay, sample
 * length — is authored here and persisted here, waiting for the engine side to
 * grow the fields. Keeping the whole config in one shape now means that later
 * change is a serialiser, not a rewrite.
 */

import { DEFAULT_SURFACE, type SurfaceConfig } from "../color/surface";
import type { CurveConfig } from "./curve";
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
}

export interface EditorConfig {
  colorKeyframes: ColorKeyframe[];
  ledKeyframes: LedKeyframe[];
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

export const SAMPLE_LENGTHS = [256, 512, 1024, 2048, 4096, 8192];

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
  threshold: -62,
  clamp: -6,
  curve: { type: "linear", p1: { x: 0.25, y: 0.1 }, p2: { x: 0.25, y: 1 } },
  decay: 0.82,
  sampleLength: 2048,
  blend: DEFAULT_SURFACE.sigma,
  gizmos: { thresholds: true, colorKeyframes: true, ledKeyframes: true, curve: true },
};

/** The half of the config the engine understands today. */
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

/** Adopt a surface pushed by the engine, leaving UI-only settings untouched. */
export function withSurface(config: EditorConfig, surface: SurfaceConfig): EditorConfig {
  return {
    ...config,
    blend: surface.sigma,
    colorKeyframes: surface.keyframes.map((k) => ({
      id: nextId("ck"),
      hz: normToHz(k.x),
      db: normToDb(k.y),
      color: k.color,
    })),
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
