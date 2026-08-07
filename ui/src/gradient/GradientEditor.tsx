import {
  useCallback,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  ActionIcon,
  Box,
  ColorPicker,
  ColorSwatch,
  Divider,
  Group,
  Popover,
  Stack,
  Text,
  Tooltip,
  useMantineTheme,
} from "@mantine/core";
import { useElementSize, useHotkeys, useMergedRef, useWindowEvent } from "@mantine/hooks";

import { Gradient } from "../color/gradient";
import { oklabToHex } from "../color/oklab";
import { nextId, type GradientStop, type StaticState } from "../config/layers";
import { clamp } from "../config/scales";
import { GradientCanvas } from "./GradientCanvas";

const RAMP_HEIGHT = 190;
const TRACK_H = 34;
const TOP_PAD = 16;
const GUTTER_W = 46;
const MIN_STOPS = 2;

interface Props {
  state: StaticState;
  onChange: (state: StaticState) => void;
  ledCount: number;
}

/**
 * The static layer's editor.
 *
 * Same principle as the spectrum plot: the background *is* the layer's output,
 * so what is being edited and what will be sent are the same picture rather
 * than a control panel that describes one. The x axis is strip position, which
 * is the one axis every layer kind ends up painting in — so the ramp lines up
 * with the preview strip beneath it by construction.
 *
 * Deliberately 1D. A static colour has no second dimension to react to; giving
 * it the surface's y axis would invent a variable that never moves.
 */
export function GradientEditor({ state, onChange, ledCount }: Props) {
  const theme = useMantineTheme();
  const palette = theme.other.plot;

  const sized = useElementSize<HTMLDivElement>();
  const boxRef = useRef<HTMLDivElement>(null);
  const containerRef = useMergedRef(sized.ref, boxRef);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);

  const width = sized.width || 900;
  const rampX = GUTTER_W;
  const rampW = Math.max(120, width - GUTTER_W);
  const height = TOP_PAD + RAMP_HEIGHT + TRACK_H;

  const ramp = useMemo(() => new Gradient(state.stops), [state.stops]);
  const sorted = useMemo(() => [...state.stops].sort((a, b) => a.pos - b.pos), [state.stops]);

  const xOfPos = useCallback((p: number) => rampX + p * rampW, [rampX, rampW]);
  const posOfX = useCallback(
    (x: number) => clamp((x - rampX) / rampW, 0, 1),
    [rampX, rampW],
  );

  const localX = useCallback((event: { clientX: number }) => {
    const rect = boxRef.current?.getBoundingClientRect();
    return rect ? event.clientX - rect.left : 0;
  }, []);

  useWindowEvent("pointermove", (event: PointerEvent) => {
    if (!dragId) return;
    const pos = posOfX(localX(event));
    onChange({
      ...state,
      stops: state.stops.map((s) => (s.id === dragId ? { ...s, pos } : s)),
    });
  });

  useWindowEvent("pointerup", () => setDragId(null));

  const removeSelected = useCallback(() => {
    if (!selectedId || state.stops.length <= MIN_STOPS) return;
    onChange({ ...state, stops: state.stops.filter((s) => s.id !== selectedId) });
    setSelectedId(null);
  }, [onChange, selectedId, state]);

  useHotkeys([
    ["Delete", removeSelected],
    ["Backspace", removeSelected],
    ["Escape", () => setSelectedId(null)],
  ]);

  const addStop = useCallback(
    (event: ReactMouseEvent) => {
      event.preventDefault();
      const pos = posOfX(localX(event));
      // Seeded with the colour already showing there, so dropping a stop never
      // makes the ramp jump before it has been given a colour.
      const stop: GradientStop = { id: nextId("gs"), pos, color: oklabToHex(ramp.sample(pos)) };
      onChange({ ...state, stops: [...state.stops, stop] });
      setSelectedId(stop.id);
    },
    [localX, onChange, posOfX, ramp, state],
  );

  const selected = state.stops.find((s) => s.id === selectedId) ?? null;

  return (
    <Box ref={containerRef} style={{ position: "relative", width: "100%", height }}>
      <GradientCanvas
        stops={state.stops}
        x={rampX}
        y={TOP_PAD}
        width={rampW}
        height={RAMP_HEIGHT}
        canvasWidth={width}
        canvasHeight={height}
        palette={palette}
      />

      <svg
        width={width}
        height={height}
        style={{ position: "absolute", inset: 0, pointerEvents: "none", userSelect: "none" }}
      >
        <rect
          x={rampX}
          y={TOP_PAD}
          width={rampW}
          height={RAMP_HEIGHT}
          fill="transparent"
          style={{ pointerEvents: "all", cursor: "crosshair" }}
          onContextMenu={addStop}
          onPointerDown={() => setSelectedId(null)}
        />

        <text
          x={GUTTER_W - 9}
          y={TOP_PAD - 5}
          textAnchor="end"
          fontSize={9}
          fill={palette.tick}
          fontFamily="var(--mantine-font-family-monospace)"
        >
          LED
        </text>

        {/* The span the layer actually covers, marked on the axis it is authored
            against — everything outside it is transparent, not black. */}
        <text
          x={GUTTER_W - 9}
          y={TOP_PAD + RAMP_HEIGHT + 20}
          textAnchor="end"
          fontSize={9}
          fill={palette.tick}
          fontFamily="var(--mantine-font-family-monospace)"
        >
          span
        </text>
        <line
          x1={rampX}
          y1={TOP_PAD + RAMP_HEIGHT + 16}
          x2={rampX + rampW}
          y2={TOP_PAD + RAMP_HEIGHT + 16}
          stroke={palette.frame}
        />
        <SpanLabel
          x={rampX}
          y={TOP_PAD + RAMP_HEIGHT + 20}
          anchor="start"
          text={String(Math.min(state.from, state.to))}
          fill={palette.tick}
        />
        <SpanLabel
          x={rampX + rampW}
          y={TOP_PAD + RAMP_HEIGHT + 20}
          anchor="end"
          text={`${Math.max(state.from, state.to)} of ${ledCount - 1}`}
          fill={palette.tick}
        />

        {sorted.map((stop) => {
          const on = stop.id === selectedId || stop.id === dragId;
          const x = xOfPos(stop.pos);
          return (
            <g
              key={stop.id}
              style={{ pointerEvents: "all", cursor: "ew-resize" }}
              onPointerDown={(e: ReactPointerEvent) => {
                if (e.button !== 0) return;
                e.stopPropagation();
                setDragId(stop.id);
                setSelectedId(stop.id);
              }}
            >
              <line
                x1={x}
                y1={TOP_PAD}
                x2={x}
                y2={TOP_PAD + RAMP_HEIGHT}
                stroke={on ? palette.gizmoActive : palette.gizmo}
                strokeOpacity={on ? 1 : 0.5}
              />
              {on && (
                <circle
                  cx={x}
                  cy={TOP_PAD + RAMP_HEIGHT}
                  r={12}
                  fill="none"
                  stroke={palette.gizmoActive}
                  strokeWidth={1.5}
                />
              )}
              <circle
                cx={x}
                cy={TOP_PAD + RAMP_HEIGHT}
                r={on ? 8 : 7}
                fill={stop.color}
                stroke={palette.keyframeRing}
                strokeWidth={2}
              />
              <circle
                cx={x}
                cy={TOP_PAD + RAMP_HEIGHT}
                r={on ? 9.5 : 8.5}
                fill="none"
                stroke={palette.gizmo}
                strokeOpacity={0.65}
              />
            </g>
          );
        })}
      </svg>

      {/* A zero-size anchor, so the popover tracks the stop it belongs to. */}
      <Popover
        opened={selected !== null && dragId === null}
        onDismiss={() => setSelectedId(null)}
        position="top"
        withArrow
      >
        <Popover.Target>
          <div
            style={{
              position: "absolute",
              left: selected ? xOfPos(selected.pos) : 0,
              top: TOP_PAD + RAMP_HEIGHT,
              width: 1,
              height: 1,
            }}
          />
        </Popover.Target>
        <Popover.Dropdown p="sm">
          {selected && (
            <Stack gap="xs">
              <Group justify="space-between" gap="xs">
                <Group gap={6}>
                  <ColorSwatch color={selected.color} size={16} />
                  <Text size="xs" ff="monospace">
                    LED {ledAt(selected.pos, state)} · {(selected.pos * 100).toFixed(0)}%
                  </Text>
                </Group>
                <Tooltip label="Delete (Del)">
                  <ActionIcon
                    color="red"
                    aria-label="Delete stop"
                    disabled={state.stops.length <= MIN_STOPS}
                    onClick={removeSelected}
                  >
                    <TrashIcon />
                  </ActionIcon>
                </Tooltip>
              </Group>
              <Divider />
              <ColorPicker
                value={selected.color}
                onChange={(color) =>
                  onChange({
                    ...state,
                    stops: state.stops.map((s) => (s.id === selected.id ? { ...s, color } : s)),
                  })
                }
                swatches={SWATCHES}
                swatchesPerRow={8}
                fullWidth
              />
            </Stack>
          )}
        </Popover.Dropdown>
      </Popover>
    </Box>
  );
}

/** A starting palette for new stops — saturated, because LEDs are. */
const SWATCHES = [
  "#ffffff", "#ff2000", "#ff6a00", "#ffc400", "#8cff00", "#00ff5e", "#00ffd0", "#00b3ff",
  "#0040ff", "#6a00ff", "#c400ff", "#ff00a6", "#402020", "#203040", "#101010", "#000000",
];

/** Which physical LED a position on the ramp lands on. */
function ledAt(pos: number, state: StaticState): number {
  const from = Math.min(state.from, state.to);
  const to = Math.max(state.from, state.to);
  return Math.round(from + pos * (to - from));
}

function SpanLabel({
  x,
  y,
  anchor,
  text,
  fill,
}: {
  x: number;
  y: number;
  anchor: "start" | "end";
  text: string;
  fill: string;
}) {
  return (
    <text
      x={x}
      y={y}
      textAnchor={anchor}
      fontSize={10}
      fill={fill}
      fontFamily="var(--mantine-font-family-monospace)"
    >
      {text}
    </text>
  );
}

function TrashIcon() {
  return (
    <svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
      <path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" />
    </svg>
  );
}
