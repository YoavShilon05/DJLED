import { describe, expect, it } from "vitest";

import { ColorSurface } from "../color/surface";
import {
  defaultLayer,
  fromEngineConfig,
  toEngineConfig,
  toRenderTimeline,
  toTimeline,
  type ColorKeyframe,
  type EditorConfig,
  type EditorLayer,
} from "./editor";
import {
  BASE_KEY,
  KEY_MAX,
  KEY_MIN,
  addKey,
  fieldAt,
  fieldLayer,
  isAnimated,
  keyOf,
  keysOf,
  mergeKeyEdit,
  moveKey,
  removeKey,
  withTimeline,
} from "./timeline";

/** A layer with two colours, so an edit to one is visibly not an edit to both. */
function layer(): EditorLayer {
  const base = defaultLayer("Test");
  return {
    ...base,
    colorKeyframes: [
      { id: "a", hz: 100, db: -40, color: "#ff0000", radius: null },
      { id: "b", hz: 8000, db: -10, color: "#0000ff", radius: 0.4 },
    ],
  };
}

/** The same layer with a second key halfway round the loop. */
function looping(): EditorLayer {
  return addKey(layer(), 0.5, "k1");
}

function colors(frames: ColorKeyframe[]): string[] {
  return frames.map((k) => k.color);
}

describe("a layer without keys", () => {
  /**
   * The compatibility promise, and the reason the first key is the layer's own
   * field rather than an entry in a list: a show that predates timelines is not
   * migrated, it simply has no keys.
   */
  it("is the still layer it always was", () => {
    const l = layer();
    expect(isAnimated(l)).toBe(false);
    expect(keysOf(l)).toHaveLength(1);
    expect(keysOf(l)[0].base).toBe(true);
    expect(toTimeline(l).keys).toEqual([]);
    // And the graph is handed the layer itself, not a copy of it.
    expect(fieldLayer(l, BASE_KEY)).toBe(l);
  });

  it("falls back to the start of the loop for a key id naming nothing", () => {
    // Which is what makes selecting a different layer safe: its key ids mean
    // nothing here, and neither does a stale one out of localStorage.
    expect(keyOf(layer(), "gone").base).toBe(true);
    expect(keyOf(looping(), "gone").base).toBe(true);
    expect(keyOf(looping(), "k1").at).toBe(0.5);
  });
});

describe("adding a key", () => {
  /**
   * The behaviour every animation tool has, and the one that makes a timeline
   * safe to subdivide mid-set: dropping a key gives the next edit somewhere to
   * land and changes nothing about what the loop currently does.
   */
  it("holds the field the loop already shows there", () => {
    const l = looping();
    const before = fieldAt(l, 0.25);
    const after = addKey(l, 0.25, "k2");

    expect(keysOf(after)).toHaveLength(3);
    const added = keyOf(after, "k2");
    expect(colors(added.colorKeyframes)).toEqual(colors(before.colorKeyframes));
    expect(added.colorKeyframes.map((k) => Math.round(k.hz))).toEqual(
      before.colorKeyframes.map((k) => Math.round(k.hz)),
    );
    // Seeded from the interpolation, so what the loop shows at that instant is
    // unchanged by the key existing.
    expect(colors(fieldAt(after, 0.25).colorKeyframes)).toEqual(colors(before.colorKeyframes));
  });

  it("keeps keys inside the loop and in order", () => {
    let l = looping();
    l = addKey(l, 5, "late");
    l = addKey(l, -3, "early");

    const ats = keysOf(l).map((k) => k.at);
    expect(ats[0]).toBe(0);
    expect(Math.max(...ats)).toBeLessThanOrEqual(KEY_MAX);
    expect(Math.min(...ats.slice(1))).toBeGreaterThanOrEqual(KEY_MIN);
    expect([...ats].sort((a, b) => a - b)).toEqual(ats);
  });

  it("cannot move or drop the start of the loop", () => {
    // It is the layer's own field, so there is nothing to move it off and
    // nothing to delete it into.
    const l = looping();
    expect(moveKey(l, BASE_KEY, 0.7)).toBe(l);
    expect(removeKey(l, BASE_KEY)).toBe(l);
  });

  it("leaves a still layer when the last key is dropped", () => {
    const l = removeKey(looping(), "k1");
    expect(isAnimated(l)).toBe(false);
    expect(toTimeline(l).keys).toEqual([]);
  });
});

describe("where an edit lands", () => {
  /**
   * The split the whole model rests on: a colour's *position* belongs to the
   * key being edited, and the *set* of colours belongs to the layer.
   */
  it("moves a colour in one key only", () => {
    const l = looping();
    const edited = fieldLayer(l, "k1");
    const moved = {
      ...edited,
      colorKeyframes: edited.colorKeyframes.map((k) =>
        k.id === "a" ? { ...k, hz: 12000, color: "#00ff00" } : k,
      ),
    };

    const next = mergeKeyEdit(l, "k1", moved);
    expect(keyOf(next, "k1").colorKeyframes[0].color).toBe("#00ff00");
    // The start of the loop is untouched, which is the whole point of a key.
    expect(next.colorKeyframes[0].color).toBe("#ff0000");
    expect(next.colorKeyframes[0].hz).toBe(100);
  });

  it("adds and deletes a colour across every key", () => {
    const l = looping();
    const edited = fieldLayer(l, "k1");

    const added: ColorKeyframe = { id: "c", hz: 1000, db: -20, color: "#ffffff", radius: null };
    const withAdded = mergeKeyEdit(l, "k1", {
      ...edited,
      colorKeyframes: [...edited.colorKeyframes, added],
    });
    for (const key of keysOf(withAdded)) {
      expect(key.colorKeyframes.map((k) => k.id)).toEqual(["a", "b", "c"]);
    }

    const stillEdited = fieldLayer(withAdded, "k1");
    const withDropped = mergeKeyEdit(withAdded, "k1", {
      ...stillEdited,
      colorKeyframes: stillEdited.colorKeyframes.filter((k) => k.id !== "a"),
    });
    for (const key of keysOf(withDropped)) {
      expect(key.colorKeyframes.map((k) => k.id)).toEqual(["b", "c"]);
    }
  });

  /**
   * Keeping the set identical across keys is not cosmetic: two keys are blended
   * by position in the list, because a keyframe carries no id over the wire.
   */
  it("keeps every key the same length, which is what the engine blends on", () => {
    const l = looping();
    const edited = fieldLayer(l, BASE_KEY);
    const next = mergeKeyEdit(l, BASE_KEY, {
      ...edited,
      colorKeyframes: [
        ...edited.colorKeyframes,
        { id: "c", hz: 1000, db: -20, color: "#ffffff", radius: null },
      ],
    });

    const lengths = toTimeline(next).keys.map((k) => k.surface.keyframes.length);
    expect(new Set(lengths).size).toBe(1);
  });

  it("carries a layer-wide edit through whichever key is selected", () => {
    // Everything that is not the colour field belongs to the layer, so editing
    // it while a key other than the first is selected must still reach it.
    const l = looping();
    const next = mergeKeyEdit(l, "k1", { ...fieldLayer(l, "k1"), mirror: true, decay: 0.5 });
    expect(next.mirror).toBe(true);
    expect(next.decay).toBe(0.5);
  });

  it("gives each key its own blend radius", () => {
    const l = looping();
    const next = mergeKeyEdit(l, "k1", { ...fieldLayer(l, "k1"), blend: 0.05 });
    expect(keyOf(next, "k1").blend).toBe(0.05);
    expect(next.blend).not.toBe(0.05);
  });
});

describe("the wire form", () => {
  /**
   * The mirror an older peer reads is *derived* from the first key rather than
   * stored beside it, so the two cannot disagree however the loop is edited.
   */
  it("writes the layer's surface as the field the loop starts from", () => {
    const l = looping();
    const wire = toEngineConfig({ layers: [l] } as EditorConfig).layers[0];
    expect(wire.timeline.keys).toHaveLength(2);
    expect(wire.surface).toEqual(wire.timeline.keys[0].surface);
    expect(wire.timeline.keys[0].at).toBe(0);
    expect(wire.timeline.keys[1].at).toBe(0.5);
  });

  it("sends no keys for a layer that does not animate", () => {
    const wire = toEngineConfig({ layers: [layer()] } as EditorConfig).layers[0];
    expect(wire.timeline.keys).toEqual([]);
  });

  it("holds the field at the start when the loop is switched off", () => {
    const off = withTimeline(looping(), { enabled: false });
    const wire = toTimeline(off);
    // The keys travel — switching it off is not a delete — and the engine holds
    // the first of them.
    expect(wire.enabled).toBe(false);
    expect(wire.keys).toHaveLength(2);
    expect(ColorSurface.forLayer(wire.keys[0].surface, wire).animates()).toBe(false);
  });

  it("survives a round trip through the engine", () => {
    const l = looping();
    const config = { layers: [l] } as EditorConfig;
    const back = fromEngineConfig(config, toEngineConfig(config));
    const restored = back.layers[0];

    expect(restored.timeline.keys).toHaveLength(1);
    expect(restored.timeline.keys[0].at).toBe(0.5);
    expect(restored.timeline.length).toBe(l.timeline.length);
    expect(colors(restored.colorKeyframes)).toEqual(colors(l.colorKeyframes));
    // Ids are minted fresh, but the *same* id has to appear at the same
    // position in every key — that correspondence is the editor's half of what
    // the engine blends on.
    expect(restored.timeline.keys[0].colorKeyframes.map((k) => k.id)).toEqual(
      restored.colorKeyframes.map((k) => k.id),
    );
  });

  /**
   * A still layer has no level axis, so every key of its loop is read along one
   * row — the whole-timeline form of the projection `toRenderSurface` does.
   */
  it("flattens every key of a still layer", () => {
    const still: EditorLayer = { ...looping(), source: { id: null, kind: "none", channel: null } };
    const flat = toRenderTimeline(still);
    expect(flat.keys).toHaveLength(2);
    for (const key of flat.keys) {
      expect(key.surface.keyframes.every((k) => k.y === 1)).toBe(true);
    }
    // And the stored form is untouched: the authored level is not wrong, it is
    // simply not being read.
    expect(toTimeline(still).keys[0].surface.keyframes.some((k) => k.y !== 1)).toBe(true);
  });
});
