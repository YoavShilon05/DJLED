/**
 * WebSocket client for the engine.
 *
 * The engine is a headless service and this is just a client, so the UI can be
 * opened, reloaded or closed without the strip noticing. Reconnection is
 * automatic and the editor stays fully usable while disconnected — you can
 * author a stack with nothing running and it will be sent when the engine
 * appears.
 *
 * # Frames are dropped, not queued
 *
 * The engine publishes a frame every 33 ms whether anyone is keeping up or not.
 * That is the right call there — the analysis thread must never wait on a
 * browser — but it makes flow control this side's problem, and there is no
 * back pressure on a WebSocket to do it with: a message that arrives while the
 * main thread is busy waits in the event queue, holding its payload, and the
 * queue has no limit. A tab that takes longer than 33 ms to fold a stack and
 * repaint therefore falls behind by a few kilobytes per frame, for as long as
 * it is open, until it is killed for running out of memory.
 *
 * So a frame is parked rather than delivered, and the newest one is handed over
 * on the next animation frame. A frame is a whole snapshot rather than a delta,
 * so an older one holds nothing a newer one does not — dropping it costs
 * nothing but the redraw nobody would have seen. What this buys is that the
 * socket's own handler stays cheap, the queue drains as fast as it fills, and
 * at most one frame is ever held. A backgrounded tab, where animation frames
 * stop entirely, simply holds that one and does no work at all.
 *
 * State messages are not coalesced. There are few of them, each one is a change
 * the UI could not have predicted, and the editor's adoption rule reads
 * `activePreset` transitions — skipping one would lose the transition.
 */

import type { SurfaceConfig } from "./color/surface";
import type { CurveConfig } from "./config/curve";
import type { EqBand } from "./config/eq";
import type { LedKeyframe } from "./config/editor";

export const DEFAULT_URL = "ws://127.0.0.1:9001";

/**
 * The engine's `show::Layer`, verbatim.
 *
 * Everything that shapes one layer, including what it listens to. The source
 * travels here rather than in a command of its own because it is part of the
 * layer: two layers of one show can be watching two different devices, and a
 * reorder must carry each one's device with it.
 */
export interface LayerConfig {
  /** Stable across a reorder, which is how per-layer telemetry finds its row. */
  id: string;
  name: string;
  /** Off skips the layer entirely — not composited, and not analysed. */
  enabled: boolean;
  /** Master opacity, multiplied into every sample's own. The Photoshop slider:
   *  it fades toward what is underneath, not toward black. */
  opacity: number;
  source: InputSource;
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
 * The engine's `ShowConfig`, verbatim: a stack of layers, bottom first.
 *
 * The order *is* the compositing order, so a drag in the editor is a reorder of
 * this array and nothing else.
 */
export interface ShowConfig {
  layers: LayerConfig[];
}

/**
 * The engine's `midi::MidiConfig`, verbatim.
 *
 * Part of the layer rather than of the source selection: the note range and
 * spread are as much a part of a look as the colours are, so they are saved
 * with it and survive switching that layer to audio and back.
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
 * Which kind of endpoint a layer is listening to.
 *
 * A WASAPI endpoint is one direction or the other, never both — an interface
 * appears as a separate playback endpoint and capture endpoint, usually under
 * the same name. So the direction is a property of the device, not something
 * the user picks alongside it; it only matters on its own when no device is
 * named. MIDI ports are a third kind, listed alongside them because choosing
 * what drives a layer is one choice, not two.
 *
 * `"none"` is the fourth and is not a device at all: a layer that listens to
 * nothing paints a still colour. It is a kind rather than a flag beside the
 * selection because that is exactly what it is — one more answer to "what is
 * driving this layer", chosen from the same dropdown as the other three.
 */
export type SourceKind = "loopback" | "input" | "midi" | "none";

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

/**
 * What one layer's selection actually resolved to.
 *
 * The authored half is already in the config this editor sent; this is the half
 * only the engine can answer. A layer whose device would not open reports it
 * here and every other layer keeps running, which is the whole reason this is
 * per layer rather than per connection.
 */
export interface LayerStatus {
  /** Matches `LayerConfig.id`. */
  id: string;
  /** The endpoint the selection resolves to. Not redundant with the layer's
   *  own `source`: "the default output" names no device, and you still want to
   *  see which one you got. */
  deviceName: string;
  kind: SourceKind;
  channels: number;
  /** Zero for MIDI, which has no sample rate. */
  sampleRate: number;
  /** Bands for audio, semitones for MIDI. */
  points: number;
  /** Notes dropped for falling outside the note range. Null for audio. */
  notesOutOfRange: number | null;
  error: string | null;
}

/**
 * The engine's `presets::PresetInfo`, verbatim: one slot of the dropdown.
 *
 * The shows themselves never come with this. Twelve of them would be resent
 * every time a device is rescanned, and the editor only ever needs one — the
 * live one, which arrives as `EngineState.config` like it always has.
 */
export interface PresetInfo {
  name: string;
  /** Whether anything has been authored into this slot. An empty one opens a
   *  blank canvas from the dropdown and is declined by its hotkey. */
  stored: boolean;
}

/** One layer's analysis, as the editor plots it. */
export interface LayerFrame {
  id: string;
  levels: number[];
  /** Band centre frequencies in Hz, so the axis can be labelled correctly. */
  centers: number[];
}

export interface Frame {
  /** Bottom layer first, matching the show. */
  layers: LayerFrame[];
  /** The whole stack composited, as the firmware will drive it — hex-encoded
   *  RGB. The one thing no single layer can tell you. */
  strip: string;
  connected: boolean;
  droppedFrames: number;
}

export interface EngineState {
  config: ShowConfig;
  brightness: number;
  ledCount: number;
  /** Colours on the wire per frame, which is what the whole stack renders at. */
  pointCount: number;
  /** Analyser dB window. The EQ is authored in these terms, not the plot's. */
  dbFloor: number;
  dbCeil: number;
  /** Everything selectable, as of the last scan. Global rather than per layer:
   *  what exists does not depend on who is listening to it. */
  devices: InputDevice[];
  /** What each layer resolved to, bottom first. */
  layers: LayerStatus[];
  /** The twelve slots in hotkey order: index 0 is ctrl+alt+F1. */
  presets: PresetInfo[];
  /**
   * Which slot `config` came from, and which one edits are saved into.
   *
   * The editor watches this. A change in it is the only signal that the show
   * was replaced by something other than this editor's own hands — a global
   * hotkey, or a second browser tab — and the one case where adopting the
   * engine's config rather than pushing our own is the correct move.
   */
  activePreset: number;
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
  /** The newest frame not yet handed on. See the module note. */
  private pending: Frame | null = null;
  private raf: number | null = null;

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
          this.queueFrame(msg as Frame);
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

  /**
   * Hold a frame until the next animation frame, replacing whatever was already
   * waiting.
   *
   * The parse happens here rather than at flush time because it is the cheap
   * half — a few tens of microseconds against a whole React pass — and doing it
   * eagerly keeps the socket handler free of any knowledge of the wire format
   * beyond the tag it already reads.
   */
  private queueFrame(frame: Frame): void {
    this.pending = frame;
    if (this.raf !== null) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = null;
      const next = this.pending;
      this.pending = null;
      if (next) this.handlers.onFrame?.(next);
    });
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
   * Push the whole show — every layer, including what each one listens to.
   *
   * Sent as one message even on the hot path, where dragging a keyframe
   * produces one of these per pointer move. The engine compares each stage
   * against what it already has, so the cost is a few float comparisons, and a
   * single message means the two sides cannot disagree about which half of an
   * edit landed. That matters more with a stack than without one: a reorder and
   * a recolour can arrive in the same gesture.
   *
   * Moving a layer to a different device is part of this message rather than a
   * command of its own. The engine opens the new stream before dropping the old
   * one, so a device that cannot be opened leaves that layer where it was and
   * comes back as an error against it in the next `state` — every other layer
   * carries on regardless.
   */
  setConfig(config: ShowConfig): void {
    this.send({ type: "config", config });
  }

  setBrightness(value: number): void {
    this.send({ type: "brightness", value });
  }

  /** Rescan the endpoints, e.g. after plugging an interface in. Also retries
   *  any layer whose device would not open. */
  listSources(): void {
    this.send({ type: "listSources" });
  }

  /**
   * Make a preset slot live.
   *
   * Deliberately does *not* send a config with it. The engine holds the twelve
   * shows — they have to survive with no browser open, which is the whole point
   * of the global hotkeys — so this asks for a switch and the new show arrives
   * in the next `state`, exactly as it does when somebody hits ctrl+alt+F<n>
   * instead. One path, so the two cannot drift.
   */
  selectPreset(slot: number): void {
    this.send({ type: "selectPreset", slot });
  }

  /** Rename a slot. Names label the dropdown and nothing else. */
  renamePreset(slot: number, name: string): void {
    this.send({ type: "renamePreset", slot, name });
  }

  close(): void {
    this.closed = true;
    if (this.timer !== null) {
      window.clearTimeout(this.timer);
      this.timer = null;
    }
    if (this.raf !== null) {
      cancelAnimationFrame(this.raf);
      this.raf = null;
    }
    // Dropped rather than delivered: a closed client has no one to deliver to,
    // and holding the last frame would keep a whole stack's levels alive for as
    // long as anything still references the client.
    this.pending = null;
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
