import type { ReactNode } from "react";
import { ActionIcon, Group, Stack, Text, Tooltip } from "@mantine/core";

interface Props {
  label: string;
  /** Current value, shown right-aligned against the label. */
  value?: ReactNode;
  /** One short line under the control. Anything longer belongs in `info`. */
  hint?: string;
  /**
   * The long version, behind an icon beside the label.
   *
   * The reasoning behind these controls is worth keeping — it is the difference
   * between "loopback" meaning something and meaning nothing — but a paragraph
   * per field turns a sidebar into a document, and the panel that has to be
   * scrolled past is the panel nobody reads. So the paragraph stays, one hover
   * away, and only the line worth having on screen permanently is `hint`.
   */
  info?: ReactNode;
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
export function Field({ label, value, hint, info, children }: Props) {
  return (
    <Stack gap={5}>
      <Group justify="space-between" gap="xs" wrap="nowrap">
        <Group gap={4} wrap="nowrap" style={{ minWidth: 0 }}>
          <Text size="xs" c="dimmed" fw={600} truncate>
            {label}
          </Text>
          {info && <InfoDot label={label}>{info}</InfoDot>}
        </Group>
        {value !== undefined && (
          <Text size="xs" ff="monospace" c="bright" style={{ whiteSpace: "nowrap" }}>
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

/**
 * The icon that holds a field's long explanation.
 *
 * A button rather than a bare glyph so the tooltip is reachable by keyboard —
 * a hover-only explanation is no explanation for anyone tabbing through.
 */
export function InfoDot({ label, children }: { label: string; children: ReactNode }) {
  return (
    <Tooltip label={children} multiline w={300} position="right" withArrow openDelay={150}>
      <ActionIcon
        size={15}
        radius="xl"
        variant="subtle"
        color="gray"
        aria-label={`About ${label}`}
        onClick={(e) => e.preventDefault()}
      >
        <Text size="xs" fw={700} lh={1}>
          ?
        </Text>
      </ActionIcon>
    </Tooltip>
  );
}
