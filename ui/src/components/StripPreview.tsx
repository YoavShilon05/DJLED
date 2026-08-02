/**
 * The strip as it will actually look on the wall.
 *
 * When the engine is connected this draws the pixels it reports, which have
 * already been through the colour surface, brightness scaling and temporal
 * dithering — ground truth rather than a re-derivation. Offline it falls back to
 * sampling the surface locally so the preview stays useful while authoring with
 * nothing running.
 */

import { useEffect, useRef } from "react";

import { oklabToHex } from "../color/oklab";
import { ColorSurface } from "../color/surface";

interface Props {
  /** Engine-reported pixels, or null when offline. */
  pixels: Array<[number, number, number]> | null;
  /** Used to synthesise a preview when offline. */
  surface: ColorSurface;
  levels: number[];
  ledCount: number;
}

export function StripPreview({ pixels, surface, levels, ledCount }: Props) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.max(1, Math.floor(rect.width * dpr));
    canvas.height = Math.max(1, Math.floor(rect.height * dpr));
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const count = pixels?.length || ledCount || 1;
    const width = rect.width / count;

    for (let i = 0; i < count; i++) {
      if (pixels) {
        const [r, g, b] = pixels[i];
        // Engine pixels are linear light, as the LED receives them. Encoding to
        // sRGB here is what makes the screen show the same brightness the eye
        // will read off the wall.
        ctx.fillStyle = `rgb(${toSrgbByte(r)} ${toSrgbByte(g)} ${toSrgbByte(b)})`;
      } else {
        const t = count > 1 ? i / (count - 1) : 0;
        const band = Math.min(levels.length - 1, Math.round(t * (levels.length - 1)));
        ctx.fillStyle = oklabToHex(surface.sample(t, levels[band] ?? 0));
      }
      // Overlap by a pixel so sub-pixel widths leave no seams.
      ctx.fillRect(i * width, 0, width + 1, rect.height);
    }
  }, [pixels, surface, levels, ledCount]);

  return <canvas ref={ref} className="strip-preview" />;
}

/** Linear light byte to its sRGB-encoded equivalent, for display on a monitor. */
function toSrgbByte(v: number): number {
  const linear = v / 255;
  const encoded = linear <= 0.0031308 ? linear * 12.92 : 1.055 * Math.pow(linear, 1 / 2.4) - 0.055;
  return Math.round(Math.min(1, Math.max(0, encoded)) * 255);
}
