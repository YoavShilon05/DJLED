import { memo, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";

import type { GizmoFlags, SpectrumState } from "../config/layers";
import {
  DB_TICKS,
  FREQ_TICKS,
  NOTE_TICKS,
  dbToNorm,
  formatHz,
  formatNote,
  noteToHz,
} from "../config/scales";
import type { PlotPalette } from "../theme";
import { CurveGizmo } from "./CurveGizmo";
import { EqGizmo } from "./EqGizmo";
import { curveBox, railLaneX, xOfHz, yOfDb, type PlotLayout } from "./layout";

/** Anything that can be grabbed. Also what a selection points at. */
export type GizmoTarget =
  | { kind: "color"; id: string }
  | { kind: "led"; id: string }
  | { kind: "eq"; id: string }
  | { kind: "threshold" }
  | { kind: "clamp" }
  | { kind: "curve"; point: 1 | 2 };

export function sameTarget(a: GizmoTarget | null, b: GizmoTarget | null): boolean {
  if (!a || !b || a.kind !== b.kind) return false;
  if (a.kind === "color" || a.kind === "led" || a.kind === "eq") {
    return a.id === (b as typeof a).id;
  }
  if (a.kind === "curve") return a.point === (b as typeof a).point;
  return true;
}

/** The id in a target that carries one, or null. */
export function targetId(target: GizmoTarget | null, kind: "color" | "led" | "eq"): string | null {
  return target?.kind === kind ? target.id : null;
}

interface Props {
  layout: PlotLayout;
  state: SpectrumState;
  gizmos: GizmoFlags;
  palette: PlotPalette;
  sampleRate: number;
  selected: GizmoTarget | null;
  active: GizmoTarget | null;
  onGrab: (target: GizmoTarget, event: ReactPointerEvent) => void;
  onAddColor: (event: ReactMouseEvent) => void;
  onAddLed: (event: ReactMouseEvent) => void;
  onAddEq: (event: ReactMouseEvent) => void;
  onClearSelection: () => void;
}

const MONO = "var(--mantine-font-family-monospace)";

/**
 * Every interactive mark in the editor, in one SVG over the canvas.
 *
 * SVG rather than more canvas: hit testing, hover and focus come free, the
 * lines stay crisp at any device pixel ratio, and React can diff a dozen
 * handles far more cheaply than a 60 fps repaint can redraw them.
 *
 * The layer itself is transparent to the pointer; only the marks and the two
 * background rects opt back in.
 *
 * Memoised because the canvas beneath it repaints at frame rate and none of
 * this changes unless an edit or a resize happens.
 */
export const GizmoLayer = memo(function GizmoLayer({
  layout,
  state,
  gizmos,
  palette,
  sampleRate,
  selected,
  active,
  onGrab,
  onAddColor,
  onAddLed,
  onAddEq,
  onClearSelection,
}: Props) {
  const { plot, gutter, axis, track } = layout;

  const thresholdY = yOfDb(layout, state.threshold);
  const clampY = yOfDb(layout, state.clamp);
  const laneX = railLaneX(layout);

  // The axis is the same axis either way — a note number is log frequency
  // relabelled — so only the ruler changes, never the geometry beneath it.
  const midi = state.source === "midi";

  return (
    <svg
      width={layout.width}
      height={layout.height}
      style={{ position: "absolute", inset: 0, pointerEvents: "none", userSelect: "none" }}
    >
      {/*
        Background target. Right-click adds a colour keyframe and double-click
        adds an EQ band — the two gestures share the plot, so they cannot share
        a button, and double-click is what every EQ already uses.
      */}
      <rect
        x={plot.x}
        y={plot.y}
        width={plot.w}
        height={plot.h}
        fill="transparent"
        style={{ pointerEvents: "all", cursor: "crosshair" }}
        onContextMenu={onAddColor}
        onDoubleClick={onAddEq}
        onPointerDown={onClearSelection}
      />

      {gizmos.thresholds && (
        <rect
          x={plot.x + 1}
          y={thresholdY}
          width={plot.w - 2}
          height={Math.max(0, plot.y + plot.h - thresholdY - 1)}
          fill={palette.muteWash}
          style={{ pointerEvents: "none" }}
        />
      )}

      {gizmos.ledKeyframes &&
        state.ledKeyframes.map((k) => {
          const x = xOfHz(layout, k.hz);
          return (
            <line
              key={`ledline-${k.id}`}
              x1={x}
              y1={plot.y}
              x2={x}
              y2={track.y + 6}
              stroke={sameTarget(active, { kind: "led", id: k.id }) ? palette.gizmoActive : palette.gizmo}
              strokeWidth={1}
              strokeOpacity={0.55}
            />
          );
        })}

      {/* Under the threshold lines: the EQ fills an area, and a filled shape
          drawn last would bury them. */}
      {gizmos.eq && (
        <EqGizmo
          layout={layout}
          bands={state.eq}
          sampleRate={sampleRate}
          palette={palette}
          selectedId={targetId(selected, "eq")}
          activeId={targetId(active, "eq")}
          onGrab={(id, e) => onGrab({ kind: "eq", id }, e)}
        />
      )}

      {gizmos.thresholds && (
        <>
          <LevelLine
            y={clampY}
            layout={layout}
            palette={palette}
            label="CLAMP"
            db={state.clamp}
            active={sameTarget(active, { kind: "clamp" })}
          />
          <LevelLine
            y={thresholdY}
            layout={layout}
            palette={palette}
            label="THRESHOLD"
            db={state.threshold}
            active={sameTarget(active, { kind: "threshold" })}
          />
        </>
      )}

      {gizmos.curve && (
        <CurveGizmo
          box={curveBox(layout, state.clamp, state.threshold)}
          curve={state.curve}
          palette={palette}
          activePoint={active?.kind === "curve" ? active.point : null}
          onGrab={(point, e) => onGrab({ kind: "curve", point }, e)}
        />
      )}

      {gizmos.thresholds && (
        <>
          <RailHandle
            cx={laneX}
            cy={clampY}
            direction="up"
            palette={palette}
            active={sameTarget(active, { kind: "clamp" })}
            onPointerDown={(e) => onGrab({ kind: "clamp" }, e)}
          />
          <RailHandle
            cx={laneX}
            cy={thresholdY}
            direction="down"
            palette={palette}
            active={sameTarget(active, { kind: "threshold" })}
            onPointerDown={(e) => onGrab({ kind: "threshold" }, e)}
          />
        </>
      )}

      {gizmos.colorKeyframes &&
        state.colorKeyframes.map((k) => {
          const target: GizmoTarget = { kind: "color", id: k.id };
          const on = sameTarget(selected, target) || sameTarget(active, target);
          return (
            <g
              key={k.id}
              style={{ pointerEvents: "all", cursor: "grab" }}
              onPointerDown={(e) => onGrab(target, e)}
            >
              {on && (
                <circle
                  cx={xOfHz(layout, k.hz)}
                  cy={yOfDb(layout, k.db)}
                  r={12}
                  fill="none"
                  stroke={palette.gizmoActive}
                  strokeWidth={1.5}
                />
              )}
              <circle
                cx={xOfHz(layout, k.hz)}
                cy={yOfDb(layout, k.db)}
                r={on ? 8 : 7}
                fill={k.color}
                stroke={palette.keyframeRing}
                strokeWidth={2}
              />
              <circle
                cx={xOfHz(layout, k.hz)}
                cy={yOfDb(layout, k.db)}
                r={on ? 9.5 : 8.5}
                fill="none"
                stroke={palette.gizmo}
                strokeOpacity={0.65}
              />
            </g>
          );
        })}

      {/*
        Axis captions. Both units live in the gutter, on their own row: the plot
        corners are exactly where the default keyframes sit, so a caption at the
        end of either axis would spend its life underneath one.
      */}
      <text x={gutter.w - 9} y={plot.y - 5} textAnchor="end" fontSize={9} fill={palette.tick} fontFamily={MONO}>
        {midi ? "vel" : "dB"}
      </text>
      {DB_TICKS.map((db) => (
        <text
          key={db}
          x={gutter.w - 9}
          y={clampText(yOfDb(layout, db), plot.y, plot.h)}
          textAnchor="end"
          fontSize={10}
          fill={palette.tick}
          fontFamily={MONO}
        >
          {/* Not a second scale either: velocity is the same normalised height
              the dB axis already draws, so the ticks stay exactly where they
              were and only the number printed against them changes. */}
          {midi ? Math.round(dbToNorm(db) * 127) : db}
        </text>
      ))}

      <text
        x={gutter.w - 9}
        y={axis.y + 18}
        textAnchor="end"
        fontSize={9}
        fill={palette.tick}
        fontFamily={MONO}
      >
        {midi ? "note" : "Hz"}
      </text>
      {midi
        ? NOTE_TICKS.map((note) => (
            <text
              key={note}
              x={xOfHz(layout, noteToHz(note))}
              y={axis.y + 18}
              textAnchor="middle"
              fontSize={10}
              fill={palette.tick}
              fontFamily={MONO}
            >
              {formatNote(note)}
            </text>
          ))
        : FREQ_TICKS.map((hz) => (
            <text
              key={hz}
              x={xOfHz(layout, hz)}
              y={axis.y + 18}
              textAnchor="middle"
              fontSize={10}
              fill={palette.tick}
              fontFamily={MONO}
            >
              {formatHz(hz)}
            </text>
          ))}

      {/* LED sector track. */}
      <rect
        x={track.x}
        y={track.y + 4}
        width={track.w}
        height={track.h - 8}
        rx={4}
        fill="rgba(203, 210, 222, 0.04)"
        stroke={palette.frame}
        style={{ pointerEvents: gizmos.ledKeyframes ? "all" : "none", cursor: "crosshair" }}
        onContextMenu={onAddLed}
        onPointerDown={onClearSelection}
      />
      {gizmos.ledKeyframes &&
        state.ledKeyframes.map((k) => {
          const target: GizmoTarget = { kind: "led", id: k.id };
          const on = sameTarget(selected, target) || sameTarget(active, target);
          const x = xOfHz(layout, k.hz);
          const top = track.y + 6;
          return (
            <g
              key={k.id}
              style={{ pointerEvents: "all", cursor: "ew-resize" }}
              onPointerDown={(e) => onGrab(target, e)}
            >
              <polygon
                points={`${x},${top} ${x - 6},${top + 6} ${x + 6},${top + 6}`}
                fill={on ? palette.gizmoActive : palette.gizmo}
              />
              <rect
                x={x - 16}
                y={top + 6}
                width={32}
                height={15}
                rx={3}
                fill={on ? palette.gizmoActive : palette.gizmo}
              />
              <text
                x={x}
                y={top + 17}
                textAnchor="middle"
                fontSize={10}
                fontFamily={MONO}
                fill={palette.keyframeRing}
              >
                {k.led}
              </text>
            </g>
          );
        })}
    </svg>
  );
});

/** Keeps the first and last dB labels inside the plot instead of straddling it. */
function clampText(y: number, top: number, height: number): number {
  return Math.min(top + height - 2, Math.max(top + 9, y + 3.5));
}

interface LevelLineProps {
  y: number;
  layout: PlotLayout;
  palette: PlotPalette;
  label: string;
  db: number;
  active: boolean;
}

function LevelLine({ y, layout, palette, label, db, active }: LevelLineProps) {
  const color = active ? palette.gizmoActive : palette.gizmo;
  return (
    <g style={{ pointerEvents: "none" }}>
      <line
        x1={layout.plot.x}
        y1={y}
        x2={railLaneX(layout)}
        y2={y}
        stroke={color}
        strokeWidth={1}
        strokeOpacity={active ? 1 : 0.75}
      />
      <text
        x={layout.plot.x + layout.plot.w - 8}
        y={y - 6}
        textAnchor="end"
        fontSize={9.5}
        fill={color}
        fillOpacity={0.85}
        fontFamily={MONO}
        style={{ letterSpacing: "0.06em" }}
      >
        {label} {db.toFixed(1)}
      </text>
    </g>
  );
}

interface RailHandleProps {
  cx: number;
  cy: number;
  direction: "up" | "down";
  palette: PlotPalette;
  active: boolean;
  onPointerDown: (event: ReactPointerEvent) => void;
}

/**
 * The chevron points the way the handle acts: the clamp saturates everything
 * above it, the threshold mutes everything below.
 */
function RailHandle({ cx, cy, direction, palette, active, onPointerDown }: RailHandleProps) {
  const fill = active ? palette.gizmoActive : palette.gizmo;
  const tip = direction === "up" ? cy - 3.5 : cy + 3.5;
  const base = direction === "up" ? cy + 2.5 : cy - 2.5;
  return (
    <g style={{ pointerEvents: "all", cursor: "ns-resize" }} onPointerDown={onPointerDown}>
      <rect x={cx - 9} y={cy - 7} width={18} height={14} rx={3} fill={fill} />
      <polygon
        points={`${cx},${tip} ${cx - 4},${base} ${cx + 4},${base}`}
        fill={palette.keyframeRing}
      />
    </g>
  );
}
