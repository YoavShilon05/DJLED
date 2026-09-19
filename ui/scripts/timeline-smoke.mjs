/**
 * End-to-end check of a layer's timeline over the real wire.
 *
 * Sends a one-layer show whose colour field is a four second loop — red at the
 * start, blue halfway — and then watches the composited strip for longer than a
 * loop, bucketing what comes back by where in the loop it arrived.
 *
 * The seam this crosses is the one no unit test can: the engine's phase comes
 * from the wall clock rather than from anything in the message, so the only way
 * to know the two sides agree about *when* is to read the same clock here and
 * compare. If the phase were computed from an uptime, or the seek happened
 * after the render, this is the check that would notice.
 *
 * The layer listens to nothing on purpose. A still layer reports frames on its
 * own clock, so the result does not depend on anything playing — and it is also
 * the case that proves the engine keeps rendering an animated show with every
 * source idle.
 *
 *   cargo run --manifest-path engine/Cargo.toml --release
 *   node ui/scripts/timeline-smoke.mjs
 */

import { WebSocket } from "ws";

const url = process.argv[2] ?? "ws://127.0.0.1:9001";
const socket = new WebSocket(url);

const LENGTH = 4;
const LED = 75;
const BINS = 8;
/** A loop and a half, so every bin is filled whenever the run starts. */
const WATCH_MS = LENGTH * 1500;

const field = (color) => ({
  keyframes: [
    { x: 0, y: 0, color },
    { x: 1, y: 1, color },
  ],
  sigma: 0.5,
});

const show = {
  layers: [
    {
      id: "loop",
      name: "Loop",
      enabled: true,
      opacity: 1,
      // Nothing to listen to: the loop is the only thing moving.
      source: { id: null, kind: "none", channel: null },
      surface: field("#ff2000"),
      timeline: {
        enabled: true,
        length: LENGTH,
        keys: [
          { id: "start", at: 0, surface: field("#ff2000") },
          { id: "half", at: 0.5, surface: field("#2040ff") },
        ],
      },
      eq: [],
      ledKeyframes: [
        { led: 0, hz: 20 },
        { led: 149, hz: 20000 },
      ],
      reverse: false,
      mirror: false,
      threshold: -100,
      clamp: -99,
      curve: { type: "linear", p1: { x: 0.25, y: 0.1 }, p2: { x: 0.25, y: 1 } },
      decay: 0.82,
      sampleLength: 256,
      midi: { lowNote: 21, highNote: 108, spread: 1, sustain: true },
    },
  ],
};

/** Where the engine's loop is right now, from the same clock it reads. */
const phase = () => ((Date.now() / 1000) % LENGTH) / LENGTH;

const bins = Array.from({ length: BINS }, () => []);
let frames = 0;
let done = false;

socket.on("open", () => {
  socket.send(JSON.stringify({ type: "config", config: show }));
  setTimeout(finish, WATCH_MS);
});

socket.on("message", (data) => {
  const msg = JSON.parse(data.toString());
  if (msg.type === "error") {
    console.error("engine rejected a message:", msg.message);
    process.exitCode = 1;
    socket.close();
    return;
  }
  if (msg.type !== "frame" || done) return;
  frames += 1;

  const hex = msg.strip.slice(LED * 6, LED * 6 + 6);
  const rgb = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16));
  bins[Math.min(BINS - 1, Math.floor(phase() * BINS))].push(rgb);
});

socket.on("error", (e) => {
  console.error("could not reach the engine:", e.message);
  console.error("start it with: cargo run --manifest-path engine/Cargo.toml --release");
  process.exitCode = 1;
});

function mean(rows) {
  if (rows.length === 0) return null;
  return [0, 1, 2].map((c) => Math.round(rows.reduce((s, r) => s + r[c], 0) / rows.length));
}

function finish() {
  done = true;
  socket.close();

  console.log(`\n${frames} frames over ${WATCH_MS / 1000}s of a ${LENGTH}s loop, at LED ${LED}\n`);
  const averages = bins.map(mean);
  for (let i = 0; i < BINS; i++) {
    const at = (i / BINS).toFixed(3);
    const rgb = averages[i];
    console.log(`  phase ${at}  ${rgb ? rgb.map((v) => String(v).padStart(3)).join(" ") : "no frames"}`);
  }

  if (frames === 0) {
    fail("no frames arrived — is the engine running?");
    return;
  }
  if (averages.some((a) => a === null)) {
    fail("some of the loop was never sampled");
    return;
  }

  // The two authored keys, and the two things that could be wrong: the loop
  // not running at all, and the engine and this script disagreeing about which
  // instant of it is live.
  const start = averages[0];
  const half = averages[BINS / 2];
  if (start[0] <= start[2]) fail(`the start of the loop is not red: ${start}`);
  if (half[2] <= half[0]) fail(`halfway round is not blue: ${half}`);

  const spread = Math.max(...averages.map((a) => a[2])) - Math.min(...averages.map((a) => a[2]));
  if (spread < 32) fail(`the field barely moved across the loop: ${spread} of 255`);

  if (!process.exitCode) console.log("\nok — the wall is where the clock says it is");
}

function fail(reason) {
  console.error(`\nFAILED: ${reason}`);
  process.exitCode = 1;
}
