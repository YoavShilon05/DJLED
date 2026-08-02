import type { ReactNode } from "react";
import { Group, Stack, Text } from "@mantine/core";

interface Props {
  label: string;
  /** Current value, shown right-aligned against the label. */
  value?: ReactNode;
  hint?: string;
  children: ReactNode;
}

/**
 * Label, live value, control, note.
 *
 * Mantine's own `label`/`description` props would do most of this, but not the
 * value readout on the same line as the label — and a slider without one is
 * unusable for a number the user has to reason about. Pure composition of
 * Mantine primitives, so it inherits the theme rather than fighting it.
 */
export function Field({ label, value, hint, children }: Props) {
  return (
    <Stack gap={5}>
      <Group justify="space-between" gap="xs" wrap="nowrap">
        <Text size="xs" c="dimmed" fw={600}>
          {label}
        </Text>
        {value !== undefined && (
          <Text size="xs" ff="monospace" c="bright">
            {value}
          </Text>
        )}
      </Group>
      {children}
      {hint && (
        <Text size="xs" c="dimmed" lh={1.35}>
          {hint}
        </Text>
      )}
    </Stack>
  );
}
