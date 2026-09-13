import { useEffect, useRef } from "react";

import type { Rgb } from "../color/display";
import { paintStrip } from "./strip";

/**
 * Where a row's thumbnail gets its pixels.
 *
 * A pull rather than a prop, and that is the whole point. The layer list is
 * memoised because a frame arrives thirty times a second and re-rendering
 * twelve Mantine rows at that rate is how a tab falls behind; handing each row
 * a fresh array of colours would put every one of them back on the frame path
 * and undo it. So `App` parks the latest strips in a ref, this object reads
 * them, and the only thing that moves at frame rate is a canvas.
 *
 * See the note on `frame` in `App.tsx`.
 */
export interface LayerPreview {
  /** The latest strip for a layer, or null if none has been rendered yet. */
  read(id: string): Rgb[] | null;
}

/**
 * Redraw interval, in ms.
 *
 * A thumbnail is 100-odd pixels wide and exists to answer "which row is this",
 * so a third of the engine's rate is plenty and twelve of them at full rate is
 * not free. The strip preview above is the one that has to be honest about
 * timing.
 */
const INTERVAL = 1000 / 20;

interface Props {
  id: string;
  preview: LayerPreview;
  height?: number;
}

/**
 * What one layer alone is putting on the wall, beside its row.
 *
 * The list used to say only what a layer was *listening to*, which does not
 * answer the question anyone actually has in front of a six-layer stack —
 * which of these is the blue one. A colour swatch would not either: a layer is
 * a whole strip's worth of colour, changing with the music, and sometimes
 * reaching only part of the wall.
 */
export function LayerThumb({ id, preview, height = 16 }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    let raf = 0;
    let last = 0;
    const tick = (now: number) => {
      raf = requestAnimationFrame(tick);
      if (now - last < INTERVAL) return;
      last = now;
      const canvas = canvasRef.current;
      const colors = preview.read(id);
      if (canvas && colors) paintStrip(canvas, colors, height);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [id, preview, height]);

  return (
    <canvas
      ref={canvasRef}
      aria-hidden
      style={{
        display: "block",
        width: "100%",
        height,
        borderRadius: "var(--mantine-radius-xs)",
        background: "#000",
      }}
    />
  );
}
