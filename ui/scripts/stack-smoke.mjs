/**
 * End-to-end check of the layer stack over the real wire.
 *
 * Connects to a running engine, sends a two-layer show, and reports what comes
 * back — the state echo, the per-layer frames, and the composited strip. The
 * point is the seam no unit test crosses: JSON out of the editor, through the
 * engine, and back as bytes for the wall.
 *
 *   cargo run --manifest-path engine/Cargo.toml --release
 *   node ui/scripts/stack-smoke.mjs
 */

import { WebSocket } from "ws";

const url = process.argv[2] ?? "ws://127.0.0.1:9001";
const socket = new WebSocket(url);

const flat = (id, name, color, opacity, extra = {}) => ({
  id,
  name,
  enabled: true,
  opacity,
  source: { id: null, kind: "loopback", channel: null },
  surface: {
    keyframes: [
      { x: 0, y: 0, color },
      { x: 1, y: 1, color },
    ],
    sigma: 0.5,
  },
  eq: [],
  ledKeyframes: [
    { led: 0, hz: 20 },
    { led: 149, hz: 20000 },
  ],
  reverse: false,
  mirror: false,
  // Wide open, so the strip is lit whatever is or is not playing.
  threshold: -100,
  clamp: -99,
  curve: { type: "linear", p1: { x: 0.25, y: 0.1 }, p2: { x: 0.25, y: 1 } },
  decay: 0.82,
  sampleLength: 256,
  midi: { lowNote: 21, highNote: 108, spread: 1, sustain: true },
  ...extra,
});

const cases = [
  { label: "one layer, red", layers: [flat("a", "Red", "#ff2000", 1)] },
  {
    label: "green over red, opaque — expect green",
    layers: [flat("a", "Red", "#ff2000", 1), flat("b", "Green", "#40ff60", 1)],
  },
  {
    label: "green over red, 50% — expect a blend",
    layers: [flat("a", "Red", "#ff2000", 1), flat("b", "Green", "#40ff60", 0.5)],
  },
  {
    label: "opaque black over red — expect dark",
    layers: [flat("a", "Red", "#ff2000", 1), flat("b", "Black", "#000000", 1)],
  },
  {
    label: "transparent black over red — expect red",
    layers: [flat("a", "Red", "#ff2000", 1), flat("b", "Clear", "#00000000", 1)],
  },
  {
    label: "green confined to the top half — expect red then green",
    layers: [
      flat("a", "Red", "#ff2000", 1),
      flat("b", "Green", "#40ff60", 1, {
        ledKeyframes: [
          { led: 75, hz: 20 },
          { led: 149, hz: 20000 },
        ],
      }),
    ],
  },
];

let index = -1;
let awaiting = 0;
const results = [];

const sample = (strip, led) => strip.slice(led * 6, led * 6 + 6);

function next() {
  index += 1;
  if (index >= cases.length) {
    report();
    socket.close();
    return;
  }
  socket.send(JSON.stringify({ type: "config", config: { layers: cases[index].layers } }));
  // A few frames of slack: the config lands on the next pass of the run loop,
  // and the snapshot is republished at 30 fps.
  awaiting = 6;
}

socket.on("open", () => next());

socket.on("message", (data) => {
  const msg = JSON.parse(data.toString());
  if (msg.type === "error") {
    console.error("engine rejected a message:", msg.message);
    process.exitCode = 1;
    socket.close();
    return;
  }
  if (msg.type === "state") {
    console.log(
      `state: ${msg.layers.length} layer(s), ${msg.pointCount} points, ${msg.ledCount} LEDs, ` +
        `${msg.devices.length} devices`,
    );
    for (const l of msg.layers) {
      console.log(
        `  ${l.id.padEnd(6)} ${l.kind.padEnd(9)} ${l.points} pts  ${l.deviceName}` +
          (l.error ? `  ERROR ${l.error}` : ""),
      );
    }
    return;
  }
  if (msg.type !== "frame" || index < 0 || index >= cases.length) return;
  if (awaiting-- > 0) return;

  results.push({
    label: cases[index].label,
    layers: msg.layers.length,
    first: sample(msg.strip, 0),
    mid: sample(msg.strip, 100),
    last: sample(msg.strip, 149),
  });
  next();
});

socket.on("error", (e) => {
  console.error("could not reach the engine:", e.message);
  console.error("start it with: cargo run --manifest-path engine/Cargo.toml --release");
  process.exitCode = 1;
});

function report() {
  console.log("\nstrip, sampled at LED 0 / 100 / 149\n");
  for (const r of results) {
    console.log(`  ${r.label}`);
    console.log(`    layers=${r.layers}  ${r.first}  ${r.mid}  ${r.last}`);
  }
}
