import { useLayoutEffect, useMemo, useRef } from "react";

import { oklabToDisplay } from "../color/display";
import { Gradient } from "../color/gradient";
import type { GradientStop } from "../config/layers";
import type { PlotPalette } from "../theme";

interface Props {
  stops: GradientStop[];
  x: number;
  y: number;
  width: number;
  height: number;
  canvasWidth: number;
  canvasHeight: number;
  palette: PlotPalette;
}

/** Column count for the ramp. A 600-LED strip is still smooth at this width. */
const SAMPLES = 256;

/**
 * The raster half of the gradient editor.
 *
 * Painted through `oklabToDisplay` rather than a CSS `linear-gradient`, and the
 * difference is the whole point: the browser would interpolate the stops in
 * sRGB and draw a ramp the strip will not produce. Sampling the same `Gradient`
 * the compositor uses means the picture under the stops is the layer's actual
 * output, not an approximation of it.
 */
export function GradientCanvas({
  stops,
  x,
  y,
  width,
  height,
  canvasWidth,
  canvasHeight,
  palette,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const ramp = useMemo(() => new Gradient(stops), [stops]);

  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const w = Math.round(canvasWidth * dpr);
    const h = Math.round(canvasHeight * dpr);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, canvasWidth, canvasHeight);

    const step = width / SAMPLES;
    for (let i = 0; i < SAMPLES; i++) {
      const [r, g, b] = oklabToDisplay(ramp.sample(i / (SAMPLES - 1)));
      ctx.fillStyle = `rgb(${r},${g},${b})`;
      // Overdrawn by a pixel so sub-pixel widths leave no seam between columns.
      ctx.fillRect(x + i * step, y, step + 1, height);
    }

    ctx.strokeStyle = palette.frame;
    ctx.lineWidth = 1;
    ctx.strokeRect(x + 0.5, y + 0.5, width - 1, height - 1);
  }, [ramp, x, y, width, height, canvasWidth, canvasHeight, palette]);

  return (
    <canvas
      ref={canvasRef}
      style={{
        position: "absolute",
        inset: 0,
        width: canvasWidth,
        height: canvasHeight,
      }}
    />
  );
}
