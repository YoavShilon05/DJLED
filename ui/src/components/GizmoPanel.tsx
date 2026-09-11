import { memo } from "react";
import { Checkbox, Paper, Stack, Text } from "@mantine/core";

import type { EditorConfig, GizmoFlags } from "../config/editor";

interface Props {
  config: EditorConfig;
  onChange: (config: EditorConfig) => void;
}

const ITEMS: Array<{ key: keyof GizmoFlags; label: string }> = [
  { key: "eq", label: "Parametric EQ" },
  { key: "thresholds", label: "Threshold handles" },
  { key: "colorKeyframes", label: "Colour keyframes" },
  { key: "ledKeyframes", label: "LED keyframes" },
  { key: "curve", label: "Intensity curve" },
];

/**
 * Which overlays are drawn. Hiding a gizmo hides its handles too, so the plot
 * can be read as a plain analyser without anything to catch a stray click.
 *
 * These are visibility, not bypass: a hidden EQ still shapes the spectrum, and
 * a hidden threshold still cuts. Nothing here changes the signal.
 *
 * Memoised: nothing in here is driven by a frame, and a frame arrives thirty
 * times a second. See the note on `frame` in `App.tsx`.
 */
export const GizmoPanel = memo(function GizmoPanel({ config, onChange }: Props) {
  const value = ITEMS.filter((i) => config.gizmos[i.key]).map((i) => i.key);

  return (
    <Paper>
      <Stack gap="sm">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Gizmos
        </Text>
        <Text size="xs" c="dimmed">
          Visibility only — everything stays in the signal path.
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
});
