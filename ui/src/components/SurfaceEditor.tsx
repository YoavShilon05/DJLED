/**
 * The 2D keyframe editor.
 *
 * The field is drawn once into an offscreen bitmap and only redrawn when the
 * surface changes; handles and the live spectrum overlay are redrawn every
 * frame on top. Sampling the field is ~6 `exp` calls per pixel, so re-rendering
 * it at animation rate would be wasteful — and it does not change unless you
 * edit it.
 *
 * The offscreen bitmap is deliberately low resolution and scaled up. The field
 * is smooth by construction, so bilinear upscaling is indistinguishable from
 * sampling per pixel and roughly twenty times cheaper.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { oklabToHex } from "../color/oklab";
import { ColorSurface, cloneSurface, type SurfaceConfig } from "../color/surface";

const FIELD_W = 160;
const FIELD_H = 96;
const HANDLE_RADIUS = 7;
const HIT_RADIUS = 12;

interface Props {
  surface: SurfaceConfig;
  onChange: (next: SurfaceConfig) => void;
  /** Current band levels, drawn over the field so you can tune against audio. */
  levels: number[];
  /** Band centre frequencies, for the axis labels. */
  centers: number[];
  selected: number | null;
  onSelect: (index: number | null) => void;
}

export function SurfaceEditor({
  surface,
  onChange,
  levels,
  centers,
  selected,
  onSelect,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const fieldRef = useRef<HTMLCanvasElement | null>(null);
  const dragRef = useRef<number | null>(null);
  const [size, setSize] = useState({ w: 720, h: 340 });

  const compiled = useMemo(() => new ColorSurface(surface), [surface]);

  // Redraw the field bitmap whenever the surface changes.
  useEffect(() => {
    let field = fieldRef.current;
    if (!field) {
      field = document.createElement("canvas");
      field.width = FIELD_W;
      field.height = FIELD_H;
      fieldRef.current = field;
    }
    const ctx = field.getContext("2d");
    if (!ctx) return;

    const image = ctx.createImageData(FIELD_W, FIELD_H);
    for (let py = 0; py < FIELD_H; py++) {
      // Screen y runs downward; intensity runs upward.
      const y = 1 - py / (FIELD_H - 1);
      for (let px = 0; px < FIELD_W; px++) {
        const x = px / (FIELD_W - 1);
        const hex = oklabToHex(compiled.sample(x, y));
        const n = parseInt(hex.slice(1), 16);
        const i = (py * FIELD_W + px) * 4;
        image.data[i] = (n >> 16) & 0xff;
        image.data[i + 1] = (n >> 8) & 0xff;
        image.data[i + 2] = n & 0xff;
        image.data[i + 3] = 255;
      }
    }
    ctx.putImageData(image, 0, 0);
  }, [compiled]);

  // Track the element's size so the canvas stays crisp and responsive.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0].contentRect;
      setSize({ w: Math.max(320, rect.width), h: Math.max(220, rect.height) });
    });
    observer.observe(canvas.parentElement ?? canvas);
    return () => observer.disconnect();
  }, []);

  // Composite: field, grid, live spectrum, handles.
  useEffect(() => {
    let raf = 0;
    const draw = () => {
      raf = requestAnimationFrame(draw);
      const canvas = canvasRef.current;
      const field = fieldRef.current;
      if (!canvas || !field) return;

      const dpr = window.devicePixelRatio || 1;
      const { w, h } = size;
      if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
        canvas.width = w * dpr;
        canvas.height = h * dpr;
      }
      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.imageSmoothingEnabled = true;
      ctx.drawImage(field, 0, 0, w, h);

      // Grid
      ctx.strokeStyle = "rgba(255,255,255,0.10)";
      ctx.lineWidth = 1;
      for (let i = 1; i < 4; i++) {
        const y = (h * i) / 4;
        ctx.beginPath();
        ctx.moveTo(0, y);
        ctx.lineTo(w, y);
        ctx.stroke();
      }

      // Live spectrum: where each band currently sits in the field. This is the
      // point of editing here rather than in a colour picker — you can see which
      // part of the surface the music is actually using.
      if (levels.length > 1) {
        ctx.beginPath();
        for (let i = 0; i < levels.length; i++) {
          const x = (i / (levels.length - 1)) * w;
          const y = h * (1 - Math.min(1, Math.max(0, levels[i])));
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.strokeStyle = "rgba(255,255,255,0.85)";
        ctx.lineWidth = 2;
        ctx.stroke();

        ctx.lineTo(w, h);
        ctx.lineTo(0, h);
        ctx.closePath();
        ctx.fillStyle = "rgba(0,0,0,0.28)";
        ctx.fill();
      }

      // Handles
      surface.keyframes.forEach((k, i) => {
        const x = k.x * w;
        const y = (1 - k.y) * h;
        const isSelected = i === selected;

        ctx.beginPath();
        ctx.arc(x, y, HANDLE_RADIUS + (isSelected ? 3 : 0), 0, Math.PI * 2);
        ctx.fillStyle = k.color;
        ctx.fill();
        ctx.lineWidth = isSelected ? 3 : 2;
        ctx.strokeStyle = isSelected ? "#fff" : "rgba(0,0,0,0.65)";
        ctx.stroke();
      });
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [size, levels, surface, selected]);

  const toField = useCallback(
    (event: React.PointerEvent<HTMLCanvasElement> | React.MouseEvent<HTMLCanvasElement>) => {
      const rect = event.currentTarget.getBoundingClientRect();
      return {
        x: Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width)),
        y: Math.min(1, Math.max(0, 1 - (event.clientY - rect.top) / rect.height)),
        px: event.clientX - rect.left,
        py: event.clientY - rect.top,
        rect,
      };
    },
    [],
  );

  const hitTest = useCallback(
    (px: number, py: number, rect: DOMRect) => {
      for (let i = surface.keyframes.length - 1; i >= 0; i--) {
        const k = surface.keyframes[i];
        const dx = px - k.x * rect.width;
        const dy = py - (1 - k.y) * rect.height;
        if (dx * dx + dy * dy <= HIT_RADIUS * HIT_RADIUS) return i;
      }
      return null;
    },
    [surface.keyframes],
  );

  const onPointerDown = (event: React.PointerEvent<HTMLCanvasElement>) => {
    if (event.button !== 0) return;
    const { x, y, px, py, rect } = toField(event);
    const hit = hitTest(px, py, rect);

    if (hit !== null) {
      dragRef.current = hit;
      onSelect(hit);
      event.currentTarget.setPointerCapture(event.pointerId);
      return;
    }

    // New keyframes adopt the colour already showing there, so adding one never
    // jumps the field — it becomes a handle on what was already the case.
    const next = cloneSurface(surface);
    next.keyframes.push({ x, y, color: oklabToHex(compiled.sample(x, y)) });
    onChange(next);
    onSelect(next.keyframes.length - 1);
  };

  const onPointerMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const index = dragRef.current;
    if (index === null) return;
    const { x, y } = toField(event);
    const next = cloneSurface(surface);
    next.keyframes[index] = { ...next.keyframes[index], x, y };
    onChange(next);
  };

  const endDrag = (event: React.PointerEvent<HTMLCanvasElement>) => {
    if (dragRef.current !== null) {
      dragRef.current = null;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    }
  };

  const onContextMenu = (event: React.MouseEvent<HTMLCanvasElement>) => {
    event.preventDefault();
    const { px, py, rect } = toField(event);
    const hit = hitTest(px, py, rect);
    // The surface needs at least one keyframe to be defined; keeping two avoids
    // a degenerate flat field that is confusing to recover from.
    if (hit === null || surface.keyframes.length <= 2) return;

    const next = cloneSurface(surface);
    next.keyframes.splice(hit, 1);
    onChange(next);
    onSelect(null);
  };

  return (
    <div className="surface-editor">
      <canvas
        ref={canvasRef}
        style={{ width: "100%", height: `${size.h}px` }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onContextMenu={onContextMenu}
      />
      <div className="axis">
        {axisLabels(centers).map((label, i) => (
          <span key={i}>{label}</span>
        ))}
      </div>
      <p className="hint">
        drag to move · click empty space to add · right-click a point to delete
      </p>
    </div>
  );
}

/** Five evenly spaced frequency labels across the band range. */
function axisLabels(centers: number[]): string[] {
  if (centers.length === 0) return ["bass", "", "", "", "treble"];
  return Array.from({ length: 5 }, (_, i) => {
    const hz = centers[Math.round((i / 4) * (centers.length - 1))];
    return hz >= 1000 ? `${(hz / 1000).toFixed(1)}k` : `${Math.round(hz)}`;
  });
}
