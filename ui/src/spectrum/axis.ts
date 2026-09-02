/**
 * What the horizontal axis is labelled with.
 *
 * The axis itself never changes — it is always the log frequency scale the
 * colour surface, the LED sectors and every gizmo are written against. What
 * changes is what the marks along it *mean*. Driving the strip from MIDI
 * stretches a note range across that same axis, so "1k" under a bar that is
 * really D#5 would be a label actively lying about the thing beneath it.
 *
 * So the scale is a small object rather than a module constant: same
 * positions, different names. Everything that draws or reads the axis takes one
 * of these and needs to know nothing else.
 */

import { noteName, noteRange, noteSubticks, noteTicks, type AxisTick } from "../config/notes";
import { FREQ_SUBTICKS, FREQ_TICKS, formatHz, hzToNorm } from "../config/scales";

export type { AxisTick };

export interface AxisScale {
  /** Labelled marks, positioned as axis frequencies. */
  ticks: AxisTick[];
  /** Unlabelled subdivisions, as axis frequencies. */
  subticks: number[];
  /** Unit caption, shown once in the gutter. */
  unit: string;
  /** A position on the axis, as the user would say it. */
  format: (hz: number) => string;
}

export const FREQUENCY_AXIS: AxisScale = {
  ticks: FREQ_TICKS.map((hz) => ({ hz, label: formatHz(hz) })),
  subticks: FREQ_SUBTICKS,
  unit: "Hz",
  format: (hz) => `${formatHz(hz)} Hz`,
};

/**
 * The note axis for a range, stretched end to end.
 *
 * `format` names the nearest note rather than interpolating, because a position
 * between two semitones is not a thing anyone means — a keyframe dropped
 * between C4 and C#4 is being placed at one of them.
 */
export function noteAxis(low: number, high: number): AxisScale {
  const [lo, hi] = noteRange(low, high);
  return {
    ticks: noteTicks(lo, hi),
    subticks: noteSubticks(lo, hi),
    unit: "note",
    format: (hz) => noteName(Math.round(lo + hzToNorm(hz) * (hi - lo))),
  };
}
