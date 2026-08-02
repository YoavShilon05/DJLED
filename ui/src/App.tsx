import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { ColorSurface, DEFAULT_SURFACE, cloneSurface, type SurfaceConfig } from "./color/surface";
import { DEFAULT_URL, EngineClient, decodeStrip, type Frame, type Status } from "./engine";
import { StripPreview } from "./components/StripPreview";
import { SurfaceEditor } from "./components/SurfaceEditor";

const PRESET_KEY = "djled.surface";

export default function App() {
  const [surface, setSurface] = useState<SurfaceConfig>(loadPreset);
  const [selected, setSelected] = useState<number | null>(null);
  const [status, setStatus] = useState<Status>("connecting");
  const [frame, setFrame] = useState<Frame | null>(null);
  const [brightness, setBrightness] = useState(1);
  const [ledCount, setLedCount] = useState(150);
  const [error, setError] = useState<string | null>(null);

  const clientRef = useRef<EngineClient | null>(null);
  const compiled = useMemo(() => new ColorSurface(surface), [surface]);

  useEffect(() => {
    const client = new EngineClient(DEFAULT_URL, {
      onStatus: setStatus,
      onFrame: setFrame,
      onError: setError,
      onState: (state) => {
        setBrightness(state.brightness);
        setLedCount(state.ledCount);
        // The engine's surface is authoritative on connect, but only if nothing
        // has been authored here yet — otherwise reconnecting would silently
        // discard unsaved edits.
        if (!localStorage.getItem(PRESET_KEY)) setSurface(state.surface);
      },
    });
    clientRef.current = client;
    client.connect();
    return () => client.close();
  }, []);

  const applySurface = useCallback((next: SurfaceConfig) => {
    setSurface(next);
    setError(null);
    clientRef.current?.setSurface(next);
  }, []);

  // Demo levels keep the editor alive while offline, so a palette can be
  // authored with nothing running.
  const demo = useDemoLevels(status !== "connected");
  const levels = status === "connected" && frame ? frame.levels : demo;
  const centers = frame?.centers ?? [];
  const pixels = status === "connected" && frame ? decodeStrip(frame.strip) : null;

  const selectedKeyframe = selected !== null ? surface.keyframes[selected] : undefined;

  return (
    <div className="app">
      <header>
        <h1>DJLED</h1>
        <span className={`status status-${status}`}>
          {status === "connected"
            ? `${frame?.sampleRate ? `${(frame.sampleRate / 1000).toFixed(1)} kHz` : "live"} · ${ledCount} LEDs`
            : status === "connecting"
              ? "connecting…"
              : "offline — editing locally"}
        </span>
      </header>

      {error && <div className="error">{error}</div>}

      <main>
        <section className="editor-pane">
          <div className="labels">
            <span>intensity ↑</span>
            <span>frequency →</span>
          </div>
          <SurfaceEditor
            surface={surface}
            onChange={applySurface}
            levels={levels}
            centers={centers}
            selected={selected}
            onSelect={setSelected}
          />
        </section>

        <aside>
          <h2>keyframe</h2>
          {selectedKeyframe ? (
            <div className="field">
              <input
                type="color"
                value={selectedKeyframe.color}
                onChange={(e) => {
                  const next = cloneSurface(surface);
                  next.keyframes[selected!] = {
                    ...next.keyframes[selected!],
                    color: e.target.value,
                  };
                  applySurface(next);
                }}
              />
              <code>{selectedKeyframe.color}</code>
              <button
                disabled={surface.keyframes.length <= 2}
                onClick={() => {
                  const next = cloneSurface(surface);
                  next.keyframes.splice(selected!, 1);
                  applySurface(next);
                  setSelected(null);
                }}
              >
                delete
              </button>
            </div>
          ) : (
            <p className="muted">select a point to change its colour</p>
          )}

          <h2>blend radius</h2>
          <input
            type="range"
            min={0.05}
            max={0.6}
            step={0.01}
            value={surface.sigma}
            onChange={(e) => applySurface({ ...cloneSurface(surface), sigma: +e.target.value })}
          />
          <code>{surface.sigma.toFixed(2)}</code>
          <p className="muted">
            how far each point's influence reaches. smaller is crisper, larger blurs
            neighbouring colours together.
          </p>

          <h2>brightness</h2>
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={brightness}
            onChange={(e) => {
              const v = +e.target.value;
              setBrightness(v);
              clientRef.current?.setBrightness(v);
            }}
          />
          <code>{Math.round(brightness * 100)}%</code>

          <h2>preset</h2>
          <div className="row">
            <button onClick={() => localStorage.setItem(PRESET_KEY, JSON.stringify(surface))}>
              save
            </button>
            <button
              onClick={() => {
                localStorage.removeItem(PRESET_KEY);
                applySurface(cloneSurface(DEFAULT_SURFACE));
                setSelected(null);
              }}
            >
              reset
            </button>
          </div>
          {frame && frame.droppedFrames > 0 && (
            <p className="muted">dropped frames: {frame.droppedFrames}</p>
          )}
        </aside>
      </main>

      <section className="strip-pane">
        <h2>strip</h2>
        <StripPreview
          pixels={pixels}
          surface={compiled}
          levels={levels}
          ledCount={ledCount}
        />
      </section>
    </div>
  );
}

function loadPreset(): SurfaceConfig {
  try {
    const raw = localStorage.getItem(PRESET_KEY);
    if (!raw) return cloneSurface(DEFAULT_SURFACE);
    const parsed = JSON.parse(raw) as SurfaceConfig;
    // A corrupt or hand-edited preset must not brick the editor.
    if (!Array.isArray(parsed.keyframes) || parsed.keyframes.length === 0) {
      return cloneSurface(DEFAULT_SURFACE);
    }
    return parsed;
  } catch {
    return cloneSurface(DEFAULT_SURFACE);
  }
}

/** A slow synthetic spectrum, so the editor is not a dead flat line offline. */
function useDemoLevels(active: boolean): number[] {
  const [levels, setLevels] = useState<number[]>(() => new Array(48).fill(0));

  useEffect(() => {
    if (!active) return;
    let raf = 0;
    const start = performance.now();
    const tick = () => {
      raf = requestAnimationFrame(tick);
      const t = (performance.now() - start) / 1000;
      setLevels(
        Array.from({ length: 48 }, (_, i) => {
          const x = i / 47;
          const envelope = Math.exp(-x * 1.5);
          const wobble = 0.5 + 0.5 * Math.sin(t * 2 + x * 9);
          return Math.min(1, envelope * wobble * 1.4);
        }),
      );
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [active]);

  return levels;
}
