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
 *
 * # Flat
 *
 * A layer listening to nothing has no level, so the plot has no vertical axis
 * to be: the field is a colour along the strip and nothing else. That layout is
 * the same one with its height collapsed to a lane and its rail taken away —
 * there is no threshold, no clamp and no curve to put in it.
 *
 *          gutter │                 plot                   │
 *          ┌──────┼────────────────────────────────────────┐
 *          │      │  ◉         ◉              ◉            │   the colour
 *          ├──────┼────────────────────────────────────────┤
 *    axis  │      │  20  100  1k  10k                      │
 *          ├──────┼────────────────────────────────────────┤
 *    track │ LED  │   ▽0        ▽50               ▽149     │
 *          └──────┴────────────────────────────────────────┘
 *
 * {@link yOfDb} and {@link dbOfY} are where that collapse is expressed, and
 * they are the *only* place: every gizmo, drag and popover anchor already goes
 * through them, so each one lands in the lane without knowing there is a second
 * kind of plot.
 */

import { EQ_RANGE_DB } from "../config/eq";
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
  /** Threshold, clamp and the intensity curve, right of the plot. Zero width
   *  when {@link flat}, which has none of the three. */
  rail: Rect;
  /** dB labels, left of the plot. Empty of labels when {@link flat}. */
  gutter: Rect;
  /**
   * No intensity axis: the plot is a lane showing one row of the colour field.
   *
   * Set for a layer that listens to nothing. Everything that reads a dB out of
   * this layout, or writes one into it, checks this — see {@link yOfDb}.
   */
  flat: boolean;
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

export function computeLayout(width: number, plotHeight: number, flat = false): PlotLayout {
  // The rail holds the threshold, the clamp and the curve, and a flat plot has
  // none of them — so it is not merely emptied, it is given back to the plot.
  const railW = flat ? 0 : RAIL_W;
  const railGap = flat ? 0 : RAIL_GAP;
  const plotW = Math.max(MIN_PLOT_W, width - GUTTER_W - railGap - railW);
  const plot: Rect = { x: GUTTER_W, y: TOP_PAD, w: plotW, h: plotHeight };
  const axisY = TOP_PAD + plotHeight;
  return {
    width,
    height: TOP_PAD + plotHeight + AXIS_H + TRACK_H,
    plot,
    gutter: { x: 0, y: TOP_PAD, w: GUTTER_W, h: plotHeight },
    axis: { x: plot.x, y: axisY, w: plotW, h: AXIS_H },
    track: { x: plot.x, y: axisY + AXIS_H, w: plotW, h: TRACK_H },
    rail: { x: plot.x + plotW + railGap, y: TOP_PAD, w: railW, h: plotHeight },
    flat,
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

/**
 * Where a dB sits vertically — or, on a flat plot, the middle of the lane
 * whatever it is.
 *
 * The collapse lives here rather than at each call site on purpose. Keyframe
 * handles, selection rings, areas of effect, hit tests, drag offsets and the
 * popover anchor all ask this one function where a keyframe is, so a flat plot
 * places every one of them correctly without any of them being told there is
 * such a thing.
 */
export function yOfDb(l: PlotLayout, db: number): number {
  if (l.flat) return l.plot.y + l.plot.h / 2;
  const t = (clamp(db, DB_MIN, DB_MAX) - DB_MIN) / (DB_MAX - DB_MIN);
  return l.plot.y + (1 - t) * l.plot.h;
}

/**
 * The dB a vertical position stands for. Constant on a flat plot: there is no
 * level to read, so dragging up and down cannot mean anything and must not
 * quietly rewrite what it finds.
 */
export function dbOfY(l: PlotLayout, y: number): number {
  if (l.flat) return DB_MAX;
  const t = 1 - (y - l.plot.y) / l.plot.h;
  return DB_MIN + clamp(t, 0, 1) * (DB_MAX - DB_MIN);
}

/**
 * The EQ's own vertical scale, laid over the level axis.
 *
 * A gain is not a level, so it cannot share the dB axis — there is no level a
 * "+6 dB boost" belongs at. It gets the conventional treatment instead: zero at
 * the vertical centre, ±`EQ_RANGE_DB` across the full height. The two scales
 * coexist because the EQ curve is drawn in the accent colour and nothing else
 * is.
 */
export function yOfGain(l: PlotLayout, gainDb: number): number {
  const half = l.plot.h / 2;
  return l.plot.y + half - (clamp(gainDb, -EQ_RANGE_DB, EQ_RANGE_DB) / EQ_RANGE_DB) * half;
}

export function gainOfY(l: PlotLayout, y: number): number {
  const half = l.plot.h / 2;
  return clamp(((l.plot.y + half - y) / half) * EQ_RANGE_DB, -EQ_RANGE_DB, EQ_RANGE_DB);
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
