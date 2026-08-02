/**
 * Pixel geometry for the editor block.
 *
 * The canvas, the gizmo SVG and the popover anchors all read from one layout
 * object in one coordinate space — the container's — so a gizmo in the rail can
 * line up with a dB on the plot, and an LED marker under the axis can raise a
 * line to the top of the graph, without any of them knowing about each other.
 *
 *          gutter │           plot            │gap│  rail
 *          ┌──────┼───────────────────────────┼───┼────────┐
 *      0dB │  0   │                           │   │ ┌────┐ │  clamp ▲
 *          │ -20  │   colour field + bars     │   │ │curve│ │
 *          │ -40  │                           │   │ └────┘ │  threshold ▼
 *          ├──────┼───────────────────────────┼───┼────────┤
 *    axis  │      │  20  100  1k  10k         │   │        │
 *          ├──────┼───────────────────────────┼───┼────────┤
 *    track │ LED  │   ▽0        ▽50      ▽149 │   │        │
 *          └──────┴───────────────────────────┴───┴────────┘
 */

import { DB_MAX, DB_MIN, F_MAX, F_MIN, clamp, hzToNorm, normToHz } from "../config/scales";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PlotLayout {
  width: number;
  height: number;
  /** Main graph area. */
  plot: Rect;
  /** Frequency labels, directly under the plot. */
  axis: Rect;
  /** Draggable LED keyframes, under the labels. */
  track: Rect;
  /** Threshold, clamp and the intensity curve, right of the plot. */
  rail: Rect;
  /** dB labels, left of the plot. */
  gutter: Rect;
}

const GUTTER_W = 46;
const AXIS_H = 28;
const TRACK_H = 34;
const RAIL_GAP = 12;
const RAIL_W = 118;
const MIN_PLOT_W = 240;

/**
 * Room above the plot. A keyframe authored at 0 dB sits exactly on the top
 * edge, and without this its upper half would be clipped away by the container.
 * It also gives the unit captions a row of their own.
 */
const TOP_PAD = 16;

export function computeLayout(width: number, plotHeight: number): PlotLayout {
  const plotW = Math.max(MIN_PLOT_W, width - GUTTER_W - RAIL_GAP - RAIL_W);
  const plot: Rect = { x: GUTTER_W, y: TOP_PAD, w: plotW, h: plotHeight };
  const axisY = TOP_PAD + plotHeight;
  return {
    width,
    height: TOP_PAD + plotHeight + AXIS_H + TRACK_H,
    plot,
    gutter: { x: 0, y: TOP_PAD, w: GUTTER_W, h: plotHeight },
    axis: { x: plot.x, y: axisY, w: plotW, h: AXIS_H },
    track: { x: plot.x, y: axisY + AXIS_H, w: plotW, h: TRACK_H },
    rail: { x: plot.x + plotW + RAIL_GAP, y: TOP_PAD, w: RAIL_W, h: plotHeight },
  };
}

/** Centre of the vertical lane the threshold and clamp handles ride in. */
export function railLaneX(l: PlotLayout): number {
  return l.rail.x + 9;
}

/**
 * The curve box, which spans exactly the dB window the two handles define —
 * its top edge is the clamp line and its bottom edge is the threshold line.
 * Both the gizmo that draws it and the drag maths that reads it call this.
 */
export function curveBox(l: PlotLayout, clampDb: number, thresholdDb: number): Rect {
  const top = yOfDb(l, clampDb);
  return {
    x: l.rail.x + 22,
    y: top,
    w: l.rail.w - 22,
    h: Math.max(0, yOfDb(l, thresholdDb) - top),
  };
}

export function xOfHz(l: PlotLayout, hz: number): number {
  return l.plot.x + hzToNorm(hz) * l.plot.w;
}

export function hzOfX(l: PlotLayout, x: number): number {
  return normToHz((x - l.plot.x) / l.plot.w);
}

export function yOfDb(l: PlotLayout, db: number): number {
  const t = (clamp(db, DB_MIN, DB_MAX) - DB_MIN) / (DB_MAX - DB_MIN);
  return l.plot.y + (1 - t) * l.plot.h;
}

export function dbOfY(l: PlotLayout, y: number): number {
  const t = 1 - (y - l.plot.y) / l.plot.h;
  return DB_MIN + clamp(t, 0, 1) * (DB_MAX - DB_MIN);
}

/** Distance in pixels from a point to a keyframe, for hit testing. */
export function distanceTo(l: PlotLayout, hz: number, db: number, px: number, py: number): number {
  const dx = xOfHz(l, hz) - px;
  const dy = yOfDb(l, db) - py;
  return Math.hypot(dx, dy);
}

/**
 * Band edges in Hz, midway between neighbouring centres *on the log axis* —
 * a geometric mean, not an arithmetic one, or every bar below 200 Hz would be
 * drawn off-centre from its own tick.
 *
 * The two outer edges have no neighbour to average with, so they are mirrored
 * through their centre, which keeps the first and last bars the same visual
 * width as their neighbour instead of running to the edge of the plot.
 */
export function bandEdges(centers: number[]): number[] {
  const n = centers.length;
  if (n === 0) return [];
  if (n === 1) return [F_MIN, F_MAX];

  const edges = new Array<number>(n + 1);
  for (let i = 1; i < n; i++) edges[i] = Math.sqrt(centers[i - 1] * centers[i]);
  edges[0] = (centers[0] * centers[0]) / edges[1];
  edges[n] = (centers[n - 1] * centers[n - 1]) / edges[n - 1];
  return edges;
}
