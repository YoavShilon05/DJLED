import { useCallback, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import {
  ActionIcon,
  Button,
  Group,
  NumberInput,
  Paper,
  Stack,
  Text,
  Tooltip,
  useMantineTheme,
} from "@mantine/core";
import { useElementSize, useMergedRef, useWindowEvent } from "@mantine/hooks";

import type { Layer } from "../config/layers";
import { KEY_SNAP_SECONDS, keyNear, phaseOf } from "../config/timeline";
import { clamp } from "../config/scales";
import type { PlotPalette } from "../theme";

const LANE_H = 30;
const LABEL_W = 92;

interface Props {
  layers: Layer[];
  focusedId: string | null;
  /** Seconds on the shared clock. */
  time: number;
  playing: boolean;
  duration: number;
  allLanes: boolean;
  onTime: (time: number) => void;
  onPlaying: (playing: boolean) => void;
  onDuration: (duration: number) => void;
  onAllLanes: (allLanes: boolean) => void;
  onAddKey: (layerId: string) => void;
  onRemoveKey: (layerId: string, keyId: string) => void;
  onMoveKey: (layerId: string, keyId: string, t: number) => void;
  onFocus: (layerId: string) => void;
}

/**
 * One transport, one playhead, one lane per layer.
 *
 * The clock is shared and the playhead is absolute, because there is only one
 * strip and therefore only one "now" — the composite at time *t* is a function
 * of every layer at *t*, so scrubbing is always a whole-show preview whether or
 * not it is drawn that way. Per-layer playheads would make what you are looking
 * at depend on which tab you had open, which is not reproducible.
 *
 * Each lane still spans its *own* loop rather than the transport's, so a 2 s
 * layer shows one cycle at full width just as an 8 s layer does. They stay in
 * step because both measure phase from the same t=0, not because they are the
 * same length.
 */
export function Timeline({
  layers,
  focusedId,
  time,
  playing,
  duration,
  allLanes,
  onTime,
  onPlaying,
  onDuration,
  onAllLanes,
  onAddKey,
  onRemoveKey,
  onMoveKey,
  onFocus,
}: Props) {
  const focused = layers.find((l) => l.id === focusedId) ?? null;
  const shown = allLanes ? [...layers].reverse() : focused ? [focused] : [];

  const atKey = focused
    ? keyNear(focused.keys, phaseOf(time, focused.loopSeconds))
    : null;

  return (
    <Paper>
      <Stack gap="sm">
        <Group justify="space-between" gap="xs" wrap="nowrap">
          <Group gap="xs" wrap="nowrap">
            <ActionIcon
              variant="filled"
              size="md"
              aria-label={playing ? "Pause" : "Play"}
              onClick={() => onPlaying(!playing)}
            >
              {playing ? <PauseIcon /> : <PlayIcon />}
            </ActionIcon>
            <Text size="xs" ff="monospace" c="bright" w={92}>
              {time.toFixed(2)} / {duration.toFixed(2)}s
            </Text>
            <Tooltip label="Back to start">
              <ActionIcon aria-label="Rewind" onClick={() => onTime(0)}>
                <RewindIcon />
              </ActionIcon>
            </Tooltip>
          </Group>

          <Group gap="xs" wrap="nowrap">
            {focused && (
              <>
                <Button
                  variant={atKey ? "light" : "default"}
                  leftSection={<DiamondIcon />}
                  onClick={() => onAddKey(focused.id)}
                >
                  {atKey ? "Update key" : "Add key"}
                </Button>
                <Tooltip label="Delete the key under the playhead">
                  <ActionIcon
                    color="red"
                    aria-label="Delete key"
                    disabled={!atKey}
                    onClick={() => atKey && onRemoveKey(focused.id, atKey.id)}
                  >
                    <TrashIcon />
                  </ActionIcon>
                </Tooltip>
              </>
            )}
            <NumberInput
              w={96}
              suffix=" s"
              min={0.1}
              max={600}
              step={1}
              decimalScale={2}
              value={duration}
              onChange={(v) => onDuration(Math.max(0.1, Number(v) || 0.1))}
              aria-label="Transport length"
            />
            <Button variant="default" onClick={() => onAllLanes(!allLanes)}>
              {allLanes ? "Focused lane" : "All lanes"}
            </Button>
          </Group>
        </Group>

        <Stack gap={2}>
          {shown.map((layer) => (
            <Lane
              key={layer.id}
              layer={layer}
              time={time}
              focused={layer.id === focusedId}
              showLabel={allLanes}
              onTime={onTime}
              onMoveKey={onMoveKey}
              onFocus={onFocus}
            />
          ))}
        </Stack>

        {focused && focused.keys.length === 0 && (
          <Text size="xs" c="dimmed" lh={1.35}>
            No keys — this layer holds still. Add one to capture the whole layer
            as it is now; add a second somewhere else and it tweens between them,
            wrapping back round at the end of the loop.
          </Text>
        )}
      </Stack>
    </Paper>
  );
}

interface LaneProps {
  layer: Layer;
  time: number;
  focused: boolean;
  showLabel: boolean;
  onTime: (time: number) => void;
  onMoveKey: (layerId: string, keyId: string, t: number) => void;
  onFocus: (layerId: string) => void;
}

function Lane({ layer, time, focused, showLabel, onTime, onMoveKey, onFocus }: LaneProps) {
  const theme = useMantineTheme();
  const palette: PlotPalette = theme.other.plot;

  const sized = useElementSize<HTMLDivElement>();
  const boxRef = useRef<HTMLDivElement>(null);
  const containerRef = useMergedRef(sized.ref, boxRef);
  const [dragId, setDragId] = useState<string | null>(null);

  const width = sized.width || 600;
  const trackX = showLabel ? LABEL_W : 8;
  const trackW = Math.max(40, width - trackX - 8);
  const mid = LANE_H / 2;

  const tOfX = useCallback(
    (x: number) => clamp(((x - trackX) / trackW) * layer.loopSeconds, 0, layer.loopSeconds),
    [layer.loopSeconds, trackW, trackX],
  );
  const xOfT = useCallback(
    (t: number) => trackX + (layer.loopSeconds > 0 ? t / layer.loopSeconds : 0) * trackW,
    [layer.loopSeconds, trackW, trackX],
  );

  const localX = useCallback((event: { clientX: number }) => {
    const rect = boxRef.current?.getBoundingClientRect();
    return rect ? event.clientX - rect.left : 0;
  }, []);

  useWindowEvent("pointermove", (event: PointerEvent) => {
    if (!dragId) return;
    onMoveKey(layer.id, dragId, tOfX(localX(event)));
  });

  useWindowEvent("pointerup", () => setDragId(null));

  const phase = phaseOf(time, layer.loopSeconds);
  const playheadX = xOfT(phase);

  return (
    <div ref={containerRef} style={{ width: "100%", height: LANE_H }}>
      <svg width={width} height={LANE_H} style={{ display: "block", userSelect: "none" }}>
        {showLabel && (
          <text
            x={8}
            y={mid + 3.5}
            fontSize={10}
            fill={focused ? palette.gizmo : palette.tick}
            fontFamily="var(--mantine-font-family-monospace)"
            style={{ cursor: "pointer" }}
            onClick={() => onFocus(layer.id)}
          >
            {truncate(layer.name, 12)}
          </text>
        )}

        {/* Scrub target. Clicking sets the phase directly, which is the same as
            setting the time while the transport is stopped. */}
        <rect
          x={trackX}
          y={6}
          width={trackW}
          height={LANE_H - 12}
          rx={4}
          fill="rgba(203, 210, 222, 0.04)"
          stroke={focused ? palette.frame : "transparent"}
          style={{ pointerEvents: "all", cursor: "text" }}
          onPointerDown={(e: ReactPointerEvent) => {
            if (e.button !== 0) return;
            onFocus(layer.id);
            onTime(tOfX(localX(e)));
          }}
        />

        {layer.keys.map((k) => {
          const x = xOfT(k.t);
          const on = dragId === k.id || Math.abs(k.t - phase) <= KEY_SNAP_SECONDS;
          return (
            <g
              key={k.id}
              style={{ pointerEvents: "all", cursor: "ew-resize" }}
              onPointerDown={(e: ReactPointerEvent) => {
                if (e.button !== 0) return;
                e.stopPropagation();
                onFocus(layer.id);
                setDragId(k.id);
                onTime(k.t);
              }}
            >
              <polygon
                points={`${x},${mid - 6} ${x + 5},${mid} ${x},${mid + 6} ${x - 5},${mid}`}
                fill={on ? palette.gizmoActive : palette.gizmo}
                stroke={palette.keyframeRing}
                strokeWidth={1}
              />
            </g>
          );
        })}

        <line
          x1={playheadX}
          y1={3}
          x2={playheadX}
          y2={LANE_H - 3}
          stroke={palette.gizmoActive}
          strokeWidth={1.5}
          style={{ pointerEvents: "none" }}
        />
        <polygon
          points={`${playheadX},${9} ${playheadX - 4},${2} ${playheadX + 4},${2}`}
          fill={palette.gizmoActive}
          style={{ pointerEvents: "none" }}
        />

        {!showLabel && (
          <text
            x={trackX + trackW}
            y={LANE_H - 1}
            textAnchor="end"
            fontSize={9}
            fill={palette.tick}
            fontFamily="var(--mantine-font-family-monospace)"
          >
            {layer.loopSeconds.toFixed(2)}s loop
          </text>
        )}
      </svg>
    </div>
  );
}

function truncate(s: string, n: number): string {
  return s.length <= n ? s : `${s.slice(0, n - 1)}…`;
}

function PlayIcon() {
  return (
    <svg width={14} height={14} viewBox="0 0 24 24" fill="currentColor">
      <path d="M7 4l13 8-13 8z" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg width={14} height={14} viewBox="0 0 24 24" fill="currentColor">
      <path d="M6 4h4v16H6zM14 4h4v16h-4z" />
    </svg>
  );
}

function RewindIcon() {
  return (
    <svg width={13} height={13} viewBox="0 0 24 24" fill="currentColor">
      <path d="M6 4h2v16H6zM20 4l-11 8 11 8z" />
    </svg>
  );
}

function DiamondIcon() {
  return (
    <svg width={11} height={11} viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 2l10 10-10 10L2 12z" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg width={13} height={13} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
      <path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" />
    </svg>
  );
}
