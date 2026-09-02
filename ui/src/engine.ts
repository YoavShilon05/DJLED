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
import type { CurveConfig } from "./config/curve";
import type { EqBand } from "./config/eq";
import type { LedKeyframe } from "./config/editor";

export const DEFAULT_URL = "ws://127.0.0.1:9001";

/**
 * The engine's `ShowConfig`, verbatim.
 *
 * Everything the editor holds that the engine acts on, in one message. The
 * colour keyframes travel as the normalised surface because that is the form
 * the renderer samples; the rest is sent as authored.
 */
export interface ShowConfig {
  surface: SurfaceConfig;
  eq: EqBand[];
  ledKeyframes: LedKeyframe[];
  reverse: boolean;
  mirror: boolean;
  threshold: number;
  clamp: number;
  curve: CurveConfig;
  decay: number;
  sampleLength: number;
  midi: MidiConfig;
}

/**
 * The engine's `midi::MidiConfig`, verbatim.
 *
 * Part of the show rather than of the source selection: the note range and
 * spread are as much a part of a look as the colours are, so they are saved
 * with it and survive switching to audio and back.
 */
export interface MidiConfig {
  /** Note at the left end of the axis. */
  lowNote: number;
  /** Note at the right end. */
  highNote: number;
  /** How far a note bleeds into its neighbours, in semitones. */
  spread: number;
  /** Honour the sustain pedal (CC64). */
  sustain: boolean;
}

/**
 * Which kind of endpoint is being listened to.
 *
 * A WASAPI endpoint is one direction or the other, never both — an interface
 * appears as a separate playback endpoint and capture endpoint, usually under
 * the same name. So the direction is a property of the device, not something
 * the user picks alongside it; it only matters on its own when no device is
 * named. MIDI ports are a third kind, listed alongside them because choosing
 * what drives the strip is one choice, not two.
 */
export type SourceKind = "loopback" | "input" | "midi";

/** The engine's `source::Source`, verbatim. */
export interface InputSource {
  /** Device or port id, or null to follow the default for this kind. */
  id: string | null;
  kind: SourceKind;
  /** Channel to listen to, 0-based. Null takes them all — mixed down for
   *  audio, merged for MIDI. */
  channel: number | null;
}

export interface InputDevice {
  id: string;
  name: string;
  kind: SourceKind;
  isDefault: boolean;
  /** Null for MIDI, which has no sample rate, and for an audio endpoint that
   *  would not open to be asked — usually because something else holds it. */
  sampleRate: number | null;
  channels: number | null;
}

export interface InputState {
  devices: InputDevice[];
  /** The live selection, as resolved rather than as requested. */
  source: InputSource;
  /** The endpoint the selection currently resolves to. Not redundant with
   *  `source`: "the default output" names no device, and you still want to see
   *  which one you got. */
  deviceName: string;
  kind: SourceKind;
  channels: number;
  /** Zero for MIDI, which has no sample rate. */
  sampleRate: number;
  error: string | null;
}

export interface Frame {
  levels: number[];
  /** Band centre frequencies in Hz, so the axis can be labelled correctly. */
  centers: number[];
  /** Strip as the firmware will drive it, hex-encoded RGB. */
  strip: string;
  sampleRate: number;
  connected: boolean;
  droppedFrames: number;
  /** Notes seen outside the configured range. Live rather than state: it is
   *  the answer to "why is the strip dark", and it changes while someone is
   *  playing rather than when they change a setting. */
  notesOutOfRange: number;
}

export interface EngineState {
  config: ShowConfig;
  brightness: number;
  ledCount: number;
  bandCount: number;
  /** Analyser dB window. The EQ is authored in these terms, not the plot's. */
  dbFloor: number;
  dbCeil: number;
  input: InputState;
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

  /**
   * Push the whole configuration.
   *
   * Sent as one message even on the hot path — dragging a keyframe produces one
   * of these per pointer move. The engine compares each stage against what it
   * already has, so the cost is a few float comparisons, and a single message
   * means the two sides cannot disagree about which half of an edit landed.
   */
  setConfig(config: ShowConfig): void {
    this.send({ type: "config", config });
  }

  setBrightness(value: number): void {
    this.send({ type: "brightness", value });
  }

  /**
   * Listen to a different endpoint.
   *
   * The engine opens the new stream before dropping the old one, so a source
   * that cannot be opened leaves capture running and comes back as an error in
   * the next `state` message rather than as silence.
   */
  setSource(source: InputSource): void {
    this.send({ type: "setSource", source });
  }

  /** Rescan the endpoints, e.g. after plugging an interface in. */
  listSources(): void {
    this.send({ type: "listSources" });
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
