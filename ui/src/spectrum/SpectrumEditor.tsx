import { useCallback, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import {
  ActionIcon,
  Box,
  Checkbox,
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

import { hexToLinearRgb, oklabToHex } from "../color/oklab";
import { ColorSurface } from "../color/surface";
import {
  isStatic,
  nextId,
  toRenderSurface,
  RADIUS_DEFAULT,
  RADIUS_MAX,
  RADIUS_MIN,
  STATIC_DB,
  type ColorKeyframe,
  type EditorLayer,
  type GizmoFlags,
  type LedKeyframe,
} from "../config/editor";
import {
  EQ_Q_MAX,
  EQ_Q_MIN,
  EQ_RANGE_DB,
  EQ_TYPE_OPTIONS,
  hasGain,
  type EqBand,
  type EqType,
} from "../config/eq";
import { DB_MAX, DB_MIN, clamp, hzToNorm, snapHz } from "../config/scales";
import { SpectrumCanvas } from "./SpectrumCanvas";
import type { AxisScale } from "./axis";
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

/**
 * The plot's height for a layer that listens to nothing.
 *
 * A lane rather than a graph, because there is nothing to graph: no level means
 * no vertical axis, so the field is a colour along the strip. Tall enough for a
 * keyframe handle, its selection ring and an area of effect to be readable, and
 * no taller — the height a still layer does not need is height every other
 * panel on the screen does.
 */
const FLAT_PLOT_HEIGHT = 96;

/** The clamp must stay above the threshold, or the curve has no domain. */
const MIN_WINDOW_DB = 6;

/**
 * Sectors, unlike colours, have a floor: one of them defines no span at all, so
 * a layer with fewer than two reaches no LED and the engine substitutes a
 * spanning layout behind the editor's back. A colour field has no such floor —
 * an empty one is a layer that paints nothing, which is a thing to author.
 */
const MIN_LED_KEYFRAMES = 2;

interface Props {
  /**
   * The layer being edited.
   *
   * The plot shows one layer at a time, deliberately. Every gizmo on it — the
   * keyframes, the sectors, the EQ, the two rail handles and the curve — is a
   * property of a single layer, and six stacks of them on one graph would be
   * unreadable and unclickable. The composited result of the whole stack is
   * what the strip preview underneath is for.
   */
  layer: EditorLayer;
  onChange: (layer: EditorLayer) => void;
  /** Overlay visibility. Editor-wide rather than per layer. */
  gizmos: GizmoFlags;
  frame: SpectrumFrame;
  /** What the horizontal marks are called. The axis and every gizmo on it stay
   *  in Hz; only the labels change when MIDI is driving the strip. */
  axis: AxisScale;
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
  layer,
  onChange,
  gizmos,
  frame,
  axis,
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

  const flat = isStatic(layer);
  const layout = useMemo(
    () => computeLayout(sized.width || 900, flat ? FLAT_PLOT_HEIGHT : PLOT_HEIGHT, flat),
    [sized.width, flat],
  );
  // The surface as it is read rather than as it is stored: a still layer's
  // field is flattened onto the one row that is ever sampled, so the lane shows
  // the colour the wall will show. See `toRenderSurface`.
  const surface = useMemo(() => toRenderSurface(layer), [layer]);

  /**
   * What the raster is drawn from.
   *
   * The channel palette rides with the field because a MIDI bar does not reveal
   * the authored field — it reveals the field as the channel that played the
   * note paints it, which is what that note will do on the strip. Undefined for
   * anything not listening to MIDI, so every other layer draws exactly what it
   * drew before.
   */
  const look = useMemo(
    () => ({
      surface,
      cycle: layer.cycle,
      channelColors: layer.source.kind === "midi" ? layer.midi.channelColors : undefined,
    }),
    [surface, layer.cycle, layer.source.kind, layer.midi.channelColors],
  );
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
      setGrab({ target, ...grabOffset(target, layer, layout, p) });
      const selectable = target.kind === "color" || target.kind === "led" || target.kind === "eq";
      setSelected(selectable ? target : null);
    },
    [layer, layout, localPoint],
  );

  useWindowEvent("pointermove", (event: PointerEvent) => {
    if (!grab) return;
    const p = localPoint(event);
    onChange(applyDrag(layer, layout, grab, p.x, p.y));
  });

  useWindowEvent("pointerup", () => setGrab(null));

  const removeSelected = useCallback(() => {
    if (!selected) return;
    if (selected.kind === "color") {
      // No minimum: a field with nothing in it paints nothing, and nothing is
      // transparent rather than black, so the layers below still show.
      onChange({
        ...layer,
        colorKeyframes: layer.colorKeyframes.filter((k) => k.id !== selected.id),
      });
      setSelected(null);
    } else if (selected.kind === "led" && layer.ledKeyframes.length > MIN_LED_KEYFRAMES) {
      onChange({ ...layer, ledKeyframes: layer.ledKeyframes.filter((k) => k.id !== selected.id) });
      setSelected(null);
    } else if (selected.kind === "eq") {
      // No minimum: an EQ with no bands is flat, which is a valid state.
      onChange({ ...layer, eq: layer.eq.filter((b) => b.id !== selected.id) });
      setSelected(null);
    }
  }, [layer, onChange, selected]);

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
      // A flat plot has no height to read a level off, and `dbOfY` says so —
      // a keyframe dropped on one is authored at the row that is sampled.
      const db = layout.flat ? STATIC_DB : Math.round(dbOfY(layout, p.y));
      // Seeded with the colour already showing there, so dropping a keyframe
      // never makes the field jump before it has been given a colour.
      const sampled = new ColorSurface(toRenderSurface(layer))
        .cycling(layer.cycle)
        .sample(hzToNorm(hz), dbToLevel(db));
      const keyframe: ColorKeyframe = {
        id: nextId("ck"),
        hz,
        db,
        color: oklabToHex(sampled),
        // Unconfined, so dropping one behaves as it always has. An area of
        // effect is something you reach for, not something you inherit.
        radius: null,
      };
      onChange({ ...layer, colorKeyframes: [...layer.colorKeyframes, keyframe] });
      setSelected({ kind: "color", id: keyframe.id });
    },
    [layer, layout, localPoint, onChange],
  );

  const addLed = useCallback(
    (event: ReactMouseEvent) => {
      event.preventDefault();
      const p = localPoint(event);
      const hz = snapHz(hzOfX(layout, p.x));
      const keyframe: LedKeyframe = {
        id: nextId("led"),
        hz,
        led: suggestLed(layer.ledKeyframes, hz, ledCount),
      };
      onChange({ ...layer, ledKeyframes: sortByHz([...layer.ledKeyframes, keyframe]) });
      setSelected({ kind: "led", id: keyframe.id });
    },
    [layer, layout, ledCount, localPoint, onChange],
  );

  const addEq = useCallback(
    (event: ReactMouseEvent) => {
      // An EQ shapes a signal, and a still layer has none. The double-click is
      // already unbound in the gizmo layer; this is the other half, so the
      // gesture cannot arrive by some other route.
      if (isStatic(layer)) return;
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
      onChange({ ...layer, eq: sortByHz([...layer.eq, band]) });
      setSelected({ kind: "eq", id: band.id });
    },
    [layer, layout, localPoint, onChange],
  );

  const updateColor = useCallback(
    (id: string, patch: Partial<ColorKeyframe>) => {
      onChange({
        ...layer,
        colorKeyframes: layer.colorKeyframes.map((k) => (k.id === id ? { ...k, ...patch } : k)),
      });
    },
    [layer, onChange],
  );

  const updateEq = useCallback(
    (id: string, patch: Partial<EqBand>) => {
      onChange({ ...layer, eq: layer.eq.map((b) => (b.id === id ? { ...b, ...patch } : b)) });
    },
    [layer, onChange],
  );

  const selectedColor =
    selected?.kind === "color"
      ? layer.colorKeyframes.find((k) => k.id === selected.id)
      : undefined;
  const selectedLed =
    selected?.kind === "led" ? layer.ledKeyframes.find((k) => k.id === selected.id) : undefined;
  const selectedEq =
    selected?.kind === "eq" ? layer.eq.find((b) => b.id === selected.id) : undefined;

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
      <SpectrumCanvas layout={layout} look={look} frame={frame} axis={axis} />
      <GizmoLayer
        layout={layout}
        layer={layer}
        gizmos={gizmos}
        palette={palette}
        axis={axis}
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
                    {axis.format(selectedColor.hz)}
                    {/* The dB is not shown on a flat plot because it is not
                        read there — quoting a level for a layer with none is
                        the one thing this readout must not do. */}
                    {!flat && ` · ${selectedColor.db.toFixed(0)} dB`}
                    {/* Only when it is not fully opaque, so the common case
                        stays uncluttered and a faded keyframe stands out. */}
                    {opacityOf(selectedColor.color) < 1 &&
                      ` · ${Math.round(opacityOf(selectedColor.color) * 100)}%`}
                  </Text>
                </Group>
                <Tooltip label="Delete (Del)">
                  <ActionIcon color="red" aria-label="Delete keyframe" onClick={removeSelected}>
                    <TrashIcon />
                  </ActionIcon>
                </Tooltip>
              </Group>
              <Divider />
              {/*
                `hexa` rather than `hex`: it adds the opacity slider and emits
                `#rrggbbaa`, which is exactly what the surface parses. Opacity
                only dims against the unlit strip today, so the swatch above and
                the field below are the honest preview of it.
              */}
              <ColorPicker
                format="hexa"
                alphaLabel="Opacity"
                value={selectedColor.color}
                onChange={(color) => updateColor(selectedColor.id, { color })}
                swatches={SWATCHES}
                swatchesPerRow={8}
                fullWidth
              />
              <Divider />
              {/*
                The checkbox is the control; the slider is only reachable once
                the answer to "how far" stops being "everywhere". Unticking
                starts from a visible area rather than from zero, so the plot
                shows a ring the moment it is switched on.
              */}
              <Checkbox
                size="xs"
                label="Infinite area of effect"
                checked={selectedColor.radius === null}
                onChange={(event) =>
                  updateColor(selectedColor.id, {
                    radius: event.currentTarget.checked ? null : RADIUS_DEFAULT,
                  })
                }
              />
              {selectedColor.radius !== null && (
                <Field
                  label="Radius"
                  value={selectedColor.radius.toFixed(2)}
                  info={
                    flat
                      ? "How far along the strip this colour reaches, as a fraction of the plot. Past it the keyframe contributes nothing and the field falls to transparent, so the layers below show through rather than black. This is not the blend radius: that decides how two keyframes that both reach a point share it, and it is one number for the whole layer."
                      : "How far this colour reaches, as a fraction of the plot. Past it the keyframe contributes nothing and the field falls to transparent, so the layers below show through rather than black. This is not the blend radius: that decides how two keyframes that both reach a point share it, and it is one number for the whole layer."
                  }
                >
                  <Slider
                    min={RADIUS_MIN}
                    max={RADIUS_MAX}
                    step={0.01}
                    value={selectedColor.radius}
                    onChange={(radius) => updateColor(selectedColor.id, { radius })}
                    label={(v) => v.toFixed(2)}
                  />
                </Field>
              )}
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
                    disabled={layer.ledKeyframes.length <= MIN_LED_KEYFRAMES}
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
                    ...layer,
                    ledKeyframes: layer.ledKeyframes.map((k) =>
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
                    ...layer,
                    ledKeyframes: sortByHz(
                      layer.ledKeyframes.map((k) =>
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

/** Opacity of an authored `#rrggbb` or `#rrggbbaa`, for the readout. */
function opacityOf(color: string): number {
  return hexToLinearRgb(color).alpha;
}

/** A starting palette for new keyframes — saturated, because LEDs are. */
const SWATCHES = [
  "#ffffff", "#ff2000", "#ff6a00", "#ffc400", "#8cff00", "#00ff5e", "#00ffd0", "#00b3ff",
  "#0040ff", "#6a00ff", "#c400ff", "#ff00a6", "#402020", "#203040", "#101010", "#000000",
];

function grabOffset(
  target: GizmoTarget,
  layer: EditorLayer,
  layout: PlotLayout,
  p: { x: number; y: number },
): { dx: number; dy: number } {
  // Only the point-like gizmos need an offset; the rails snap to the pointer.
  if (target.kind === "color") {
    const k = layer.colorKeyframes.find((c) => c.id === target.id);
    if (k) return { dx: xOfHz(layout, k.hz) - p.x, dy: yOfDb(layout, k.db) - p.y };
  }
  if (target.kind === "led") {
    const k = layer.ledKeyframes.find((c) => c.id === target.id);
    if (k) return { dx: xOfHz(layout, k.hz) - p.x, dy: 0 };
  }
  if (target.kind === "eq") {
    const b = layer.eq.find((c) => c.id === target.id);
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
  layer: EditorLayer,
  layout: PlotLayout,
  grab: Grab,
  px: number,
  py: number,
): EditorLayer {
  const x = px + grab.dx;
  const y = py + grab.dy;

  switch (grab.target.kind) {
    case "color": {
      const id = grab.target.id;
      const hz = snapHz(hzOfX(layout, x));
      return {
        ...layer,
        colorKeyframes: layer.colorKeyframes.map((k) => {
          if (k.id !== id) return k;
          // Vertical movement is discarded on a flat plot rather than rounded
          // to the constant `dbOfY` returns there. The dB is not being edited —
          // it is not being *read* — and rewriting it would quietly flatten a
          // field the moment a still layer's colours were nudged sideways, so
          // switching back to a device would find the authoring gone.
          return layout.flat ? { ...k, hz } : { ...k, hz, db: Math.round(dbOfY(layout, y) * 2) / 2 };
        }),
      };
    }
    case "led": {
      const id = grab.target.id;
      const hz = snapHz(hzOfX(layout, x));
      return {
        ...layer,
        ledKeyframes: sortByHz(
          layer.ledKeyframes.map((k) => (k.id === id ? { ...k, hz } : k)),
        ),
      };
    }
    case "eq": {
      const id = grab.target.id;
      const hz = snapHz(hzOfX(layout, x));
      return {
        ...layer,
        eq: sortByHz(
          layer.eq.map((b) => {
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
        ...layer,
        threshold: clamp(round1(dbOfY(layout, y)), DB_MIN, layer.clamp - MIN_WINDOW_DB),
      };
    case "clamp":
      return {
        ...layer,
        clamp: clamp(round1(dbOfY(layout, y)), layer.threshold + MIN_WINDOW_DB, DB_MAX),
      };
    case "curve": {
      const box = curveBox(layout, layer.clamp, layer.threshold);
      if (box.h <= 0 || box.w <= 0) return layer;
      const point = {
        x: clamp(1 - (y - box.y) / box.h, 0, 1),
        y: clamp((x - box.x) / box.w, 0, 1),
      };
      const key = grab.target.point === 1 ? "p1" : "p2";
      return { ...layer, curve: { ...layer.curve, [key]: point } };
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
