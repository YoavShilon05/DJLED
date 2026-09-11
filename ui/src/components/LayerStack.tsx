import { memo, useCallback, useState } from "react";
import {
  ActionIcon,
  Badge,
  Box,
  Button,
  Group,
  Paper,
  Slider,
  Stack,
  Text,
  TextInput,
  Tooltip,
} from "@mantine/core";
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { restrictToParentElement, restrictToVerticalAxis } from "@dnd-kit/modifiers";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";

import {
  MAX_LAYERS,
  newLayer,
  nextId,
  type EditorConfig,
  type EditorLayer,
} from "../config/editor";
import type { LayerStatus } from "../engine";

interface Props {
  config: EditorConfig;
  onChange: (config: EditorConfig) => void;
  /** What each layer resolved to, keyed by id. Empty while offline. */
  status: Map<string, LayerStatus>;
}

/**
 * The stack, top layer first.
 *
 * Listed in the reverse of how it is stored, and that is not a detail: the
 * config is bottom-first because that is the order it is composited in, while
 * a stack is *read* top-down — the row at the top of the list is the one
 * painting over everything else, which is what every compositor does and what
 * anyone reaching for this expects.
 *
 * Dragging reorders. Everything else in the editor edits whichever row is
 * selected, so this is also the navigation for the whole panel.
 *
 * Memoised: nothing in here is driven by a frame, and a frame arrives thirty
 * times a second. See the note on `frame` in `App.tsx`.
 */
export const LayerStack = memo(function LayerStack({ config, onChange, status }: Props) {
  const sensors = useSensors(
    // A small distance before a drag starts, so clicking a row to select it,
    // or grabbing its opacity slider, is not read as the beginning of a drag.
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  // Top first for display; the config stays bottom first.
  const rows = [...config.layers].reverse();

  const onDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event;
      if (!over || active.id === over.id) return;

      const from = config.layers.findIndex((l) => l.id === active.id);
      const to = config.layers.findIndex((l) => l.id === over.id);
      if (from < 0 || to < 0) return;

      const layers = [...config.layers];
      const [moved] = layers.splice(from, 1);
      layers.splice(to, 0, moved);
      onChange({ ...config, layers });
    },
    [config, onChange],
  );

  const update = useCallback(
    (id: string, patch: Partial<EditorLayer>) =>
      onChange({
        ...config,
        layers: config.layers.map((l) => (l.id === id ? { ...l, ...patch } : l)),
      }),
    [config, onChange],
  );

  const add = useCallback(() => {
    if (config.layers.length >= MAX_LAYERS) return;
    const layer = newLayer(`Layer ${config.layers.length + 1}`);
    // Appended, so it lands on top — which is where a new layer goes in every
    // compositor, and where it can be seen.
    onChange({ ...config, layers: [...config.layers, layer], activeLayerId: layer.id });
  }, [config, onChange]);

  const duplicate = useCallback(
    (id: string) => {
      if (config.layers.length >= MAX_LAYERS) return;
      const index = config.layers.findIndex((l) => l.id === id);
      if (index < 0) return;
      const source = config.layers[index];
      // Every id inside the layer is minted fresh too. Sharing a keyframe id
      // between two layers would have one selection highlight both.
      const copy: EditorLayer = {
        ...source,
        id: nextId("layer"),
        name: `${source.name} copy`,
        source: { ...source.source },
        colorKeyframes: source.colorKeyframes.map((k) => ({ ...k, id: nextId("ck") })),
        ledKeyframes: source.ledKeyframes.map((k) => ({ ...k, id: nextId("led") })),
        eq: source.eq.map((b) => ({ ...b, id: nextId("eq") })),
        curve: { ...source.curve },
        midi: { ...source.midi },
      };
      const layers = [...config.layers];
      layers.splice(index + 1, 0, copy);
      onChange({ ...config, layers, activeLayerId: copy.id });
    },
    [config, onChange],
  );

  const remove = useCallback(
    (id: string) => {
      // A show with no layers has nothing to render and no row to select, so
      // the last one cannot be deleted. Emptying it is what disable is for.
      if (config.layers.length <= 1) return;
      const layers = config.layers.filter((l) => l.id !== id);
      onChange({
        ...config,
        layers,
        activeLayerId:
          config.activeLayerId === id ? layers[layers.length - 1].id : config.activeLayerId,
      });
    },
    [config, onChange],
  );

  return (
    <Paper>
      <Stack gap="sm">
        <Group justify="space-between" align="center">
          <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
            Layers
          </Text>
          <Text size="xs" c="dimmed">
            {config.layers.length} of {MAX_LAYERS}
          </Text>
        </Group>

        <Text size="xs" c="dimmed" lh={1.35}>
          Top of the list paints over the rest. Drag to reorder; opacity blends with what is
          below, and black covers it.
        </Text>

        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          modifiers={[restrictToVerticalAxis, restrictToParentElement]}
          onDragEnd={onDragEnd}
        >
          <SortableContext
            items={rows.map((l) => l.id)}
            strategy={verticalListSortingStrategy}
          >
            <Stack gap={6}>
              {rows.map((layer) => (
                <LayerRow
                  key={layer.id}
                  layer={layer}
                  status={status.get(layer.id) ?? null}
                  selected={layer.id === config.activeLayerId}
                  canRemove={config.layers.length > 1}
                  canDuplicate={config.layers.length < MAX_LAYERS}
                  onSelect={() => onChange({ ...config, activeLayerId: layer.id })}
                  onPatch={(patch) => update(layer.id, patch)}
                  onDuplicate={() => duplicate(layer.id)}
                  onRemove={() => remove(layer.id)}
                />
              ))}
            </Stack>
          </SortableContext>
        </DndContext>

        <Button
          variant="default"
          size="xs"
          onClick={add}
          disabled={config.layers.length >= MAX_LAYERS}
        >
          Add layer
        </Button>
      </Stack>
    </Paper>
  );
});

interface RowProps {
  layer: EditorLayer;
  status: LayerStatus | null;
  selected: boolean;
  canRemove: boolean;
  canDuplicate: boolean;
  onSelect: () => void;
  onPatch: (patch: Partial<EditorLayer>) => void;
  onDuplicate: () => void;
  onRemove: () => void;
}

function LayerRow({
  layer,
  status,
  selected,
  canRemove,
  canDuplicate,
  onSelect,
  onPatch,
  onDuplicate,
  onRemove,
}: RowProps) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: layer.id,
  });
  const [renaming, setRenaming] = useState(false);

  const dim = !layer.enabled || layer.opacity <= 0;

  return (
    <Paper
      ref={setNodeRef}
      p="xs"
      withBorder
      onPointerDown={onSelect}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
        // Lifted out of the flow while dragging, or the rows it passes over
        // would paint on top of it.
        zIndex: isDragging ? 2 : undefined,
        position: "relative",
        opacity: isDragging ? 0.85 : 1,
        borderColor: selected ? "var(--mantine-primary-color-filled)" : undefined,
        cursor: "pointer",
      }}
    >
      <Stack gap={6}>
        <Group gap={6} wrap="nowrap" align="center">
          {/*
            The handle is its own target rather than the whole row: the row also
            selects, renames and carries a slider, and a drag that starts
            anywhere would make all three feel unreliable.
          */}
          <Box
            {...attributes}
            {...listeners}
            aria-label={`Reorder ${layer.name}`}
            style={{ cursor: "grab", display: "flex", touchAction: "none" }}
          >
            <GripIcon />
          </Box>

          <Tooltip label={layer.enabled ? "Hide layer" : "Show layer"}>
            <ActionIcon
              size="sm"
              variant="subtle"
              color={layer.enabled ? undefined : "gray"}
              aria-label={layer.enabled ? "Hide layer" : "Show layer"}
              onClick={(e) => {
                e.stopPropagation();
                onPatch({ enabled: !layer.enabled });
              }}
            >
              <EyeIcon open={layer.enabled} />
            </ActionIcon>
          </Tooltip>

          {renaming ? (
            <TextInput
              size="xs"
              autoFocus
              value={layer.name}
              onChange={(e) => onPatch({ name: e.currentTarget.value })}
              onBlur={() => setRenaming(false)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === "Escape") setRenaming(false);
              }}
              style={{ flex: 1, minWidth: 0 }}
            />
          ) : (
            <Text
              size="sm"
              fw={selected ? 600 : 400}
              c={dim ? "dimmed" : undefined}
              onDoubleClick={() => setRenaming(true)}
              style={{
                flex: 1,
                minWidth: 0,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {layer.name}
            </Text>
          )}

          <Tooltip label="Duplicate">
            <ActionIcon
              size="sm"
              variant="subtle"
              aria-label="Duplicate layer"
              disabled={!canDuplicate}
              onClick={(e) => {
                e.stopPropagation();
                onDuplicate();
              }}
            >
              <CopyIcon />
            </ActionIcon>
          </Tooltip>
          <Tooltip label={canRemove ? "Delete" : "A show needs one layer"}>
            <ActionIcon
              size="sm"
              variant="subtle"
              color="red"
              aria-label="Delete layer"
              disabled={!canRemove}
              onClick={(e) => {
                e.stopPropagation();
                onRemove();
              }}
            >
              <TrashIcon />
            </ActionIcon>
          </Tooltip>
        </Group>

        <Group gap={8} wrap="nowrap" align="center">
          <Text size="xs" c="dimmed" w={30} ff="monospace">
            {Math.round(layer.opacity * 100)}%
          </Text>
          <Slider
            flex={1}
            size="xs"
            min={0}
            max={1}
            step={0.01}
            value={layer.opacity}
            onChange={(opacity) => onPatch({ opacity })}
            label={(v) => `${Math.round(v * 100)}%`}
            // The slider lives inside a draggable row; without this a drag on
            // the thumb is stolen by the sortable sensor.
            style={{ touchAction: "none" }}
            onPointerDown={(e) => e.stopPropagation()}
          />
        </Group>

        <Group gap={6} wrap="nowrap">
          <Badge
            size="xs"
            variant="light"
            color={status?.error ? "red" : status ? "teal" : "gray"}
          >
            {status?.kind ?? layer.source.kind}
          </Badge>
          <Text
            size="xs"
            c={status?.error ? "red" : "dimmed"}
            style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
          >
            {describe(layer, status)}
          </Text>
        </Group>
      </Stack>
    </Paper>
  );
}

/**
 * One line saying what this row is actually listening to.
 *
 * The engine answers this, not the selection: "the system default" names no
 * device, and whether it opened at all is not something the editor can know.
 * Offline it says what was asked for instead, which is all there is.
 */
function describe(layer: EditorLayer, status: LayerStatus | null): string {
  if (status?.error) return status.error;
  if (!status) return layer.source.id ? "offline" : "system default · offline";

  const channel =
    layer.source.channel === null
      ? null
      : `ch ${layer.source.channel + 1}${status.channels ? ` of ${status.channels}` : ""}`;
  const rate =
    status.kind === "midi" || !status.sampleRate
      ? null
      : `${(status.sampleRate / 1000).toFixed(1)} kHz`;

  return [status.deviceName || "system default", rate, channel].filter(Boolean).join(" · ");
}

function GripIcon() {
  return (
    <svg width={12} height={16} viewBox="0 0 12 16" fill="currentColor" opacity={0.5}>
      <circle cx={4} cy={4} r={1.4} />
      <circle cx={8} cy={4} r={1.4} />
      <circle cx={4} cy={8} r={1.4} />
      <circle cx={8} cy={8} r={1.4} />
      <circle cx={4} cy={12} r={1.4} />
      <circle cx={8} cy={12} r={1.4} />
    </svg>
  );
}

function EyeIcon({ open }: { open: boolean }) {
  return (
    <svg
      width={14}
      height={14}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
    >
      <path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6Z" />
      <circle cx={12} cy={12} r={2.6} />
      {!open && <path d="M3 3l18 18" />}
    </svg>
  );
}

function CopyIcon() {
  return (
    <svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
      <rect x={9} y={9} width={11} height={11} rx={2} />
      <path d="M5 15V5a2 2 0 0 1 2-2h10" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg
      width={14}
      height={14}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
    >
      <path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" />
    </svg>
  );
}
