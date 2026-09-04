/**
 * End-to-end check of the preset store over the real wire.
 *
 * Connects to a running engine and exercises the path the dropdown takes:
 * edit, switch, edit, switch back, and confirm the first show came back. The
 * seam this crosses is the one no unit test can — a `ShowConfig` out of the
 * editor, into a slot on disk, and back out as the show the engine is running.
 *
 * The hotkeys are the half that cannot be checked from here: they are
 * registered with Windows, so pressing them is the test. What this does check
 * is that the switch they trigger behaves — they and the dropdown take the
 * same path through the engine.
 *
 *   cargo run --manifest-path engine/Cargo.toml --release
 *   node ui/scripts/preset-smoke.mjs
 */

import { WebSocket } from "ws";

const url = process.argv[2] ?? "ws://127.0.0.1:9001";
const socket = new WebSocket(url);

const layer = (color, threshold) => ({
  id: "a",
  name: "Smoke",
  enabled: true,
  opacity: 1,
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
  threshold,
  clamp: -99,
  curve: { type: "linear", p1: { x: 0.25, y: 0.1 }, p2: { x: 0.25, y: 1 } },
  decay: 0.82,
  sampleLength: 256,
  midi: { lowNote: 21, highNote: 108, spread: 1, sustain: true },
});

/** Resolve on the next `state`, so each step is read after it has landed. */
let pending = null;
socket.on("message", (data) => {
  const msg = JSON.parse(data.toString());
  if (msg.type === "state" && pending) {
    const resolve = pending;
    pending = null;
    resolve(msg);
  }
});

const nextState = () => new Promise((resolve) => (pending = resolve));
const send = (payload) => socket.send(JSON.stringify(payload));

const colorOf = (state) => state.config.layers[0].surface.keyframes[0].color;

let failures = 0;
function check(label, got, want) {
  const ok = got === want;
  if (!ok) failures += 1;
  console.log(`  ${ok ? "ok  " : "FAIL"}  ${label}: ${got}${ok ? "" : ` (expected ${want})`}`);
}

socket.on("open", async () => {
  const initial = await nextState();
  console.log(`presets: ${initial.presets.length} slots, live ${initial.activePreset + 1}`);
  console.log(initial.presets.map((p, i) => `  F${i + 1} ${p.name}${p.stored ? "" : " — empty"}`).join("\n"));

  console.log("\nauthoring red into slot 1");
  send({ type: "config", config: { layers: [layer("#ff2000", -62)] } });
  // The slot filling is announced; give it a moment to land either way.
  await new Promise((r) => setTimeout(r, 300));

  console.log("switching to slot 5");
  send({ type: "selectPreset", slot: 4 });
  let state = await nextState();
  check("live slot", state.activePreset, 4);
  check("empty slot opened a blank canvas", state.presets[4].stored, false);

  console.log("\nauthoring blue into slot 5, and naming it");
  send({ type: "config", config: { layers: [layer("#2040ff", -50)] } });
  send({ type: "renamePreset", slot: 4, name: "Cool" });
  state = await nextState();
  check("slot 5 name", state.presets[4].name, "Cool");
  check("slot 5 colour", colorOf(state), "#2040ff");

  console.log("\nswitching back to slot 1");
  send({ type: "selectPreset", slot: 0 });
  state = await nextState();
  check("live slot", state.activePreset, 0);
  check("slot 1 came back", colorOf(state), "#ff2000");
  check("slot 1 threshold came back", state.config.layers[0].threshold, -62);
  check("slot 1 is now stored", state.presets[0].stored, true);

  console.log("\nswitching to slot 5 again");
  send({ type: "selectPreset", slot: 4 });
  state = await nextState();
  check("slot 5 came back", colorOf(state), "#2040ff");
  check("slot 5 threshold came back", state.config.layers[0].threshold, -50);

  console.log("\nout of range is ignored, not obeyed");
  send({ type: "selectPreset", slot: 99 });
  send({ type: "renamePreset", slot: 0, name: "Warm" });
  state = await nextState();
  check("live slot unchanged", state.activePreset, 4);
  check("rename landed", state.presets[0].name, "Warm");

  console.log(failures === 0 ? "\nall checks passed" : `\n${failures} failed`);
  socket.close();
  process.exit(failures === 0 ? 0 : 1);
});

socket.on("error", (e) => {
  console.error(`could not reach ${url}: ${e.message}`);
  console.error("start the engine first: cargo run --manifest-path engine/Cargo.toml --release");
  process.exit(1);
});
