import { useId, useMemo, type PointerEvent as ReactPointerEvent } from "react";

import { EqCurve, bandGainAt, hasGain, type EqBand } from "../config/eq";
import { F_MAX, F_MIN } from "../config/scales";
import type { PlotPalette } from "../theme";
import { hzOfX, xOfHz, yOfGain, type PlotLayout } from "./layout";

interface Props {
  layout: PlotLayout;
  bands: EqBand[];
  sampleRate: number;
  palette: PlotPalette;
  selectedId: string | null;
  activeId: string | null;
  onGrab: (id: string, event: ReactPointerEvent) => void;
}

/** One curve sample every few pixels; the response has no detail finer than that. */
const STEP_PX = 3;

/**
 * The EQ response drawn over the spectrum, with a draggable node per band.
 *
 * The curve is the *sum* of the bands in dB, which is what cascading filters
 * actually does, so overlapping bells add the way the ear expects rather than
 * the way a max() would look.
 */
export function EqGizmo({
  layout,
  bands,
  sampleRate,
  palette,
  selectedId,
  activeId,
  onGrab,
}: Props) {
  const { plot } = layout;
  const zeroY = yOfGain(layout, 0);
  // SVG ids are document-global, so a second editor on the page would otherwise
  // silently clip against the first one's rect.
  const clipId = `eq-clip-${useId()}`;

  const path = useMemo(() => {
    const curve = new EqCurve(bands, sampleRate);
    const points: string[] = [];
    for (let x = plot.x; x <= plot.x + plot.w; x += STEP_PX) {
      const y = yOfGain(layout, curve.gainAt(hzOfX(layout, x)));
      points.push(`${points.length === 0 ? "M" : "L"}${x.toFixed(1)} ${y.toFixed(2)}`);
    }
    return points.join(" ");
  }, [bands, sampleRate, layout, plot.x, plot.w]);

  // The band under the cursor gets its own response drawn, so what a handle is
  // responsible for is visible when two of them overlap.
  const soloPath = useMemo(() => {
    const id = activeId ?? selectedId;
    const band = bands.find((b) => b.id === id);
    if (!band) return null;
    const points: string[] = [];
    for (let x = plot.x; x <= plot.x + plot.w; x += STEP_PX) {
      const y = yOfGain(layout, bandGainAt(band, hzOfX(layout, x), sampleRate));
      points.push(`${points.length === 0 ? "M" : "L"}${x.toFixed(1)} ${y.toFixed(2)}`);
    }
    return points.join(" ");
  }, [bands, activeId, selectedId, sampleRate, layout, plot.x, plot.w]);

  return (
    <g>
      <clipPath id={clipId}>
        <rect x={plot.x} y={plot.y} width={plot.w} height={plot.h} />
      </clipPath>

      <g clipPath={`url(#${clipId})`}>
        {/* Unity gain. Without it a flat EQ is an unexplained horizontal line. */}
        <line
          x1={plot.x}
          y1={zeroY}
          x2={plot.x + plot.w}
          y2={zeroY}
          stroke={palette.gizmoActive}
          strokeOpacity={0.35}
          strokeDasharray="2 4"
        />

        {bands.length > 0 && (
          <>
            <path
              d={`${path} L${plot.x + plot.w} ${zeroY} L${plot.x} ${zeroY} Z`}
              fill={palette.gizmoActive}
              fillOpacity={0.12}
            />
            {soloPath && (
              <path
                d={soloPath}
                fill="none"
                stroke={palette.gizmoActive}
                strokeOpacity={0.4}
                strokeDasharray="4 3"
              />
            )}
            <path d={path} fill="none" stroke={palette.gizmoActive} strokeWidth={2} />
          </>
        )}
      </g>

      {bands.map((band) => {
        const on = band.id === selectedId || band.id === activeId;
        const cx = xOfHz(layout, band.hz);
        const cy = yOfGain(layout, hasGain(band.type) ? band.gain : 0);
        // A node dragged past the edge of the plot would be unreachable.
        if (band.hz < F_MIN * 0.99 || band.hz > F_MAX * 1.01) return null;
        return (
          <g
            key={band.id}
            style={{ pointerEvents: "all", cursor: "grab" }}
            onPointerDown={(e) => onGrab(band.id, e)}
          >
            {on && (
              <circle cx={cx} cy={cy} r={12} fill="none" stroke={palette.gizmo} strokeWidth={1} />
            )}
            <circle
              cx={cx}
              cy={cy}
              r={on ? 7.5 : 6}
              fill={palette.gizmoActive}
              stroke={palette.keyframeRing}
              strokeWidth={2}
            />
            {/* Pass filters cut instead of boosting; the slash says so at a glance. */}
            {!hasGain(band.type) && (
              <line
                x1={cx - 3}
                y1={cy + 3}
                x2={cx + 3}
                y2={cy - 3}
                stroke={palette.keyframeRing}
                strokeWidth={1.5}
              />
            )}
          </g>
        );
      })}
    </g>
  );
}
