import { memo } from "react";
import { Badge, Group, Menu, Paper, Slider, Text, Title, Tooltip } from "@mantine/core";

import type { LayerStatus, Status } from "../engine";

interface Props {
  status: Status;
  ledCount: number;
  layers: number;
  statuses: LayerStatus[];
  brightness: number;
  onBrightness: (value: number) => void;
  /** The live slot's name, for saying what Reset is about to replace. */
  presetLabel: string;
  onReset: () => void;
}

/**
 * The bar that is always on screen.
 *
 * It holds the three things that are never about one layer: whether the engine
 * is there, how bright the wall is, and the escape hatch. Master brightness in
 * particular used to be the last slider of the fourth panel down, behind a
 * divider, inside the settings for a single layer — which is the wrong place
 * for the one control somebody reaches for mid-set, and the wrong place for the
 * software half of a power budget.
 *
 * Memoised: nothing in here is driven by a frame, and a frame arrives thirty
 * times a second. See the note on `frame` in `App.tsx`.
 */
export const HeaderBar = memo(function HeaderBar({
  status,
  ledCount,
  layers,
  statuses,
  brightness,
  onBrightness,
  presetLabel,
  onReset,
}: Props) {
  return (
    <Paper p="xs">
      <Group justify="space-between" align="center" gap="md" wrap="nowrap">
        <Group gap="sm" align="center" wrap="nowrap">
          <Title order={1} size="h5" tt="uppercase" lts="0.12em">
            DJLED
          </Title>
          <StatusBadge status={status} ledCount={ledCount} layers={layers} statuses={statuses} />
        </Group>

        <Group gap="sm" align="center" wrap="nowrap" style={{ flexShrink: 0 }}>
          <Tooltip label="The whole strip, every layer. Also the software half of the power budget.">
            <Text size="xs" c="dimmed" fw={600} style={{ cursor: "help" }}>
              Master
            </Text>
          </Tooltip>
          <Slider
            w={180}
            min={0}
            max={1}
            step={0.01}
            value={brightness}
            onChange={onBrightness}
            label={(v) => `${Math.round(v * 100)}%`}
          />
          <Text size="xs" ff="monospace" c="bright" w={38} ta="right">
            {Math.round(brightness * 100)}%
          </Text>

          <Menu position="bottom-end" withArrow>
            <Menu.Target>
              <Text
                component="button"
                size="sm"
                c="dimmed"
                px={6}
                aria-label="More"
                style={{ background: "none", border: 0, cursor: "pointer" }}
              >
                ⋯
              </Text>
            </Menu.Target>
            <Menu.Dropdown>
              <Menu.Label>Live preset — {presetLabel}</Menu.Label>
              {/* Behind a menu now rather than a button beside the title.
                  It reads like undo and is not: it replaces the live preset
                  with the default show, and edits are saved as they are made,
                  so there is no older version for it to put back. */}
              <Menu.Item color="red" onClick={onReset}>
                Replace with the default show
              </Menu.Item>
              <Menu.Label>The other eleven presets are untouched.</Menu.Label>
            </Menu.Dropdown>
          </Menu>
        </Group>
      </Group>
    </Paper>
  );
});

function StatusBadge({
  status,
  ledCount,
  layers,
  statuses,
}: {
  status: Status;
  ledCount: number;
  layers: number;
  statuses: LayerStatus[];
}) {
  if (status === "connected") {
    // One dead layer is not a dead engine, and burying it in a per-layer panel
    // would let a stack run half-dark without anything saying so up here.
    const failed = statuses.filter((s) => s.error).length;
    if (failed > 0) {
      return <Badge color="red">{`${failed} of ${statuses.length} layers have no source`}</Badge>;
    }
    const kinds = new Set(statuses.map((s) => s.kind));
    const label = kinds.size === 1 ? [...kinds][0] : `${kinds.size} kinds`;
    const stack = layers === 1 ? "1 layer" : `${layers} layers`;
    return <Badge color="teal">{`${stack} · ${label} · ${ledCount} LEDs`}</Badge>;
  }
  if (status === "connecting") return <Badge color="gray">connecting…</Badge>;
  return <Badge color="yellow">offline — editing locally</Badge>;
}
