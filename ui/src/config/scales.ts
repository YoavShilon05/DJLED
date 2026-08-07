/**
 * The two axes, and nothing else.
 *
 * Every gizmo, tick and hit test goes through here, so a keyframe authored at
 * 250 Hz lands on the same pixel as the 250 Hz grid line by construction rather
 * than by two call sites agreeing.
 *
 * Normalised space is `0..1`, origin bottom-left — the same convention the
 * colour surface uses, which is why a keyframe can be handed to the engine
 * without a second mapping.
 */

export const F_MIN = 20;
export const F_MAX = 20_000;

export const DB_MIN = -80;
export const DB_MAX = 0;

/** Labelled frequency pivots. Everything else is an unlabelled subdivision. */
export const FREQ_TICKS = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10_000, 20_000];

/** 1-2-5 decade subdivisions, minus the pivots that are already labelled. */
export const FREQ_SUBTICKS = [
  30, 40, 60, 70, 80, 90, 150, 300, 400, 600, 700, 800, 900, 1500, 3000, 4000, 6000, 7000, 8000,
  9000, 15_000,
];

export const DB_TICKS = [0, -10, -20, -30, -40, -50, -60, -70, -80];

const LOG_SPAN = Math.log(F_MAX / F_MIN);

export function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

/** Hz → 0..1 across the log axis. */
export function hzToNorm(hz: number): number {
  return clamp(Math.log(hz / F_MIN) / LOG_SPAN, 0, 1);
}

/** 0..1 → Hz. */
export function normToHz(n: number): number {
  return F_MIN * Math.exp(clamp(n, 0, 1) * LOG_SPAN);
}

/** dB → 0..1, 0 at the floor. */
export function dbToNorm(db: number): number {
  return clamp((db - DB_MIN) / (DB_MAX - DB_MIN), 0, 1);
}

/** 0..1 → dB. */
export function normToDb(n: number): number {
  return DB_MIN + clamp(n, 0, 1) * (DB_MAX - DB_MIN);
}

/* ------------------------------------------------------------- note numbers */

/**
 * The MIDI ruler, over the same axis.
 *
 * Not a second scale — semitones *are* logarithmic in frequency, so a note
 * number is the existing log-Hz axis relabelled. A440 is note 69 by definition,
 * and everything below follows from twelve semitones to an octave. Note 21 (A0)
 * lands at 27.5 Hz and note 127 at 12.5 kHz, both comfortably inside the plot's
 * 20 Hz–20 kHz range, which is why a spectrum layer can switch source without
 * moving a single colour keyframe.
 */
export function noteToHz(note: number): number {
  return 440 * Math.pow(2, (note - 69) / 12);
}

export function hzToNote(hz: number): number {
  return 69 + 12 * Math.log2(Math.max(hz, 1e-6) / 440);
}

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/** "C4", "A#2" — scientific pitch notation, where middle C is C4. */
export function formatNote(note: number): string {
  const n = Math.round(note);
  return `${NOTE_NAMES[((n % 12) + 12) % 12]}${Math.floor(n / 12) - 1}`;
}

/** Labelled octaves. Every C that falls inside the plot's frequency range. */
export const NOTE_TICKS = [24, 36, 48, 60, 72, 84, 96, 108, 120].filter((n) => {
  const hz = noteToHz(n);
  return hz >= F_MIN && hz <= F_MAX;
});

/** Unlabelled subdivisions: the F a fifth up from each labelled C. */
export const NOTE_SUBTICKS = NOTE_TICKS.map((n) => n + 5).filter((n) => noteToHz(n) <= F_MAX);

/** "250 Hz", "1.5k", "20k" — short enough to sit under a tick. */
export function formatHz(hz: number): string {
  if (hz >= 1000) {
    const k = hz / 1000;
    return `${k >= 10 || Number.isInteger(k) ? k.toFixed(0) : k.toFixed(1)}k`;
  }
  return `${Math.round(hz)}`;
}

/** Round to something a user would have typed, so dragging reads cleanly. */
export function snapHz(hz: number): number {
  if (hz >= 10_000) return Math.round(hz / 500) * 500;
  if (hz >= 1000) return Math.round(hz / 100) * 100;
  if (hz >= 100) return Math.round(hz / 10) * 10;
  return Math.round(hz);
}
