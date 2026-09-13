/**
 * Painting a run of LEDs into a canvas.
 *
 * Shared by the big preview strips and the thumbnail on every layer row, which
 * are the same picture at two sizes — and, more to the point, the same hazard:
 * both are redrawn from a live frame, so both have to avoid the reallocation
 * that assigning to `canvas.width` performs even when the number has not
 * changed. At 600 LEDs and 30 fps that is megabytes a second of buffer churn
 * for a canvas whose size never moves.
 */

import type { Rgb } from "../color/display";

/** Draws `colors` edge to edge across the canvas, black where there are none. */
export function paintStrip(canvas: HTMLCanvasElement, colors: Rgb[], height: number): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  if (w === 0) return;
  const pw = Math.max(1, Math.round(w * dpr));
  const ph = Math.max(1, Math.round(height * dpr));

  // Only when it actually changed — see the note above. The repaint below
  // covers every pixel, so nothing is relying on the clear a resize would have
  // done.
  if (canvas.width !== pw || canvas.height !== ph) {
    canvas.width = pw;
    canvas.height = ph;
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, w, height);
  if (colors.length === 0) return;

  const step = w / colors.length;
  for (let i = 0; i < colors.length; i++) {
    const [r, g, b] = colors[i];
    ctx.fillStyle = `rgb(${r},${g},${b})`;
    // Overdrawn by a pixel so sub-pixel widths leave no seam between LEDs.
    ctx.fillRect(i * step, 0, step + 1, height);
  }
}
