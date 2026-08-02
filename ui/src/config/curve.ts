/**
 * The intensity curve: the middleware between a band's level and the brightness
 * the LED is driven at.
 *
 * Input is the level already normalised into the threshold→clamp window, so the
 * curve is always a map of the unit square. Every type is a cubic Bézier with
 * endpoints pinned at (0,0) and (1,1) — the presets are just fixed control
 * points, which is why only `bezier` shows handles.
 */

export type CurveType = "linear" | "easeIn" | "easeOut" | "easeInOut" | "bezier";

export interface Point {
  x: number;
  y: number;
}

export interface CurveConfig {
  type: CurveType;
  /** Control points, only editable when `type` is `bezier`. */
  p1: Point;
  p2: Point;
}

export const CURVE_OPTIONS: Array<{ value: CurveType; label: string }> = [
  { value: "linear", label: "Linear" },
  { value: "easeIn", label: "Ease in" },
  { value: "easeOut", label: "Ease out" },
  { value: "easeInOut", label: "Ease in-out" },
  { value: "bezier", label: "Bézier" },
];

const PRESETS: Record<Exclude<CurveType, "bezier">, [Point, Point]> = {
  linear: [
    { x: 1 / 3, y: 1 / 3 },
    { x: 2 / 3, y: 2 / 3 },
  ],
  easeIn: [
    { x: 0.42, y: 0 },
    { x: 1, y: 1 },
  ],
  easeOut: [
    { x: 0, y: 0 },
    { x: 0.58, y: 1 },
  ],
  easeInOut: [
    { x: 0.42, y: 0 },
    { x: 0.58, y: 1 },
  ],
};

/** True only for `bezier` — the others are fixed and must not show handles. */
export function isEditable(type: CurveType): boolean {
  return type === "bezier";
}

/** The control points actually in force, preset or authored. */
export function controlPoints(curve: CurveConfig): [Point, Point] {
  return curve.type === "bezier" ? [curve.p1, curve.p2] : PRESETS[curve.type];
}

function cubic(a: number, b: number, t: number): number {
  const u = 1 - t;
  // Endpoints are 0 and 1, so their terms collapse to the t³ term alone.
  return 3 * u * u * t * a + 3 * u * t * t * b + t * t * t;
}

/**
 * Brightness for a normalised level.
 *
 * A Bézier is parametric, so x has to be inverted before y can be read. Bisection
 * rather than Newton: the derivative vanishes at the ends of `easeIn`/`easeOut`,
 * where Newton stalls, and 24 halvings is exact to well below a pixel.
 */
export function evalCurve(curve: CurveConfig, x: number): number {
  const [p1, p2] = controlPoints(curve);
  if (x <= 0) return 0;
  if (x >= 1) return 1;

  let lo = 0;
  let hi = 1;
  let t = x;
  for (let i = 0; i < 24; i++) {
    const at = cubic(p1.x, p2.x, t);
    if (at < x) lo = t;
    else hi = t;
    t = (lo + hi) / 2;
  }
  return cubic(p1.y, p2.y, t);
}

/** Points along the curve, for drawing it. */
export function samplePath(curve: CurveConfig, steps = 48): Point[] {
  const [p1, p2] = controlPoints(curve);
  const out: Point[] = [];
  for (let i = 0; i <= steps; i++) {
    const t = i / steps;
    out.push({ x: cubic(p1.x, p2.x, t), y: cubic(p1.y, p2.y, t) });
  }
  return out;
}
