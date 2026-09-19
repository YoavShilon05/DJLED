/**
 * The MIDI note axis, and how it borrows the frequency one.
 *
 * The engine stretches the configured note range across the whole 20 Hz–20 kHz
 * axis, so a note has a *position* on that axis rather than a pitch on it. This
 * is where that mapping lives, and it exists for one reason: every gizmo, tick
 * and hit test in the editor is written in Hz, and none of them should have to
 * learn about notes. A note is turned into the Hz that sits where it sits, and
 * the rest of the editor carries on unchanged.
 *
 * That also means the axis has to be *relabelled* in MIDI mode. Leaving "1k"
 * under a bar that is really D#5 would be worse than no label at all, so
 * [`noteTicks`] replaces the frequency scale with note names at the octaves.
 */

import { F_MAX, F_MIN, normToHz } from "./scales";

/** MIDI's note range, which is fixed by the protocol. */
export const MIN_NOTE = 0;
export const MAX_NOTE = 127;

/** A0 and C8 — the two ends of an 88-key piano, and the engine's defaults. */
export const DEFAULT_LOW_NOTE = 21;
export const DEFAULT_HIGH_NOTE = 108;

/** Matches the engine's `MIN_SPAN`: below an octave the strip stops reading as
 *  pitch at all. */
export const MIN_NOTE_SPAN = 12;

/** MIDI's sixteen channels, which the protocol fixes as firmly as its notes. */
export const CHANNEL_COUNT = 16;

/**
 * No channel owns this level — the engine's `NO_CHANNEL`.
 *
 * Channels are 0..15, so anything outside that says "nobody": every audio
 * level, and every MIDI point with nothing sounding on it. It matters that
 * those are the same answer, because a channel colour paints where a note is
 * and an unlit point has no note to be anyone's.
 */
export const NO_CHANNEL = 255;

/**
 * The engine's `default_channel_colors`, verbatim.
 *
 * Sixteen distinct hues at full opacity, so a MIDI layer reads by channel out
 * of the box and blending a channel back into the colour field is a slider away
 * on its square. Kept in step with the Rust by hand — it is a default rather
 * than a computation, and a drift shows up as a layer whose colours change when
 * the engine adopts the editor's config.
 */
export const DEFAULT_CHANNEL_COLORS = [
  "#ff0000ff",
  "#ff6000ff",
  "#ffbf00ff",
  "#dfff00ff",
  "#80ff00ff",
  "#20ff00ff",
  "#00ff40ff",
  "#00ff9fff",
  "#00ffffff",
  "#009fffff",
  "#0040ffff",
  "#2000ffff",
  "#8000ffff",
  "#df00ffff",
  "#ff00bfff",
  "#ff0060ff",
];

/**
 * The grid the editor lays the sixteen out on: four columns of four, numbered
 * down each column.
 *
 *     1  5   9  13
 *     2  6  10  14
 *     3  7  11  15
 *     4  8  12  16
 *
 * A DAW's channels are read as a list, so the columns are what stays stable as
 * the eye moves — 1..4 together is the first instrument, not the first row of a
 * table nobody is reading across.
 */
export const CHANNEL_GRID: number[] = [
  0, 4, 8, 12, 1, 5, 9, 13, 2, 6, 10, 14, 3, 7, 11, 15,
];

const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/** The range as the engine will actually use it: ordered, in range, and at
 *  least an octave wide. Mirrors `MidiConfig::range` so the editor draws what
 *  the strip is doing rather than what was typed. */
export function noteRange(low: number, high: number): [number, number] {
  const a = Math.round(Number.isFinite(low) ? low : DEFAULT_LOW_NOTE);
  const b = Math.round(Number.isFinite(high) ? high : DEFAULT_HIGH_NOTE);
  const lo = Math.min(Math.min(a, b), MAX_NOTE - MIN_NOTE_SPAN);
  const hi = Math.min(Math.max(Math.max(a, b), lo + MIN_NOTE_SPAN), MAX_NOTE);
  return [Math.max(MIN_NOTE, lo), hi];
}

/** Note name with octave, middle C (60) being C4. */
export function noteName(note: number): string {
  const n = Math.round(note);
  return `${NAMES[((n % 12) + 12) % 12]}${Math.floor(n / 12) - 1}`;
}

/** Concert pitch, for the tooltip that says what a note actually is. Not where
 *  it is drawn — see the module note. */
export function noteHz(note: number): number {
  return 440 * Math.pow(2, (note - 69) / 12);
}

/** 0..1 across the plot. The stretch: the low note sits at 0, the high at 1. */
export function noteToNorm(note: number, low: number, high: number): number {
  const [lo, hi] = noteRange(low, high);
  return (note - lo) / (hi - lo);
}

/**
 * The axis frequency a note is drawn at.
 *
 * This is the whole bridge: everything in the editor positions by Hz, so a note
 * becomes the Hz of its position and needs no special case anywhere else.
 */
export function noteToHz(note: number, low: number, high: number): number {
  return normToHz(noteToNorm(note, low, high));
}

export interface AxisTick {
  /** Position, as an axis frequency. */
  hz: number;
  label: string;
}

/**
 * Labelled ticks for a note axis: every C, thinned until the labels fit.
 *
 * Octaves rather than a fixed count, because they are what the eye is looking
 * for on a keyboard — C4 tells you where you are in a way that "note 60" does
 * not. A range of a few octaves labels every one; a full 88 keys labels every
 * other, which is as many as will fit without the names touching.
 */
export function noteTicks(low: number, high: number): AxisTick[] {
  const [lo, hi] = noteRange(low, high);
  const octaves: number[] = [];
  for (let note = Math.ceil(lo / 12) * 12; note <= hi; note += 12) octaves.push(note);

  const every = octaves.length > 8 ? 2 : 1;
  const ticks = octaves
    .filter((_, i) => i % every === 0)
    .map((note) => ({ hz: noteToHz(note, lo, hi), label: noteName(note) }));

  // The ends of the range are the two positions worth naming whatever else is
  // shown, and neither is likely to be a C.
  const ends: AxisTick[] = [];
  if (lo % 12 !== 0) ends.push({ hz: F_MIN, label: noteName(lo) });
  if (hi % 12 !== 0) ends.push({ hz: F_MAX, label: noteName(hi) });
  return [...ends, ...ticks];
}

/** Unlabelled subdivisions: one per note, or per C where that would be a blur. */
export function noteSubticks(low: number, high: number): number[] {
  const [lo, hi] = noteRange(low, high);
  const span = hi - lo;
  const step = span > 60 ? 12 : span > 30 ? 3 : 1;

  const out: number[] = [];
  for (let note = lo; note <= hi; note += step) out.push(noteToHz(note, lo, hi));
  return out;
}
