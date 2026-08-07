import { useCallback, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import {
  ActionIcon,
  Box,
  ColorPicker,
  ColorSwatch,
  Divider,
  Group,
  NumberInput,
  Popover,
  Select,
  Slider,
  Stack,
  Text,
  Tooltip,
  useMantineTheme,
} from "@mantine/core";
import { useElementSize, useHotkeys, useMergedRef, useWindowEvent } from "@mantine/hooks";

import { Field } from "../components/Field";

import { oklabToHex } from "../color/oklab";
import { ColorSurface } from "../color/surface";
import {
  nextId,
  toSurface,
  type ColorKeyframe,
  type GizmoFlags,
  type LedKeyframe,
  type SpectrumState,
} from "../config/layers";
import {
  EQ_Q_MAX,
  EQ_Q_MIN,
  EQ_RANGE_DB,
  EQ_TYPE_OPTIONS,
  hasGain,
  type EqBand,
  type EqType,
} from "../config/eq";
import {
  DB_MAX,
  DB_MIN,
  clamp,
  dbToNorm,
  formatHz,
  formatNote,
  hzToNorm,
  hzToNote,
  snapHz,
} from "../config/scales";
import { SpectrumCanvas } from "./SpectrumCanvas";
import { GizmoLayer, type GizmoTarget } from "./GizmoLayer";
import {
  computeLayout,
  curveBox,
  dbOfY,
  gainOfY,
  hzOfX,
  xOfHz,
  yOfDb,
  yOfGain,
  type PlotLayout,
} from "./layout";
import { dbToLevel, type SpectrumFrame } from "./paint";

const PLOT_HEIGHT = 400;

/** The clamp must stay above the threshold, or the curve has no domain. */
const MIN_WINDOW_DB = 6;

const MIN_COLOR_KEYFRAMES = 2;
const MIN_LED_KEYFRAMES = 2;

interface Props {
  state: SpectrumState;
  onChange: (state: SpectrumState) => void;
  /** View-only, and passed apart from the state so it is never animated. */
  gizmos: GizmoFlags;
  frame: SpectrumFrame;
  ledCount: number;
  sampleRate: number;
}

/** Where the pointer was, relative to the thing it grabbed. */
interface Grab {
  target: GizmoTarget;
  dx: number;
  dy: number;
}

export function SpectrumEditor({
  state,
  onChange,
  gizmos,
  frame,
  ledCount,
  sampleRate,
}: Props) {
  const theme = useMantineTheme();
  const palette = theme.other.plot;

  const sized = useElementSize<HTMLDivElement>();
  const boxRef = useRef<HTMLDivElement>(null);
  const containerRef = useMergedRef(sized.ref, boxRef);

  const [selected, setSelected] = useState<GizmoTarget | null>(null);
  const [grab, setGrab] = useState<Grab | null>(null);

  const layout = useMemo(() => computeLayout(sized.width || 900, PLOT_HEIGHT), [sized.width]);
  const surface = useMemo(() => toSurface(state), [state]);
  const clearSelection = useCallback(() => setSelected(null), []);

  const localPoint = useCallback((event: { clientX: number; clientY: number }) => {
    const rect = boxRef.current?.getBoundingClientRect();
    return rect
      ? { x: event.clientX - rect.left, y: event.clientY - rect.top }
      : { x: 0, y: 0 };
  }, []);

  const onGrabTarget = useCallback(
    (target: GizmoTarget, event: ReactPointerEvent) => {
      if (event.button !== 0) return;
      event.stopPropagation();
      const p = localPoint(event);
      setGrab({ target, ...grabOffset(target, state, layout, p) });
      const selectable = target.kind === "color" || target.kind === "led" || target.kind === "eq";
      setSelected(selectable ? target : null);
    },
    [state,layout, localPoint],
  );

  useWindowEvent("pointermove", (event: PointerEvent) => {
    if (!grab) return;
    const p = localPoint(event);
    onChange(applyDrag(state, layout, grab, p.x, p.y));
  });

  useWindowEvent("pointerup", () => setGrab(null));

  const removeSelected = useCallback(() => {
    if (!selected) return;
    if (selected.kind === "color" && state.colorKeyframes.length > MIN_COLOR_KEYFRAMES) {
      onChange({
        ...state,
        colorKeyframes: state.colorKeyframes.filter((k) => k.id !== selected.id),
      });
      setSelected(null);
    } else if (selected.kind === "led" && state.ledKeyframes.length > MIN_LED_KEYFRAMES) {
      onChange({ ...state,ledKeyframes: state.ledKeyframes.filter((k) => k.id !== selected.id) });
      setSelected(null);
    } else if (selected.kind === "eq") {
      // No minimum: an EQ with no bands is flat, which is a valid state.
      onChange({ ...state, eq:state.eq.filter((b) => b.id !== selected.id) });
      setSelected(null);
    }
  }, [state,onChange, selected]);

  useHotkeys([
    ["Delete", removeSelected],
    ["Backspace", removeSelected],
    ["Escape", () => setSelected(null)],
  ]);

  const addColor = useCallback(
    (event: ReactMouseEvent) => {
      event.preventDefault();
      const p = localPoint(event);
      const hz = snapHz(hzOfX(layout, p.x));
      const db = Math.round(dbOfY(layout, p.y));
      // Seeded with the colour already showing there, so dropping a keyframe
      // never makes the field jump before it has been given a colour.
      const sampled = new ColorSurface(toSurface(state)).sample(hzToNorm(hz), dbToLevel(db));
      const keyframe: ColorKeyframe = {
        id: nextId("ck"),
        hz,
        db,
        color: oklabToHex(sampled),
      };
      onChange({ ...state,colorKeyframes: [...state.colorKeyframes, keyframe] });
      setSelected({ kind: "color", id: keyframe.id });
    },
    [state,layout, localPoint, onChange],
  );

  const addLed = useCallback(
    (event: ReactMouseEvent) => {
      event.preventDefault();
      const p = localPoint(event);
      const hz = snapHz(hzOfX(layout, p.x));
      const keyframe: LedKeyframe = {
        id: nextId("led"),
        hz,
        led: suggestLed(state.ledKeyframes, hz, ledCount),
      };
      onChange({ ...state,ledKeyframes: sortByHz([...state.ledKeyframes, keyframe]) });
      setSelected({ kind: "led", id: keyframe.id });
    },
    [state,layout, ledCount, localPoint, onChange],
  );

  const addEq = useCallback(
    (event: ReactMouseEvent) => {
      event.preventDefault();
      const p = localPoint(event);
      const band: EqBand = {
        id: nextId("eq"),
        type: "peak",
        hz: snapHz(hzOfX(layout, p.x)),
        // Placed at the height it was dropped, so a double-click high on the
        // plot is already a boost rather than a no-op the user has to then drag.
        gain: Math.round(gainOfY(layout, p.y) * 2) / 2,
        q: 1,
      };
      onChange({ ...state, eq:sortByHz([...state.eq, band]) });
      setSelected({ kind: "eq", id: band.id });
    },
    [state,layout, localPoint, onChange],
  );

  const updateEq = useCallback(
    (id: string, patch: Partial<EqBand>) => {
      onChange({ ...state, eq:state.eq.map((b) => (b.id === id ? { ...b, ...patch } : b)) });
    },
    [state,onChange],
  );

  const selectedColor =
    selected?.kind === "color"
      ? state.colorKeyframes.find((k) => k.id === selected.id)
      : undefined;
  const selectedLed =
    selected?.kind === "led" ? state.ledKeyframes.find((k) => k.id === selected.id) : undefined;
  const selectedEq =
    selected?.kind === "eq" ? state.eq.find((b) => b.id === selected.id) : undefined;

  const anchor = selectedColor
    ? { x: xOfHz(layout, selectedColor.hz), y: yOfDb(layout, selectedColor.db) }
    : selectedLed
      ? { x: xOfHz(layout, selectedLed.hz), y: layout.track.y + 14 }
      : selectedEq
        ? {
            x: xOfHz(layout, selectedEq.hz),
            y: yOfGain(layout, hasGain(selectedEq.type) ? selectedEq.gain : 0),
          }
        : null;

  return (
    <Box
      ref={containerRef}
      style={{ position: "relative", width: "100%", height: layout.height }}
    >
      <SpectrumCanvas layout={layout} surface={surface} frame={frame} />
      <GizmoLayer
        layout={layout}
        state={state}
        gizmos={gizmos}
        palette={palette}
        sampleRate={sampleRate}
        selected={selected}
        active={grab?.target ?? null}
        onGrab={onGrabTarget}
        onAddColor={addColor}
        onAddLed={addLed}
        onAddEq={addEq}
        onClearSelection={clearSelection}
      />

      {/* A zero-size anchor, so the popover tracks the gizmo it belongs to. */}
      <Popover
        opened={anchor !== null && grab === null}
        onDismiss={() => setSelected(null)}
        position={selectedLed ? "bottom" : "top"}
        withArrow
      >
        <Popover.Target>
          <div
            style={{
              position: "absolute",
              left: anchor?.x ?? 0,
              top: anchor?.y ?? 0,
              width: 1,
              height: 1,
            }}
          />
        </Popover.Target>
        <Popover.Dropdown p="sm">
          {selectedColor && (
            <Stack gap="xs">
              <Group justify="space-between" gap="xs">
                <Group gap={6}>
                  <ColorSwatch color={selectedColor.color} size={16} />
                  <Text size="xs" ff="monospace">
                    {state.source === "midi"
                      ? `${formatNote(hzToNote(selectedColor.hz))} · vel ${Math.round(
                          dbToNorm(selectedColor.db) * 127,
                        )}`
                      : `${formatHz(selectedColor.hz)} Hz · ${selectedColor.db.toFixed(0)} dB`}
                  </Text>
                </Group>
                <Tooltip label="Delete (Del)">
                  <ActionIcon
                    color="red"
                    aria-label="Delete keyframe"
                    disabled={state.colorKeyframes.length <= MIN_COLOR_KEYFRAMES}
                    onClick={removeSelected}
                  >
                    <TrashIcon />
                  </ActionIcon>
                </Tooltip>
              </Group>
              <Divider />
              <ColorPicker
                value={selectedColor.color}
                onChange={(color) =>
                  onChange({
                    ...state,
                    colorKeyframes: state.colorKeyframes.map((k) =>
                      k.id === selectedColor.id ? { ...k, color } : k,
                    ),
                  })
                }
                swatches={SWATCHES}
                swatchesPerRow={8}
                fullWidth
              />
            </Stack>
          )}

          {selectedLed && (
            <Stack gap="xs" w={190}>
              <Group justify="space-between" gap="xs">
                <Text size="xs" c="dimmed" tt="uppercase" fw={700}>
                  LED keyframe
                </Text>
                <Tooltip label="Delete (Del)">
                  <ActionIcon
                    color="red"
                    aria-label="Delete LED keyframe"
                    disabled={state.ledKeyframes.length <= MIN_LED_KEYFRAMES}
                    onClick={removeSelected}
                  >
                    <TrashIcon />
                  </ActionIcon>
                </Tooltip>
              </Group>
              <NumberInput
                label="Index"
                min={0}
                max={Math.max(0, ledCount - 1)}
                value={selectedLed.led}
                onChange={(value) =>
                  onChange({
                    ...state,
                    ledKeyframes: state.ledKeyframes.map((k) =>
                      k.id === selectedLed.id
                        ? { ...k, led: clamp(Number(value) || 0, 0, ledCount - 1) }
                        : k,
                    ),
                  })
                }
              />
              <NumberInput
                label="Frequency"
                suffix=" Hz"
                min={20}
                max={20_000}
                value={Math.round(selectedLed.hz)}
                onChange={(value) =>
                  onChange({
                    ...state,
                    ledKeyframes: sortByHz(
                      state.ledKeyframes.map((k) =>
                        k.id === selectedLed.id
                          ? { ...k, hz: clamp(Number(value) || 20, 20, 20_000) }
                          : k,
                      ),
                    ),
                  })
                }
              />
            </Stack>
          )}

          {selectedEq && (
            <Stack gap="xs" w={210}>
              <Group justify="space-between" gap="xs">
                <Text size="xs" c="dimmed" tt="uppercase" fw={700}>
                  EQ band
                </Text>
                <Tooltip label="Delete (Del)">
                  <ActionIcon color="red" aria-label="Delete EQ band" onClick={removeSelected}>
                    <TrashIcon />
                  </ActionIcon>
                </Tooltip>
              </Group>
              <Select
                label="Type"
                data={EQ_TYPE_OPTIONS}
                value={selectedEq.type}
                allowDeselect={false}
                comboboxProps={{ withinPortal: true }}
                onChange={(value) => updateEq(selectedEq.id, { type: (value as EqType) ?? "peak" })}
              />
              <NumberInput
                label="Frequency"
                suffix=" Hz"
                min={20}
                max={20_000}
                value={Math.round(selectedEq.hz)}
                onChange={(value) =>
                  updateEq(selectedEq.id, { hz: clamp(Number(value) || 20, 20, 20_000) })
                }
              />
              {hasGain(selectedEq.type) && (
                <NumberInput
                  label="Gain"
                  suffix=" dB"
                  step={0.5}
                  decimalScale={1}
                  min={-EQ_RANGE_DB}
                  max={EQ_RANGE_DB}
                  value={selectedEq.gain}
                  onChange={(value) =>
                    updateEq(selectedEq.id, {
                      gain: clamp(Number(value) || 0, -EQ_RANGE_DB, EQ_RANGE_DB),
                    })
                  }
                />
              )}
              <Field label="Q" value={selectedEq.q.toFixed(2)}>
                <Slider
                  min={EQ_Q_MIN}
                  max={EQ_Q_MAX}
                  step={0.05}
                  value={selectedEq.q}
                  onChange={(q) => updateEq(selectedEq.id, { q })}
                  label={(v) => v.toFixed(2)}
                />
              </Field>
            </Stack>
          )}
        </Popover.Dropdown>
      </Popover>
    </Box>
  );
}

/** A starting palette for new keyframes — saturated, because LEDs are. */
const SWATCHES = [
  "#ffffff", "#ff2000", "#ff6a00", "#ffc400", "#8cff00", "#00ff5e", "#00ffd0", "#00b3ff",
  "#0040ff", "#6a00ff", "#c400ff", "#ff00a6", "#402020", "#203040", "#101010", "#000000",
];

function grabOffset(
  target: GizmoTarget,
  state: SpectrumState,
  layout: PlotLayout,
  p: { x: number; y: number },
): { dx: number; dy: number } {
  // Only the point-like gizmos need an offset; the rails snap to the pointer.
  if (target.kind === "color") {
    const k = state.colorKeyframes.find((c) => c.id === target.id);
    if (k) return { dx: xOfHz(layout, k.hz) - p.x, dy: yOfDb(layout, k.db) - p.y };
  }
  if (target.kind === "led") {
    const k = state.ledKeyframes.find((c) => c.id === target.id);
    if (k) return { dx: xOfHz(layout, k.hz) - p.x, dy: 0 };
  }
  if (target.kind === "eq") {
    const b = state.eq.find((c) => c.id === target.id);
    if (b) {
      return {
        dx: xOfHz(layout, b.hz) - p.x,
        dy: yOfGain(layout, hasGain(b.type) ? b.gain : 0) - p.y,
      };
    }
  }
  return { dx: 0, dy: 0 };
}

function applyDrag(
  state: SpectrumState,
  layout: PlotLayout,
  grab: Grab,
  px: number,
  py: number,
): SpectrumState {
  const x = px + grab.dx;
  const y = py + grab.dy;

  switch (grab.target.kind) {
    case "color": {
      const id = grab.target.id;
      const hz = snapHz(hzOfX(layout, x));
      const db = Math.round(dbOfY(layout, y) * 2) / 2;
      return {
        ...state,
        colorKeyframes: state.colorKeyframes.map((k) => (k.id === id ? { ...k, hz, db } : k)),
      };
    }
    case "led": {
      const id = grab.target.id;
      const hz = snapHz(hzOfX(layout, x));
      return {
        ...state,
        ledKeyframes: sortByHz(
          state.ledKeyframes.map((k) => (k.id === id ? { ...k, hz } : k)),
        ),
      };
    }
    case "eq": {
      const id = grab.target.id;
      const hz = snapHz(hzOfX(layout, x));
      return {
        ...state,
        eq: sortByHz(
          state.eq.map((b) => {
            if (b.id !== id) return b;
            // A pass filter has no gain to drag, so vertical movement is
            // discarded rather than silently stored and never used.
            const gain = hasGain(b.type) ? round1(gainOfY(layout, y)) : 0;
            return { ...b, hz, gain };
          }),
        ),
      };
    }
    case "threshold":
      return {
        ...state,
        threshold: clamp(round1(dbOfY(layout, y)), DB_MIN, state.clamp - MIN_WINDOW_DB),
      };
    case "clamp":
      return {
        ...state,
        clamp: clamp(round1(dbOfY(layout, y)), state.threshold + MIN_WINDOW_DB, DB_MAX),
      };
    case "curve": {
      const box = curveBox(layout, state.clamp, state.threshold);
      if (box.h <= 0 || box.w <= 0) return state;
      const point = {
        x: clamp(1 - (y - box.y) / box.h, 0, 1),
        y: clamp((x - box.x) / box.w, 0, 1),
      };
      const key = grab.target.point === 1 ? "p1" : "p2";
      return { ...state,curve: { ...state.curve, [key]: point } };
    }
  }
}

function round1(v: number): number {
  return Math.round(v * 10) / 10;
}

function sortByHz<T extends { hz: number }>(items: T[]): T[] {
  return [...items].sort((a, b) => a.hz - b.hz);
}

/**
 * A new LED keyframe should land where the existing sectors already imply,
 * so adding one splits a region rather than reshaping it.
 */
function suggestLed(keyframes: LedKeyframe[], hz: number, ledCount: number): number {
  const sorted = sortByHz(keyframes);
  if (sorted.length === 0) return 0;

  const before = [...sorted].reverse().find((k) => k.hz <= hz);
  const after = sorted.find((k) => k.hz >= hz);
  if (!before) return sorted[0].led;
  if (!after || after.hz === before.hz) return before.led;

  const t = (hzToNorm(hz) - hzToNorm(before.hz)) / (hzToNorm(after.hz) - hzToNorm(before.hz));
  return clamp(Math.round(before.led + t * (after.led - before.led)), 0, Math.max(0, ledCount - 1));
}

function TrashIcon() {
  return (
    <svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
      <path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" />
    </svg>
  );
}
