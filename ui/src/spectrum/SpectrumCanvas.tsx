import { useLayoutEffect, useMemo, useRef } from "react";
import { useMantineTheme } from "@mantine/core";

import type { SurfaceConfig } from "../color/surface";
import type { AxisScale } from "./axis";
import type { PlotLayout } from "./layout";
import { ColorField, paintPlot, type SpectrumFrame } from "./paint";

interface Props {
  layout: PlotLayout;
  surface: SurfaceConfig;
  frame: SpectrumFrame;
  axis: AxisScale;
}

/**
 * The raster half of the editor. Sits under the gizmo SVG in the same
 * coordinate space and never handles a pointer event — hit testing belongs to
 * the layer that already knows where the handles are.
 */
export function SpectrumCanvas({ layout, surface, frame, axis }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const theme = useMantineTheme();
  const field = useMemo(() => new ColorField(), []);

  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const w = Math.round(layout.width * dpr);
    const h = Math.round(layout.height * dpr);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    paintPlot(ctx, layout, field.render(surface), frame, theme.other.plot, axis);
  }, [layout, surface, frame, field, theme, axis]);

  return (
    <canvas
      ref={canvasRef}
      style={{
        position: "absolute",
        inset: 0,
        width: layout.width,
        height: layout.height,
      }}
    />
  );
}
