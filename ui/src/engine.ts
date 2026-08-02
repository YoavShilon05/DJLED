/**
 * WebSocket client for the engine.
 *
 * The engine is a headless service and this is just a client, so the UI can be
 * opened, reloaded or closed without the strip noticing. Reconnection is
 * automatic and the editor stays fully usable while disconnected — you can
 * author a palette with nothing running and it will be sent when the engine
 * appears.
 */

import type { SurfaceConfig } from "./color/surface";

export const DEFAULT_URL = "ws://127.0.0.1:9001";

export interface Frame {
  levels: number[];
  /** Band centre frequencies in Hz, so the axis can be labelled correctly. */
  centers: number[];
  /** Strip as the firmware will drive it, hex-encoded RGB. */
  strip: string;
  sampleRate: number;
  connected: boolean;
  droppedFrames: number;
}

export interface EngineState {
  surface: SurfaceConfig;
  brightness: number;
  ledCount: number;
  bandCount: number;
}

export type Status = "connecting" | "connected" | "offline";

export interface Handlers {
  onFrame?: (frame: Frame) => void;
  onState?: (state: EngineState) => void;
  onStatus?: (status: Status) => void;
  onError?: (message: string) => void;
}

const RECONNECT_MS = 1500;

export class EngineClient {
  private socket: WebSocket | null = null;
  private timer: number | null = null;
  private closed = false;

  constructor(
    private url: string,
    private handlers: Handlers,
  ) {}

  connect(): void {
    this.closed = false;
    this.open();
  }

  private open(): void {
    if (this.closed) return;
    this.handlers.onStatus?.("connecting");

    let socket: WebSocket;
    try {
      socket = new WebSocket(this.url);
    } catch {
      // Constructing can throw on a malformed URL; treat it as a failed attempt
      // so the retry loop keeps running rather than dying silently.
      this.scheduleReconnect();
      return;
    }
    this.socket = socket;

    socket.onopen = () => {
      this.handlers.onStatus?.("connected");
      this.send({ type: "requestState" });
    };

    socket.onmessage = (event) => {
      let msg: unknown;
      try {
        msg = JSON.parse(event.data as string);
      } catch {
        return;
      }
      if (typeof msg !== "object" || msg === null) return;

      const tagged = msg as { type?: string };
      switch (tagged.type) {
        case "frame":
          this.handlers.onFrame?.(msg as Frame);
          break;
        case "state":
          this.handlers.onState?.(msg as EngineState);
          break;
        case "error":
          this.handlers.onError?.((msg as { message: string }).message);
          break;
      }
    };

    socket.onclose = () => {
      this.socket = null;
      this.handlers.onStatus?.("offline");
      this.scheduleReconnect();
    };

    // `onerror` is always followed by `onclose`, so reconnection is handled
    // there and this only avoids an unhandled event.
    socket.onerror = () => {};
  }

  private scheduleReconnect(): void {
    if (this.closed || this.timer !== null) return;
    this.timer = window.setTimeout(() => {
      this.timer = null;
      this.open();
    }, RECONNECT_MS);
  }

  private send(payload: unknown): void {
    if (this.socket?.readyState === WebSocket.OPEN) {
      this.socket.send(JSON.stringify(payload));
    }
  }

  setSurface(surface: SurfaceConfig): void {
    this.send({ type: "surface", surface });
  }

  setBrightness(value: number): void {
    this.send({ type: "brightness", value });
  }

  close(): void {
    this.closed = true;
    if (this.timer !== null) {
      window.clearTimeout(this.timer);
      this.timer = null;
    }
    this.socket?.close();
    this.socket = null;
  }
}

/** Decode the hex strip payload into RGB triplets. */
export function decodeStrip(hex: string): Array<[number, number, number]> {
  const out: Array<[number, number, number]> = [];
  for (let i = 0; i + 6 <= hex.length; i += 6) {
    out.push([
      parseInt(hex.slice(i, i + 2), 16),
      parseInt(hex.slice(i + 2, i + 4), 16),
      parseInt(hex.slice(i + 4, i + 6), 16),
    ]);
  }
  return out;
}
