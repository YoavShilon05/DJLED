import type { ReactNode } from "react";
import { Group, Text } from "@mantine/core";

import { InfoDot } from "./Field";

interface Props {
  title: string;
  /** The paragraph that used to sit under the heading, one hover away. */
  info?: ReactNode;
  /** A count, a control, whatever belongs opposite the title. */
  right?: ReactNode;
}

/**
 * The small uppercase label at the top of a panel.
 *
 * Five copies of the same three props were spread across the panels, and each
 * one carried a paragraph of explanation directly underneath — which is how a
 * sidebar of six panels became taller than any screen it was opened on. The
 * paragraph moves behind the icon; the label stays.
 */
export function PanelHeading({ title, info, right }: Props) {
  return (
    <Group justify="space-between" align="center" gap="xs" wrap="nowrap">
      <Group gap={4} align="center" wrap="nowrap" style={{ minWidth: 0 }}>
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em" truncate>
          {title}
        </Text>
        {info && <InfoDot label={title}>{info}</InfoDot>}
      </Group>
      {right}
    </Group>
  );
}
