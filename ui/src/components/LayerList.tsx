import { ActionIcon, Badge, Group, Menu, Paper, Stack, Text, Tooltip, UnstyledButton } from "@mantine/core";

import {
  LAYER_KIND_LABEL,
  type BlendMode,
  type Layer,
  type LayerKind,
} from "../config/layers";

interface Props {
  layers: Layer[];
  focusedId: string | null;
  soloId: string | null;
  onFocus: (id: string) => void;
  onToggle: (id: string) => void;
  onSolo: (id: string | null) => void;
  onMove: (id: string, delta: number) => void;
  onAdd: (kind: LayerKind) => void;
  onRemove: (id: string) => void;
}

const BLEND_SHORT: Record<BlendMode, string> = {
  normal: "nrm",
  add: "add",
  multiply: "mul",
};

/**
 * The stack, drawn as a stack.
 *
 * Top of the list is top of the composite, which is why the array is reversed
 * for display: `layers[0]` is the bottom layer because it is composited first,
 * and showing it at the top would put the fold order upside down.
 *
 * The bottom layer's blend mode is shown greyed. It has nothing beneath it to
 * combine with, so whatever it is set to, it lands on black — saying so is
 * cheaper than letting someone spend a minute wondering why switching it does
 * nothing.
 */
export function LayerList({
  layers,
  focusedId,
  soloId,
  onFocus,
  onToggle,
  onSolo,
  onMove,
  onAdd,
  onRemove,
}: Props) {
  const top = [...layers].reverse();

  return (
    <Paper>
      <Stack gap="sm">
        <Group justify="space-between" gap="xs">
          <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
            Layers
          </Text>
          <Menu position="bottom-end" withinPortal>
            <Menu.Target>
              <ActionIcon aria-label="Add layer" variant="default">
                <PlusIcon />
              </ActionIcon>
            </Menu.Target>
            <Menu.Dropdown>
              <Menu.Item onClick={() => onAdd("spectrum")}>Spectrum</Menu.Item>
              <Menu.Item onClick={() => onAdd("static")}>Static colour</Menu.Item>
            </Menu.Dropdown>
          </Menu>
        </Group>

        <Stack gap={4}>
          {top.map((layer, i) => {
            const index = layers.length - 1 - i;
            const focused = layer.id === focusedId;
            const dimmed = soloId !== null && soloId !== layer.id;
            return (
              <Paper
                key={layer.id}
                p={8}
                withBorder
                style={{
                  borderColor: focused
                    ? "var(--mantine-color-brand-6)"
                    : "var(--mantine-color-dark-4)",
                  opacity: layer.enabled && !dimmed ? 1 : 0.45,
                }}
              >
                <Stack gap={4}>
                  <Group gap={6} wrap="nowrap">
                    <Tooltip label={layer.enabled ? "Mute" : "Unmute"}>
                      <ActionIcon
                        size="xs"
                        variant="subtle"
                        aria-label={layer.enabled ? "Mute layer" : "Unmute layer"}
                        onClick={() => onToggle(layer.id)}
                      >
                        {layer.enabled ? <DotFilled /> : <DotHollow />}
                      </ActionIcon>
                    </Tooltip>

                    <UnstyledButton
                      style={{ flex: 1, minWidth: 0 }}
                      onClick={() => onFocus(layer.id)}
                    >
                      <Text size="xs" fw={focused ? 700 : 500} truncate>
                        {layer.name}
                      </Text>
                    </UnstyledButton>

                    <Tooltip label="Solo">
                      <ActionIcon
                        size="xs"
                        variant={soloId === layer.id ? "filled" : "subtle"}
                        aria-label="Solo layer"
                        onClick={() => onSolo(soloId === layer.id ? null : layer.id)}
                      >
                        <Text size="9px" fw={700}>
                          S
                        </Text>
                      </ActionIcon>
                    </Tooltip>
                  </Group>

                  <Group gap={6} justify="space-between" wrap="nowrap">
                    <Group gap={4} wrap="nowrap">
                      <Badge size="xs" variant="default">
                        {LAYER_KIND_LABEL[layer.kind]}
                      </Badge>
                      <Text
                        size="10px"
                        ff="monospace"
                        c={index === 0 ? "dimmed" : undefined}
                        title={
                          index === 0
                            ? "Nothing beneath this layer, so its blend mode has no effect."
                            : undefined
                        }
                      >
                        {BLEND_SHORT[layer.blendMode]} {Math.round(layer.opacity * 100)}%
                      </Text>
                      {layer.keys.length > 0 && (
                        <Tooltip label={`${layer.keys.length} keys · ${layer.loopSeconds}s loop`}>
                          <Text size="10px" ff="monospace" c="brand.4">
                            ◆{layer.keys.length}
                          </Text>
                        </Tooltip>
                      )}
                    </Group>

                    <Group gap={0} wrap="nowrap">
                      <ActionIcon
                        size="xs"
                        aria-label="Move up"
                        disabled={index === layers.length - 1}
                        onClick={() => onMove(layer.id, 1)}
                      >
                        <ChevronUp />
                      </ActionIcon>
                      <ActionIcon
                        size="xs"
                        aria-label="Move down"
                        disabled={index === 0}
                        onClick={() => onMove(layer.id, -1)}
                      >
                        <ChevronDown />
                      </ActionIcon>
                      <ActionIcon
                        size="xs"
                        color="red"
                        aria-label="Remove layer"
                        disabled={layers.length <= 1}
                        onClick={() => onRemove(layer.id)}
                      >
                        <TrashIcon />
                      </ActionIcon>
                    </Group>
                  </Group>
                </Stack>
              </Paper>
            );
          })}
        </Stack>

        <Text size="xs" c="dimmed" lh={1.35}>
          Top of the list is top of the composite. Add and multiply are
          order-independent; normal is not.
        </Text>
      </Stack>
    </Paper>
  );
}

function PlusIcon() {
  return (
    <svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}

function ChevronUp() {
  return (
    <svg width={12} height={12} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5} strokeLinecap="round">
      <path d="M6 15l6-6 6 6" />
    </svg>
  );
}

function ChevronDown() {
  return (
    <svg width={12} height={12} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5} strokeLinecap="round">
      <path d="M6 9l6 6 6-6" />
    </svg>
  );
}

function DotFilled() {
  return (
    <svg width={12} height={12} viewBox="0 0 24 24" fill="currentColor">
      <circle cx={12} cy={12} r={7} />
    </svg>
  );
}

function DotHollow() {
  return (
    <svg width={12} height={12} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
      <circle cx={12} cy={12} r={6} />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg width={12} height={12} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
      <path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" />
    </svg>
  );
}
