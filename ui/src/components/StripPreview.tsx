import { useLayoutEffect, useRef } from "react";
import { Group, Stack, Text } from "@mantine/core";

import type { Rgb } from "../color/display";
import { paintStrip } from "./strip";

interface Props {
  label: string;
  hint?: string;
  /** Already in display sRGB — see `color/display.ts`. */
  colors: Rgb[];
  height?: number;
}

/** One run of LEDs, drawn edge to edge with no gaps — a 10 m strip has 600. */
export function StripPreview({ label, hint, colors, height = 26 }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (canvas) paintStrip(canvas, colors, height);
  }, [colors, height]);

  return (
    <Stack gap={4}>
      <Group justify="space-between" gap="xs">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.06em">
          {label}
        </Text>
        {hint && (
          <Text size="xs" c="dimmed" ff="monospace">
            {hint}
          </Text>
        )}
      </Group>
      <canvas
        ref={canvasRef}
        style={{
          display: "block",
          width: "100%",
          height,
          borderRadius: "var(--mantine-radius-sm)",
          background: "#000",
        }}
      />
    </Stack>
  );
}
