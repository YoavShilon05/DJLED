import { useLayoutEffect, useRef } from "react";
import { Group, Stack, Text } from "@mantine/core";

import type { Rgb } from "../color/display";

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
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const w = canvas.clientWidth;
    canvas.width = Math.max(1, Math.round(w * dpr));
    canvas.height = Math.round(height * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, w, height);
    if (colors.length === 0) return;

    const step = w / colors.length;
    for (let i = 0; i < colors.length; i++) {
      const [r, g, b] = colors[i];
      ctx.fillStyle = `rgb(${r},${g},${b})`;
      // Overdrawn by a pixel so sub-pixel widths leave no seam between LEDs.
      ctx.fillRect(i * step, 0, step + 1, height);
    }
  }, [colors, height]);

  return (
    <Stack gap={6}>
      <Group justify="space-between" gap="xs">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase">
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
