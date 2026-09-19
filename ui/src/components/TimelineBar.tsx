import { memo, useCallback, useEffect, useRef, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { ActionIcon, Box, Group, NumberInput, Switch, Text, Tooltip, useMantineTheme } from "@mantine/core";
import { useElementSize } from "@mantine/hooks";

import { nowSeconds } from "../color/timeline";
import { isStatic, nextId, type EditorLayer } from "../config/editor";
import {
  BASE_KEY,
  KEY_MAX,
  KEY_MIN,
  MAX_LENGTH,
  MIN_LENGTH,
  addKey,
  isAnimated,
  keysOf,
  moveKey,
  phaseAt,
  removeKey,
  withTimeline,
} from "../config/timeline";
import { clamp } from "../config/scales";
import { computeLayout } from "../spectrum/layout";

/** Height of the track the keys ride in. Enough for a marker and its label,
 *  and no more: this sits under the graph and every pixel is the graph's. */
const TRACK_H = 30;

/** Half-width of a key marker, in pixels. */
const MARK = 7;

interface Props {
  layer: EditorLayer;
  onChange: (layer: EditorLayer) => void;
  /** Which key the graph above is showing. Empty is the start of the loop. */
  activeKeyId: string;
  onSelectKey: (id: string) => void;
}

/**
 * The layer's loop: when its colour field is what, and for how long.
 *
 * The graph above shows *one* key, and this is where that key is chosen. A key
 * is the whole field at a moment, so selecting one and editing the graph is the
 * entire authoring gesture — there is no second editor and no curve sheet.
 *
 * # What it does not have
 *
 * No transport. The loop is a function of the wall clock rather than of a
 * playhead anyone owns, which is what lets the engine and this page agree on
 * the instant without exchanging it, and what keeps a preset switch from
 * lurching a show that is already up. So the head here is a readout, not a
 * control: it says where the wall is, and it is drawn by writing a transform
 * onto one element from an animation frame rather than by re-rendering
 * anything. Putting it in React state would re-render this panel sixty times a
 * second to move a line two pixels.
 *
 * # The first key
 *
 * Pinned at the start and undeleteable, because it *is* the layer's own colour
 * field — see `config/timeline.ts`. Deleting every other key therefore leaves a
 * still layer rather than an empty loop, which is the only sane way out.
 *
 * Memoised: nothing here is driven by an engine frame. See the note on `frame`
 * in `App.tsx`.
 */
export const TimelineBar = memo(function TimelineBar({
  layer,
  onChange,
  activeKeyId,
  onSelectKey,
}: Props) {
  const theme = useMantineTheme();
  const palette = theme.other.plot;

  // The track spans exactly the plot above it, measured the same way and
  // through the same function — so the two read as one block rather than as two
  // controls that happen to be stacked. The axes have nothing to do with each
  // other: one is frequency and one is time.
  const sized = useElementSize<HTMLDivElement>();
  const layout = computeLayout(sized.width || 900, 0, isStatic(layer));

  const trackRef = useRef<HTMLDivElement>(null);
  const headRef = useRef<HTMLDivElement>(null);
  const dragging = useRef<string | null>(null);

  const keys = keysOf(layer);
  const running = isAnimated(layer);
  const { length } = layer.timeline;

  const xOfAt = useCallback(
    (at: number) => layout.plot.x + at * layout.plot.w,
    [layout.plot.x, layout.plot.w],
  );

  const atOfEvent = useCallback(
    (event: { clientX: number }) => {
      const rect = trackRef.current?.getBoundingClientRect();
      if (!rect || layout.plot.w <= 0) return 0;
      return clamp((event.clientX - rect.left - layout.plot.x) / layout.plot.w, 0, 1);
    },
    [layout.plot.x, layout.plot.w],
  );

  // The head is written straight onto the element. It is the one thing on this
  // panel that moves on its own, and it must not cost a render to do it.
  useEffect(() => {
    if (!running) {
      if (headRef.current) headRef.current.style.opacity = "0";
      return;
    }
    let raf = 0;
    const tick = () => {
      raf = requestAnimationFrame(tick);
      const head = headRef.current;
      if (!head) return;
      head.style.opacity = "1";
      head.style.transform = `translateX(${xOfAt(phaseAt(layer, nowSeconds()))}px)`;
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [running, layer, xOfAt]);

  // Deliberately re-registered on every render rather than once: the handler
  // has to close over the *current* layer, and the drag target lives in a ref
  // precisely so that grabbing a key does not re-render anything. Two window
  // listeners per render of a panel that no frame drives is nothing.
  useEffect(() => {
    const move = (event: PointerEvent) => {
      if (dragging.current === null) return;
      onChange(moveKey(layer, dragging.current, atOfEvent(event)));
    };
    const up = () => {
      dragging.current = null;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  });

  const grab = useCallback(
    (id: string, event: ReactPointerEvent) => {
      event.stopPropagation();
      onSelectKey(id);
      // The first key is the layer's own field and has no position to drag.
      if (event.button === 0 && id !== BASE_KEY) dragging.current = id;
    },
    [onSelectKey],
  );

  const add = useCallback(
    (at: number) => {
      const id = nextId("key");
      onChange(addKey(layer, at, id));
      onSelectKey(id);
    },
    [layer, onChange, onSelectKey],
  );

  const addHere = useCallback(
    (event: ReactMouseEvent) => {
      event.preventDefault();
      add(atOfEvent(event));
    },
    [add, atOfEvent],
  );

  const drop = useCallback(() => {
    if (activeKeyId === BASE_KEY) return;
    onChange(removeKey(layer, activeKeyId));
    onSelectKey(BASE_KEY);
  }, [activeKeyId, layer, onChange, onSelectKey]);

  return (
    <Box ref={sized.ref}>
      <Group justify="space-between" gap="xs" wrap="nowrap" mb={4}>
        <Group gap="xs" wrap="nowrap">
          <Switch
            size="xs"
            label="Loop"
            checked={layer.timeline.enabled}
            onChange={(event) =>
              onChange(withTimeline(layer, { enabled: event.currentTarget.checked }))
            }
          />
          <NumberInput
            size="xs"
            w={104}
            suffix=" s"
            step={0.5}
            decimalScale={2}
            min={MIN_LENGTH}
            max={MAX_LENGTH}
            value={length}
            aria-label="Loop length"
            onChange={(value) =>
              onChange(
                withTimeline(layer, {
                  length: clamp(Number(value) || MIN_LENGTH, MIN_LENGTH, MAX_LENGTH),
                }),
              )
            }
          />
          <Text size="xs" c="dimmed" truncate>
            {summary(keys.length, running)}
          </Text>
        </Group>
        <Group gap={6} wrap="nowrap">
          <Tooltip label="Add a key halfway to the next one">
            <ActionIcon
              variant="default"
              aria-label="Add timeline key"
              onClick={() => add(nextGap(keys.map((k) => k.at)))}
            >
              <PlusIcon />
            </ActionIcon>
          </Tooltip>
          <Tooltip label="Delete the selected key">
            <ActionIcon
              color="red"
              aria-label="Delete timeline key"
              disabled={activeKeyId === BASE_KEY}
              onClick={drop}
            >
              <TrashIcon />
            </ActionIcon>
          </Tooltip>
        </Group>
      </Group>

      <Box
        ref={trackRef}
        onContextMenu={addHere}
        style={{
          position: "relative",
          height: TRACK_H,
          borderRadius: "var(--mantine-radius-sm)",
          background: "var(--mantine-color-default)",
          border: `1px solid ${palette.frame}`,
          cursor: "context-menu",
          opacity: layer.timeline.enabled ? 1 : 0.55,
        }}
      >
        {/* Where the wall is in the loop. Position is written from an animation
            frame; nothing about it is React state. */}
        <div
          ref={headRef}
          style={{
            position: "absolute",
            left: 0,
            top: 2,
            width: 1,
            height: TRACK_H - 4,
            background: palette.gizmoActive,
            opacity: 0,
            pointerEvents: "none",
            willChange: "transform",
          }}
        />

        {keys.map((key) => {
          const selected = key.id === activeKeyId;
          return (
            <div
              key={key.id || "start"}
              role="button"
              tabIndex={0}
              aria-label={key.base ? "Start of loop" : `Key at ${Math.round(key.at * 100)}%`}
              onPointerDown={(event) => grab(key.id, event)}
              onKeyDown={(event) => event.key === "Enter" && onSelectKey(key.id)}
              style={{
                position: "absolute",
                left: xOfAt(key.at) - MARK,
                top: (TRACK_H - MARK * 2) / 2,
                width: MARK * 2,
                height: MARK * 2,
                borderRadius: key.base ? 3 : "50%",
                background: selected ? palette.gizmoActive : palette.gizmo,
                border: `2px solid ${selected ? palette.keyframeRing : "transparent"}`,
                boxSizing: "content-box",
                cursor: key.base ? "pointer" : "ew-resize",
              }}
            />
          );
        })}
      </Box>
    </Box>
  );
});

/** What the bar says about itself, in the one line there is room for. */
function summary(keys: number, running: boolean): string {
  if (keys < 2) return "one key — a still field until a second is added";
  return running ? `${keys} keys` : `${keys} keys — held at the first`;
}

/**
 * Where the "+" button drops a key: halfway across the widest gap in the loop,
 * the wrap included.
 *
 * A key on top of an existing one would be a span of no length and an edit
 * nobody can see, so the button subdivides rather than appending. Right-clicking
 * the track is the gesture for placing one exactly.
 */
function nextGap(ats: number[]): number {
  const sorted = [...ats].sort((a, b) => a - b);
  let best = { at: (KEY_MIN + KEY_MAX) / 2, span: -1 };
  for (let i = 0; i < sorted.length; i++) {
    const from = sorted[i];
    // The last gap runs off the end and back to the start, because the loop
    // does. Without it a timeline whose keys all sit early can never be
    // subdivided across its own wrap.
    const to = i + 1 < sorted.length ? sorted[i + 1] : sorted[0] + 1;
    const span = to - from;
    if (span > best.span) best = { at: clamp((from + to) / 2, KEY_MIN, KEY_MAX), span };
  }
  return best.at;
}

function PlusIcon() {
  return (
    <svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
      <path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" />
    </svg>
  );
}
