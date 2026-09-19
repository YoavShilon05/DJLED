/**
 * Everything drawn to the raster layer: the colour field, the grid, and the
 * live spectrum. Pure functions over a context — no React, no state.
 *
 * The spectrum is not painted as a solid colour. The field is laid down dimmed
 * across the whole plot, then a second time at full strength clipped to the
 * bars, so a bar reads as a *reveal* of the authored surface. That is what the
 * strip will actually do at that frequency and level, which makes the graph a
 * preview rather than a decoration.
 *
 * A flat plot — a layer listening to nothing — has nothing to reveal: there is
 * no level, so there are no bars and no quiet regions for the dimming to stand
 * for. Its lane is the field at full strength, which is the same promise kept
 * with the one thing there is to say.
 */

import { oklabToDisplayRgba } from "../color/display";
import { ColorSurface, type SurfaceConfig } from "../color/surface";
import { DB_MAX, DB_MIN, DB_TICKS } from "../config/scales";
import type { PlotPalette } from "../theme";
import type { AxisScale } from "./axis";
import { bandEdges, xOfHz, yOfDb, type PlotLayout } from "./layout";

/** The field is smooth by construction, so it is sampled coarse and upscaled. */
const FIELD_W = 160;
const FIELD_H = 96;

/** How visible the surface is where no signal is present. */
const FIELD_DIM = 0.38;

export interface SpectrumFrame {
  /** Normalised band levels, 0..1. */
  levels: number[];
  /** Band centre frequencies in Hz, same length as `levels`. */
  centers: number[];
}

/**
 * The engine reports levels already normalised, so the dB axis is a linear
 * remap of that range rather than a second log. When the protocol grows a real
 * dBFS field this is the only line that changes.
 */
export function levelToDb(level: number): number {
  return DB_MIN + level * (DB_MAX - DB_MIN);
}

export function dbToLevel(db: number): number {
  return (db - DB_MIN) / (DB_MAX - DB_MIN);
}

/**
 * Rasterises the colour surface once per edit, not once per frame — at 60 fps
 * with a dozen keyframes the Gaussian sum is the only thing here that would
 * cost anything.
 */
export class ColorField {
  private canvas = document.createElement("canvas");
  private key = "";
  /** The object the cached key was taken from, so a repaint that was not caused
   *  by an edit costs one reference comparison instead of a serialisation. */
  private source: SurfaceConfig | null = null;
  /** Whether the cached raster was drawn on a joined axis. Part of the cache
   *  key rather than of the surface, because that is where it lives on the
   *  layer — and a flip of it changes every pixel near the two edges. */
  private cycle = false;

  constructor() {
    this.canvas.width = FIELD_W;
    this.canvas.height = FIELD_H;
  }

  render(surface: SurfaceConfig, cycle = false): HTMLCanvasElement {
    // The plot repaints on every frame the engine sends, and the surface it is
    // handed is rebuilt only when the layer changes — so the common case is the
    // same object twice and never needs stringifying at all. The key stays for
    // the case that is not: a config adopted from the engine is a new object
    // holding what was already on screen.
    if (surface === this.source && cycle === this.cycle) return this.canvas;
    this.source = surface;
    this.cycle = cycle;

    const key = `${cycle}:${JSON.stringify(surface)}`;
    if (key === this.key) return this.canvas;
    this.key = key;

    const ctx = this.canvas.getContext("2d");
    if (!ctx) return this.canvas;

    const compiled = new ColorSurface(surface).cycling(cycle);
    const image = ctx.createImageData(FIELD_W, FIELD_H);
    const data = image.data;

    for (let py = 0; py < FIELD_H; py++) {
      // Image rows run top-down; the surface's y runs bottom-up.
      const y = 1 - py / (FIELD_H - 1);
      for (let px = 0; px < FIELD_W; px++) {
        // Kept translucent rather than composited onto black: the field is drawn
        // over the plot, so a faded region should read as the plot showing
        // through, which is what it will do over a layer below it.
        const [r, g, b, a] = oklabToDisplayRgba(compiled.sample(px / (FIELD_W - 1), y));
        const i = (py * FIELD_W + px) * 4;
        data[i] = r;
        data[i + 1] = g;
        data[i + 2] = b;
        data[i + 3] = a;
      }
    }

    ctx.putImageData(image, 0, 0);
    return this.canvas;
  }
}

export function paintPlot(
  ctx: CanvasRenderingContext2D,
  l: PlotLayout,
  field: HTMLCanvasElement,
  frame: SpectrumFrame,
  palette: PlotPalette,
  axis: AxisScale,
): void {
  const { plot } = l;
  ctx.clearRect(0, 0, l.width, l.height);

  ctx.save();
  ctx.beginPath();
  ctx.rect(plot.x, plot.y, plot.w, plot.h);
  ctx.clip();

  // Nothing is being revealed on a flat plot, so nothing is dimmed: the lane
  // *is* the colour, at the strength the strip will show it.
  ctx.globalAlpha = l.flat ? 1 : FIELD_DIM;
  ctx.drawImage(field, plot.x, plot.y, plot.w, plot.h);
  ctx.globalAlpha = 1;

  const bars = l.flat ? null : spectrumPath(l, frame);
  if (bars) {
    ctx.save();
    ctx.clip(bars.area);
    ctx.drawImage(field, plot.x, plot.y, plot.w, plot.h);
    ctx.restore();

    ctx.fillStyle = palette.barCap;
    ctx.fill(bars.caps);
  }

  paintGrid(ctx, l, palette, axis);
  ctx.restore();

  ctx.strokeStyle = palette.frame;
  ctx.lineWidth = 1;
  ctx.strokeRect(plot.x + 0.5, plot.y + 0.5, plot.w - 1, plot.h - 1);
}

interface Bars {
  /** Filled region under each band. */
  area: Path2D;
  /** Bright line along the top of each band. */
  caps: Path2D;
}

function spectrumPath(l: PlotLayout, frame: SpectrumFrame): Bars | null {
  const { levels, centers } = frame;
  if (levels.length === 0 || centers.length !== levels.length) return null;

  const edges = bandEdges(centers);
  const floor = l.plot.y + l.plot.h;
  const area = new Path2D();
  const caps = new Path2D();

  for (let i = 0; i < levels.length; i++) {
    const x0 = xOfHz(l, edges[i]);
    // A one-pixel gutter separates bars without needing a stroke per bar.
    const w = Math.max(1, xOfHz(l, edges[i + 1]) - x0 - 1);
    const y = yOfDb(l, levelToDb(levels[i]));
    if (y >= floor) continue;
    area.rect(x0, y, w, floor - y);
    caps.rect(x0, y, w, 2);
  }
  return { area, caps };
}

function paintGrid(
  ctx: CanvasRenderingContext2D,
  l: PlotLayout,
  palette: PlotPalette,
  axis: AxisScale,
): void {
  const { plot } = l;
  ctx.lineWidth = 1;

  ctx.strokeStyle = palette.grid;
  ctx.beginPath();
  for (const hz of axis.subticks) {
    const x = Math.round(xOfHz(l, hz)) + 0.5;
    ctx.moveTo(x, plot.y);
    ctx.lineTo(x, plot.y + plot.h);
  }
  ctx.stroke();

  ctx.strokeStyle = palette.gridStrong;
  ctx.beginPath();
  for (const { hz } of axis.ticks) {
    const x = Math.round(xOfHz(l, hz)) + 0.5;
    ctx.moveTo(x, plot.y);
    ctx.lineTo(x, plot.y + plot.h);
  }
  // No level axis, so no lines across it. Drawn on a flat plot they would all
  // land on the same row anyway — `yOfDb` collapses them.
  if (!l.flat) {
    for (const db of DB_TICKS) {
      if (db === DB_MAX || db === DB_MIN) continue; // the frame already draws these
      const y = Math.round(yOfDb(l, db)) + 0.5;
      ctx.moveTo(plot.x, y);
      ctx.lineTo(plot.x + plot.w, y);
    }
  }
  ctx.stroke();
}
