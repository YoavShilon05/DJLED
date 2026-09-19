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
import { hexToOklab, tint, type Oklab } from "../color/oklab";
import { ColorSurface, type SurfaceConfig } from "../color/surface";
import { NO_CHANNEL } from "../config/notes";
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
  /**
   * Which MIDI channel each level came from, 255 (`NO_CHANNEL`) for none.
   *
   * Absent or empty for anything with no channels to report, which is every
   * audio source and the offline demo spectrum. Read beside `levels` and
   * indexed the same way: the pair is one answer about one grid point.
   */
  channels?: number[];
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
  /**
   * One raster per tint, the untinted field under the empty key.
   *
   * A MIDI bar does not reveal the authored field — it reveals the field as its
   * channel paints it — so the plot needs the same picture once per channel
   * colour in play. They are built on demand and thrown away together whenever
   * the field changes, which keeps a drag paying for the channels actually
   * sounding rather than for all sixteen.
   */
  private rasters = new Map<string, HTMLCanvasElement>();
  private key = "";
  /** The object the cached key was taken from, so a repaint that was not caused
   *  by an edit costs one reference comparison instead of a serialisation. */
  private source: SurfaceConfig | null = null;
  /** Whether the cached raster was drawn on a joined axis. Part of the cache
   *  key rather than of the surface, because that is where it lives on the
   *  layer — and a flip of it changes every pixel near the two edges. */
  private cycle = false;

  /** The authored field. */
  render(surface: SurfaceConfig, cycle = false): HTMLCanvasElement {
    return this.raster(surface, cycle, null);
  }

  /**
   * The same field, every pixel pulled toward `color` by its own opacity — what
   * a note on that channel reveals.
   *
   * Tinted here rather than by drawing the colour over the bars, because the
   * mix is in Oklab and a canvas composite is not: at anything but full opacity
   * the two land on different colours, and this is the plot whose whole claim
   * is that a bar shows what the strip will do.
   */
  tinted(surface: SurfaceConfig, cycle: boolean, color: string): HTMLCanvasElement {
    return this.raster(surface, cycle, color);
  }

  private raster(
    surface: SurfaceConfig,
    cycle: boolean,
    color: string | null,
  ): HTMLCanvasElement {
    this.sync(surface, cycle);
    const slot = color ?? "";
    const cached = this.rasters.get(slot);
    if (cached) return cached;

    const canvas = document.createElement("canvas");
    canvas.width = FIELD_W;
    canvas.height = FIELD_H;
    this.rasters.set(slot, canvas);

    const ctx = canvas.getContext("2d");
    if (!ctx) return canvas;

    const compiled = new ColorSurface(surface).cycling(cycle);
    const over: Oklab | null = color === null ? null : hexToOklab(color);
    const image = ctx.createImageData(FIELD_W, FIELD_H);
    const data = image.data;

    for (let py = 0; py < FIELD_H; py++) {
      // Image rows run top-down; the surface's y runs bottom-up.
      const y = 1 - py / (FIELD_H - 1);
      for (let px = 0; px < FIELD_W; px++) {
        const sample = compiled.sample(px / (FIELD_W - 1), y);
        // Kept translucent rather than composited onto black: the field is drawn
        // over the plot, so a faded region should read as the plot showing
        // through, which is what it will do over a layer below it. The tint
        // keeps that alpha untouched — a channel colour says what a note looks
        // like, not what it covers.
        const [r, g, b, a] = oklabToDisplayRgba(over ? tint(sample, over) : sample);
        const i = (py * FIELD_W + px) * 4;
        data[i] = r;
        data[i + 1] = g;
        data[i + 2] = b;
        data[i + 3] = a;
      }
    }

    ctx.putImageData(image, 0, 0);
    return canvas;
  }

  /** Drop every raster if the field they were drawn from has changed. */
  private sync(surface: SurfaceConfig, cycle: boolean): void {
    // The plot repaints on every frame the engine sends, and the surface it is
    // handed is rebuilt only when the layer changes — so the common case is the
    // same object twice and never needs stringifying at all. The key stays for
    // the case that is not: a config adopted from the engine is a new object
    // holding what was already on screen.
    if (surface === this.source && cycle === this.cycle) return;
    this.source = surface;
    this.cycle = cycle;

    const key = `${cycle}:${JSON.stringify(surface)}`;
    if (key === this.key) return;
    this.key = key;
    this.rasters.clear();
  }
}

/**
 * The field this plot is drawing, and how a note on it is coloured.
 *
 * One object rather than three arguments because the three are one question —
 * what a bar of this layer looks like — and because it is memoised upstream to
 * keep the raster cache from being asked a new question thirty times a second.
 */
export interface FieldLook {
  surface: SurfaceConfig;
  /** Whether this layer's position axis is joined end to end. */
  cycle: boolean;
  /** One colour per MIDI channel, or undefined for a layer that is not
   *  listening to MIDI — see `MidiConfig.channelColors`. */
  channelColors?: string[];
}

export function paintPlot(
  ctx: CanvasRenderingContext2D,
  l: PlotLayout,
  field: ColorField,
  look: FieldLook,
  frame: SpectrumFrame,
  palette: PlotPalette,
  axis: AxisScale,
): void {
  const { plot } = l;
  const base = field.render(look.surface, look.cycle);
  ctx.clearRect(0, 0, l.width, l.height);

  ctx.save();
  ctx.beginPath();
  ctx.rect(plot.x, plot.y, plot.w, plot.h);
  ctx.clip();

  // Nothing is being revealed on a flat plot, so nothing is dimmed: the lane
  // *is* the colour, at the strength the strip will show it. The dimmed field
  // is never tinted — where no note is sounding there is no channel, and the
  // quiet parts of the plot are exactly that.
  ctx.globalAlpha = l.flat ? 1 : FIELD_DIM;
  ctx.drawImage(base, plot.x, plot.y, plot.w, plot.h);
  ctx.globalAlpha = 1;

  const bars = l.flat ? null : spectrumPath(l, frame, look.channelColors);
  if (bars) {
    // One reveal per tint rather than one for all the bars: a bar shows the
    // field as *its* channel paints it, which is what the strip will do with
    // that note. Drawn once each and never over one another, because the field
    // carries opacity and a second pass over the same pixels would double it.
    for (const [color, area] of bars.areas) {
      ctx.save();
      ctx.clip(area);
      ctx.drawImage(
        color === "" ? base : field.tinted(look.surface, look.cycle, color),
        plot.x,
        plot.y,
        plot.w,
        plot.h,
      );
      ctx.restore();
    }

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
  /**
   * The filled region under the bands, grouped by the tint they reveal the
   * field through. The empty key is the untinted field: an audio band, or a
   * note on a channel with no colour of its own.
   */
  areas: Map<string, Path2D>;
  /** Bright line along the top of each band. */
  caps: Path2D;
}

function spectrumPath(
  l: PlotLayout,
  frame: SpectrumFrame,
  channelColors?: string[],
): Bars | null {
  const { levels, centers } = frame;
  if (levels.length === 0 || centers.length !== levels.length) return null;

  const edges = bandEdges(centers);
  const floor = l.plot.y + l.plot.h;
  const areas = new Map<string, Path2D>();
  const caps = new Path2D();

  for (let i = 0; i < levels.length; i++) {
    const x0 = xOfHz(l, edges[i]);
    // A one-pixel gutter separates bars without needing a stroke per bar.
    const w = Math.max(1, xOfHz(l, edges[i + 1]) - x0 - 1);
    const y = yOfDb(l, levelToDb(levels[i]));
    if (y >= floor) continue;

    // The channel is read at the band's own index rather than looked up by
    // frequency: the level and the channel came from one note, and the bar is
    // that note. This is the one place in the editor that does not have to
    // find the nearest grid point, because it *is* the grid point.
    const area = tintOf(channelColors, frame.channels?.[i]);
    let path = areas.get(area);
    if (!path) {
      path = new Path2D();
      areas.set(area, path);
    }
    path.rect(x0, y, w, floor - y);
    caps.rect(x0, y, w, 2);
  }
  return { areas, caps };
}

/** The colour a band's bar reveals the field through, or `""` for the field
 *  itself. */
function tintOf(channelColors: string[] | undefined, channel: number | undefined): string {
  if (!channelColors || channel === undefined || channel === NO_CHANNEL) return "";
  return channelColors[channel] ?? "";
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
