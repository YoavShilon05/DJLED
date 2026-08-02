import type { PointerEvent as ReactPointerEvent } from "react";

import { controlPoints, isEditable, samplePath, type CurveConfig } from "../config/curve";
import type { PlotPalette } from "../theme";
import type { Rect } from "./layout";

interface Props {
  box: Rect;
  curve: CurveConfig;
  palette: PlotPalette;
  activePoint: 1 | 2 | null;
  onGrab: (point: 1 | 2, event: ReactPointerEvent) => void;
}

/** Below this the box is unreadable, so it is dropped rather than squashed. */
export const MIN_CURVE_HEIGHT = 52;

/**
 * Brightness against level, drawn in the rail between the clamp and threshold
 * handles.
 *
 * The vertical axis is shared with the plot on purpose: the top edge *is* the
 * clamp line and the bottom edge *is* the threshold line, so the curve can be
 * read straight across from a band. Brightness therefore runs horizontally,
 * 0 at the left, full at the right.
 */
export function CurveGizmo({ box, curve, palette, activePoint, onGrab }: Props) {
  if (box.h < MIN_CURVE_HEIGHT) return null;

  const sx = (brightness: number) => box.x + brightness * box.w;
  const sy = (level: number) => box.y + (1 - level) * box.h;

  const path = samplePath(curve)
    .map((p, i) => `${i === 0 ? "M" : "L"}${sx(p.y).toFixed(2)} ${sy(p.x).toFixed(2)}`)
    .join(" ");

  const [p1, p2] = controlPoints(curve);
  const editable = isEditable(curve.type);

  return (
    <g>
      <rect
        x={box.x}
        y={box.y}
        width={box.w}
        height={box.h}
        rx={4}
        fill="rgba(9, 11, 15, 0.5)"
        stroke={palette.frame}
      />

      {/* Diagonal reference: where a linear response would sit. */}
      <line
        x1={sx(0)}
        y1={sy(0)}
        x2={sx(1)}
        y2={sy(1)}
        stroke={palette.grid}
        strokeDasharray="3 3"
      />

      <path d={path} fill="none" stroke={palette.gizmo} strokeWidth={1.75} />

      {editable && (
        <>
          <line
            x1={sx(0)}
            y1={sy(0)}
            x2={sx(p1.y)}
            y2={sy(p1.x)}
            stroke={palette.gizmoActive}
            strokeOpacity={0.5}
          />
          <line
            x1={sx(1)}
            y1={sy(1)}
            x2={sx(p2.y)}
            y2={sy(p2.x)}
            stroke={palette.gizmoActive}
            strokeOpacity={0.5}
          />
          {([1, 2] as const).map((n) => {
            const p = n === 1 ? p1 : p2;
            return (
              <circle
                key={n}
                cx={sx(p.y)}
                cy={sy(p.x)}
                r={activePoint === n ? 6 : 4.5}
                fill={palette.gizmoActive}
                stroke={palette.keyframeRing}
                strokeWidth={1.5}
                style={{ pointerEvents: "all", cursor: "grab" }}
                onPointerDown={(e) => onGrab(n, e)}
              />
            );
          })}
        </>
      )}

      <text
        x={box.x + box.w / 2}
        y={box.y + box.h - 5}
        textAnchor="middle"
        fontSize={9}
        fill={palette.tick}
        style={{ letterSpacing: "0.08em" }}
      >
        BRIGHTNESS →
      </text>
    </g>
  );
}
