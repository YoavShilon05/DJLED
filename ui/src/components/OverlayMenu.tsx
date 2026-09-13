import { memo } from "react";
import { Button, Checkbox, Popover, Stack, Text } from "@mantine/core";

import type { EditorConfig, GizmoFlags } from "../config/editor";

const ITEMS: Array<{ key: keyof GizmoFlags; label: string }> = [
  { key: "eq", label: "Parametric EQ" },
  { key: "thresholds", label: "Threshold handles" },
  { key: "colorKeyframes", label: "Colour keyframes" },
  { key: "ledKeyframes", label: "LED keyframes" },
  { key: "curve", label: "Intensity curve" },
];

interface Props {
  config: EditorConfig;
  onChange: (config: EditorConfig) => void;
}

/**
 * Which overlays the graph draws.
 *
 * Moved out of the sidebar and onto the graph's own header, because that is
 * what it controls — five checkboxes holding down a permanent panel in a
 * column of settings, three panels away from the thing they show and hide, was
 * both the least-used control in the editor and one of the tallest.
 *
 * Hiding a gizmo hides its handles too, so the plot can be read as a plain
 * analyser without anything to catch a stray click. These are visibility, not
 * bypass: a hidden EQ still shapes the spectrum and a hidden threshold still
 * cuts. Nothing here changes the signal.
 *
 * Memoised: nothing in here is driven by a frame, and a frame arrives thirty
 * times a second. See the note on `frame` in `App.tsx`.
 */
export const OverlayMenu = memo(function OverlayMenu({ config, onChange }: Props) {
  const shown = ITEMS.filter((i) => config.gizmos[i.key]).map((i) => i.key);

  return (
    <Popover position="bottom-end" withArrow>
      <Popover.Target>
        <Button variant="default" size="xs">
          Overlays · {shown.length}/{ITEMS.length}
        </Button>
      </Popover.Target>
      <Popover.Dropdown p="sm">
        <Stack gap="xs">
          <Checkbox.Group
            value={shown}
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
          <Text size="xs" c="dimmed" lh={1.35} maw={220}>
            Visibility only — everything stays in the signal path.
          </Text>
        </Stack>
      </Popover.Dropdown>
    </Popover>
  );
});
