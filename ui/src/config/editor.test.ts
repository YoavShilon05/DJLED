import { beforeEach, describe, expect, it } from "vitest";

import {
  DEFAULT_CONFIG,
  MAX_LAYERS,
  activeLayer,
  defaultLayer,
  fromEngineConfig,
  loadConfig,
  nextId,
  saveConfig,
  toEngineConfig,
  toRenderSurface,
  toSurface,
  withLayer,
  type EditorConfig,
} from "./editor";

const STORAGE_KEY = "djled.editor";

/** vitest runs in node by default, so localStorage has to be stood up. */
beforeEach(() => {
  const store = new Map<string, string>();
  globalThis.localStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
    key: (i: number) => [...store.keys()][i] ?? null,
    get length() {
      return store.size;
    },
  } as Storage;
});

describe("stored configs", () => {
  /**
   * The migration that matters. Everything saved before layers existed has one
   * layer's worth of fields at the top level, and it has to load as the look it
   * was rather than being silently replaced by the default.
   */
  it("folds a pre-stack config into one layer", () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        colorKeyframes: [{ id: "ck-1", hz: 440, db: -20, color: "#ff2000" }],
        ledKeyframes: [
          { id: "led-1", led: 0, hz: 20 },
          { id: "led-2", led: 149, hz: 20_000 },
        ],
        eq: [{ id: "eq-1", type: "peak", hz: 2000, gain: 6, q: 1.4 }],
        reverse: true,
        mirror: true,
        threshold: -55,
        clamp: -12,
        decay: 0.9,
        sampleLength: 1024,
        blend: 0.4,
        midi: { lowNote: 36, highNote: 96, spread: 0.5, sustain: false },
        gizmos: { eq: false, thresholds: true, colorKeyframes: true, ledKeyframes: true, curve: true },
      }),
    );

    const config = loadConfig();
    expect(config.layers).toHaveLength(1);

    const layer = config.layers[0];
    expect(layer.reverse).toBe(true);
    expect(layer.mirror).toBe(true);
    expect(layer.threshold).toBe(-55);
    expect(layer.clamp).toBe(-12);
    expect(layer.decay).toBe(0.9);
    expect(layer.sampleLength).toBe(1024);
    expect(layer.blend).toBe(0.4);
    expect(layer.eq).toHaveLength(1);
    expect(layer.colorKeyframes[0].color).toBe("#ff2000");
    expect(layer.midi.lowNote).toBe(36);

    // Fields the old shape had no place for take the layer default.
    expect(layer.enabled).toBe(true);
    expect(layer.opacity).toBe(1);
    expect(layer.source.kind).toBe("loopback");

    // Gizmos were always editor-wide and stay there.
    expect(config.gizmos.eq).toBe(false);
    // The one layer is what edits land on, since there is nothing else.
    expect(activeLayer(config).id).toBe(layer.id);
  });

  it("round-trips a stack through storage", () => {
    const a = defaultLayer("Bass");
    const b = { ...defaultLayer("Keys"), opacity: 0.4, enabled: false };
    const original: EditorConfig = { ...DEFAULT_CONFIG, layers: [a, b], activeLayerId: b.id };

    saveConfig(original);
    const back = loadConfig();

    expect(back.layers.map((l) => l.name)).toEqual(["Bass", "Keys"]);
    expect(back.activeLayerId).toBe(b.id);
    expect(back.layers[1].opacity).toBe(0.4);
    expect(back.layers[1].enabled).toBe(false);
  });

  it("falls back to the defaults for junk", () => {
    localStorage.setItem(STORAGE_KEY, "{not json");
    expect(loadConfig()).toEqual(DEFAULT_CONFIG);
  });

  /**
   * Ids are how a row finds its live telemetry and its selection. Two layers
   * sharing one would have two rows fighting over one plot, so a duplicate is
   * repaired rather than trusted — hand-edited storage is a real source of this.
   */
  it("repairs duplicate layer ids", () => {
    const layer = defaultLayer("One");
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ layers: [layer, { ...layer, name: "Two" }], activeLayerId: layer.id }),
    );

    const config = loadConfig();
    expect(config.layers).toHaveLength(2);
    expect(config.layers[0].id).not.toBe(config.layers[1].id);
  });

  /**
   * An empty colour field is a layer that paints nothing, which is a state the
   * editor passes through whenever a palette is replaced one keyframe at a
   * time. Restoring the default palette behind the user's back would undo the
   * deletions they just made.
   */
  it("keeps a colour field that was emptied on purpose", () => {
    const layer = { ...defaultLayer("Blank"), colorKeyframes: [] };
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ layers: [layer], activeLayerId: layer.id }),
    );
    expect(loadConfig().layers[0].colorKeyframes).toEqual([]);
  });

  /** Sectors are the opposite case: fewer than two spans no LEDs at all. */
  it("never loads an empty stack", () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ layers: [] }));
    expect(loadConfig().layers.length).toBeGreaterThan(0);
  });

  it("caps the stack at the documented maximum", () => {
    const layers = Array.from({ length: MAX_LAYERS + 5 }, (_, i) => defaultLayer(`L${i}`));
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ layers }));
    expect(loadConfig().layers).toHaveLength(MAX_LAYERS);
  });

  /** A selection naming a layer that is gone must land somewhere real. */
  it("recovers an active layer that no longer exists", () => {
    const layer = defaultLayer("Only");
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ layers: [layer], activeLayerId: "vanished" }),
    );
    expect(loadConfig().activeLayerId).toBe(layer.id);
  });
});

describe("the wire format", () => {
  it("sends the stack in order, sources and all", () => {
    const a = { ...defaultLayer("Bass"), id: nextId("layer") };
    const b = {
      ...defaultLayer("Keys"),
      id: nextId("layer"),
      opacity: 0.5,
      source: { id: "midi:x", kind: "midi" as const, channel: 3 },
    };
    const show = toEngineConfig({ ...DEFAULT_CONFIG, layers: [a, b], activeLayerId: a.id });

    expect(show.layers.map((l) => l.id)).toEqual([a.id, b.id]);
    expect(show.layers[1].opacity).toBe(0.5);
    expect(show.layers[1].source).toEqual({ id: "midi:x", kind: "midi", channel: 3 });
    // Colour keyframes go as the normalised surface, which is the form the
    // renderer samples.
    expect(show.layers[0].surface.keyframes[0].x).toBeGreaterThanOrEqual(0);
    expect(show.layers[0].surface.keyframes[0].x).toBeLessThanOrEqual(1);
  });

  /**
   * An area of effect has to survive both directions, and an unconfined
   * keyframe has to cross the wire with no radius on it at all — that absence
   * is what every show written before the feature relies on meaning
   * "everywhere".
   */
  it("carries an area of effect both ways, and omits one that is not set", () => {
    const base = defaultLayer("Confined");
    const layer = {
      ...base,
      colorKeyframes: [
        { ...base.colorKeyframes[0], radius: 0.4 },
        { ...base.colorKeyframes[1], radius: null },
      ],
    };

    const show = toEngineConfig({ ...DEFAULT_CONFIG, layers: [layer], activeLayerId: layer.id });
    expect(show.layers[0].surface.keyframes[0].radius).toBe(0.4);
    expect("radius" in show.layers[0].surface.keyframes[1]).toBe(false);

    const back = fromEngineConfig(DEFAULT_CONFIG, show);
    expect(back.layers[0].colorKeyframes[0].radius).toBe(0.4);
    expect(back.layers[0].colorKeyframes[1].radius).toBe(null);
  });

  /**
   * The colour cycle is a layer setting, so it has to survive both directions —
   * and an engine that predates it sends nothing, which has to read as off
   * rather than as undefined finding its way into the renderer.
   */
  it("carries the colour cycle both ways, and reads an absent one as off", () => {
    const layer = { ...defaultLayer("Chase"), cycle: true };
    const show = toEngineConfig({ ...DEFAULT_CONFIG, layers: [layer], activeLayerId: layer.id });
    expect(show.layers[0].cycle).toBe(true);
    expect(fromEngineConfig(DEFAULT_CONFIG, show).layers[0].cycle).toBe(true);

    const older = { layers: [{ ...show.layers[0], cycle: undefined as unknown as boolean }] };
    expect(fromEngineConfig(DEFAULT_CONFIG, older).layers[0].cycle).toBe(false);
  });

  /** Adopting the engine's stack must not leave the selection pointing at a
   *  layer that no longer exists. */
  it("adopts an engine stack and reselects", () => {
    const local = DEFAULT_CONFIG;
    const show = toEngineConfig({
      ...DEFAULT_CONFIG,
      layers: [defaultLayer("A"), defaultLayer("B")],
      activeLayerId: "gone",
    });

    const adopted = fromEngineConfig(local, show);
    expect(adopted.layers.map((l) => l.name)).toEqual(["A", "B"]);
    expect(adopted.layers.some((l) => l.id === adopted.activeLayerId)).toBe(true);
    // Editor-only settings survive.
    expect(adopted.gizmos).toEqual(local.gizmos);
  });

  it("replaces exactly one layer and keeps the order", () => {
    const a = defaultLayer("A");
    const b = defaultLayer("B");
    const config: EditorConfig = { ...DEFAULT_CONFIG, layers: [a, b], activeLayerId: a.id };

    const next = withLayer(config, { ...a, name: "renamed" });
    expect(next.layers.map((l) => l.name)).toEqual(["renamed", "B"]);
    expect(next.layers[1]).toBe(b);
  });
});

describe("a layer with no source", () => {
  const still = () => ({
    ...defaultLayer("still"),
    source: { id: null, kind: "none" as const, channel: null },
    colorKeyframes: [
      { id: "ck-1", hz: 100, db: -70, color: "#ff2000", radius: null },
      { id: "ck-2", hz: 5000, db: -10, color: "#40c0ff", radius: null },
    ],
  });

  /**
   * The flattening is a property of how the field is *read*, not of how it is
   * stored. Writing it to the wire instead would mean a layer switched to no
   * source and back had lost the two dimensional field it was authored with —
   * and the engine keeps every config it is sent, so one preset switch would
   * make that permanent.
   */
  it("sends the authored keyframes and reads a flattened field", () => {
    const layer = still();
    expect(toSurface(layer).keyframes.map((k) => k.y)).not.toEqual([1, 1]);
    expect(toRenderSurface(layer).keyframes.map((k) => k.y)).toEqual([1, 1]);
    // The positions along the strip, and the colours, are untouched by it.
    expect(toRenderSurface(layer).keyframes.map((k) => k.x)).toEqual(
      toSurface(layer).keyframes.map((k) => k.x),
    );
  });

  it("leaves a layer with a device two dimensional", () => {
    const layer = { ...still(), source: { id: null, kind: "loopback" as const, channel: null } };
    expect(toRenderSurface(layer)).toEqual(toSurface(layer));
  });

  it("survives a round trip to the engine and back", () => {
    const layer = still();
    const config: EditorConfig = { ...DEFAULT_CONFIG, layers: [layer], activeLayerId: layer.id };
    const back = fromEngineConfig(config, toEngineConfig(config));
    expect(back.layers[0].source.kind).toBe("none");
    // Including the dB the wire carried but nothing read.
    expect(back.layers[0].colorKeyframes.map((k) => Math.round(k.db))).toEqual([-70, -10]);
  });
});
