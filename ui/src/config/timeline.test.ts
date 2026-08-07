import { describe, expect, it } from "vitest";

import {
  addKeyAt,
  phaseOf,
  segmentAt,
  stateAt,
  writeStateAt,
} from "./timeline";
import {
  defaultSpectrumState,
  makeLayer,
  nextId,
  type LayerKey,
  type SpectrumLayer,
  type SpectrumState,
} from "./layers";

function spectrumLayer(loopSeconds = 8): SpectrumLayer {
  const layer = makeLayer("spectrum") as SpectrumLayer;
  return { ...layer, loopSeconds };
}

function keyAt(t: number, state: SpectrumState): LayerKey<SpectrumState> {
  return { id: nextId("key"), t, state };
}

/** A state that differs from the default in one easily-asserted number. */
function withThreshold(db: number): SpectrumState {
  return { ...defaultSpectrumState(), threshold: db };
}

describe("phase", () => {
  it("wraps forwards and backwards", () => {
    expect(phaseOf(2, 8)).toBe(2);
    expect(phaseOf(10, 8)).toBe(2);
    // JS `%` returns a negative for a negative left operand, which is not a
    // phase — a scrub that lands just before zero must not fall off the ring.
    expect(phaseOf(-2, 8)).toBe(6);
  });

  it("is zero for a loop with no length", () => {
    expect(phaseOf(3, 0)).toBe(0);
  });
});

describe("segments", () => {
  const a = keyAt(0, withThreshold(-80));
  const b = keyAt(4, withThreshold(-40));

  it("interpolates between the two surrounding keys", () => {
    const seg = segmentAt([a, b], 2, 8);
    expect(seg?.a.t).toBe(0);
    expect(seg?.b.t).toBe(4);
    expect(seg?.t).toBeCloseTo(0.5, 10);
  });

  /**
   * The loop is a ring, not a line: past the last key the segment wraps to the
   * first, so an animation returns to where it started without the author
   * duplicating a key at the end.
   */
  it("wraps from the last key back to the first", () => {
    const seg = segmentAt([a, b], 6, 8);
    expect(seg?.a.t).toBe(4);
    expect(seg?.b.t).toBe(0);
    expect(seg?.t).toBeCloseTo(0.5, 10);
  });

  it("covers the phase before the first key with the wrap segment", () => {
    const late = keyAt(2, withThreshold(-80));
    const seg = segmentAt([late, keyAt(6, withThreshold(-40))], 0, 8);
    expect(seg?.a.t).toBe(6);
    expect(seg?.b.t).toBe(2);
    // 2 s into a 4 s wrap segment that started at t=6.
    expect(seg?.t).toBeCloseTo(0.5, 10);
  });

  it("holds a single key across the whole loop", () => {
    const seg = segmentAt([a], 5, 8);
    expect(seg?.a).toBe(a);
    expect(seg?.t).toBe(0);
  });
});

describe("state at a time", () => {
  it("is the layer's own state when nothing is keyed", () => {
    const layer = spectrumLayer();
    expect(stateAt(layer, 3)).toBe(layer.state);
  });

  it("tweens between snapshots", () => {
    const layer: SpectrumLayer = {
      ...spectrumLayer(8),
      keys: [keyAt(0, withThreshold(-80)), keyAt(4, withThreshold(-40))],
    };
    expect((stateAt(layer, 2) as SpectrumState).threshold).toBeCloseTo(-60, 10);
  });

  /**
   * Matching by id and letting an unmatched item hold its own value is what
   * stops a keyframe added at one snapshot from popping in halfway through the
   * segment before it — the count stays stable across a segment.
   */
  it("holds a keyframe that exists in only one of the two keys", () => {
    const base = defaultSpectrumState();
    const extra = { id: "extra", hz: 1000, db: -20, color: "#ff0000" };
    const layer: SpectrumLayer = {
      ...spectrumLayer(8),
      keys: [
        keyAt(0, base),
        keyAt(4, { ...base, colorKeyframes: [...base.colorKeyframes, extra] }),
      ],
    };
    const mid = stateAt(layer, 2) as SpectrumState;
    expect(mid.colorKeyframes.find((k) => k.id === "extra")).toEqual(extra);
    expect(mid.colorKeyframes).toHaveLength(base.colorKeyframes.length + 1);
  });

  it("takes each layer's phase from the shared clock, so loops stay in step", () => {
    const fast: SpectrumLayer = {
      ...spectrumLayer(2),
      keys: [keyAt(0, withThreshold(-80)), keyAt(1, withThreshold(-40))],
    };
    const slow: SpectrumLayer = {
      ...spectrumLayer(8),
      keys: [keyAt(0, withThreshold(-80)), keyAt(4, withThreshold(-40))],
    };
    // At t=8 both are back at the start together: 8 is a whole number of cycles
    // of each. That is the whole claim behind one clock rather than several.
    expect((stateAt(fast, 8) as SpectrumState).threshold).toBeCloseTo(-80, 10);
    expect((stateAt(slow, 8) as SpectrumState).threshold).toBeCloseTo(-80, 10);
  });
});

describe("authoring", () => {
  it("edits the state directly while the layer has no keys", () => {
    const layer = spectrumLayer();
    const next = writeStateAt(layer, 3, withThreshold(-30));
    expect(next.keys).toHaveLength(0);
    expect((next.state as SpectrumState).threshold).toBe(-30);
  });

  /** Auto-key: with keys present, an edit lands in one at the playhead. */
  it("creates a key at the playhead once the layer is animated", () => {
    const layer: SpectrumLayer = {
      ...spectrumLayer(8),
      keys: [keyAt(0, withThreshold(-80))],
    };
    const next = writeStateAt(layer, 5, withThreshold(-30));
    expect(next.keys).toHaveLength(2);
    expect(next.keys.find((k) => k.t === 5)).toBeDefined();
  });

  it("rewrites the key under the playhead rather than stacking a second on it", () => {
    const layer: SpectrumLayer = {
      ...spectrumLayer(8),
      keys: [keyAt(4, withThreshold(-80))],
    };
    // Inside KEY_SNAP_SECONDS of the existing key.
    const next = writeStateAt(layer, 4.02, withThreshold(-30));
    expect(next.keys).toHaveLength(1);
    expect((next.keys[0].state as SpectrumState).threshold).toBe(-30);
  });

  it("captures the tweened state, so adding a key mid-segment changes nothing", () => {
    const layer: SpectrumLayer = {
      ...spectrumLayer(8),
      keys: [keyAt(0, withThreshold(-80)), keyAt(4, withThreshold(-40))],
    };
    const before = (stateAt(layer, 2) as SpectrumState).threshold;
    const after = stateAt(addKeyAt(layer, 2), 2) as SpectrumState;
    expect(after.threshold).toBeCloseTo(before, 10);
  });
});
