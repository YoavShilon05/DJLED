import { Checkbox, Paper, Stack, Text } from "@mantine/core";

import type { EditorConfig, GizmoFlags } from "../config/editor";

interface Props {
  config: EditorConfig;
  onChange: (config: EditorConfig) => void;
}

const ITEMS: Array<{ key: keyof GizmoFlags; label: string }> = [
  { key: "thresholds", label: "Threshold handles" },
  { key: "colorKeyframes", label: "Colour keyframes" },
  { key: "ledKeyframes", label: "LED keyframes" },
  { key: "curve", label: "Intensity curve" },
];

/**
 * Which overlays are drawn. Hiding a gizmo hides its handles too, so the plot
 * can be read as a plain analyser without anything to catch a stray click.
 */
export function GizmoPanel({ config, onChange }: Props) {
  const value = ITEMS.filter((i) => config.gizmos[i.key]).map((i) => i.key);

  return (
    <Paper>
      <Stack gap="sm">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Gizmos
        </Text>
        <Checkbox.Group
          value={value}
          onChange={(next) =>
            onChange({
              ...config,
              gizmos: ITEMS.reduce(
                (acc, i) => ({ ...acc, [i.key]: next.includes(i.key) }),
                {} as GizmoFlags,
              ),
            })
          }
        >
          <Stack gap="xs">
            {ITEMS.map((i) => (
              <Checkbox key={i.key} value={i.key} label={i.label} />
            ))}
          </Stack>
        </Checkbox.Group>
      </Stack>
    </Paper>
  );
}
