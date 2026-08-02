/**
 * A parametric EQ sitting between the analyser and the colour mapping.
 *
 * The engine works on band magnitudes, not samples, so the EQ is applied as a
 * gain curve in dB rather than by running audio through a filter. That makes
 * the *shape* of the curve the whole specification — and the shape people
 * expect from a parametric EQ is the RBJ cookbook biquad, so that is what is
 * evaluated here rather than an invented bell.
 *
 * Doing it properly is not much more code than faking it with a Gaussian, and
 * it means the engine can implement this with the same coefficients and get the
 * same answer instead of being reverse-engineered from a drawing.
 */

export type EqType = "peak" | "lowShelf" | "highShelf" | "lowPass" | "highPass";

export interface EqBand {
  id: string;
  type: EqType;
  hz: number;
  /** dB. Ignored by the pass filters, which cut rather than gain. */
  gain: number;
  q: number;
}

/** Vertical extent of the gain scale drawn over the plot. */
export const EQ_RANGE_DB = 24;

export const EQ_Q_MIN = 0.1;
export const EQ_Q_MAX = 12;

export const EQ_TYPE_OPTIONS: Array<{ value: EqType; label: string }> = [
  { value: "peak", label: "Bell" },
  { value: "lowShelf", label: "Low shelf" },
  { value: "highShelf", label: "High shelf" },
  { value: "lowPass", label: "Low pass" },
  { value: "highPass", label: "High pass" },
];

/** Pass filters have no gain, so their node is pinned to the zero line. */
export function hasGain(type: EqType): boolean {
  return type === "peak" || type === "lowShelf" || type === "highShelf";
}

export const DEFAULT_SAMPLE_RATE = 48_000;

interface Biquad {
  b0: number;
  b1: number;
  b2: number;
  a0: number;
  a1: number;
  a2: number;
}

/** RBJ Audio EQ Cookbook, verbatim. */
function coefficients(band: EqBand, fs: number): Biquad {
  const nyquist = fs / 2;
  const f0 = Math.min(Math.max(band.hz, 1), nyquist * 0.999);
  const q = Math.min(Math.max(band.q, EQ_Q_MIN), EQ_Q_MAX);
  const w0 = (2 * Math.PI * f0) / fs;
  const cos = Math.cos(w0);
  const sin = Math.sin(w0);
  const alpha = sin / (2 * q);
  const A = Math.pow(10, band.gain / 40);
  const sqrtA = Math.sqrt(A);

  switch (band.type) {
    case "peak":
      return {
        b0: 1 + alpha * A,
        b1: -2 * cos,
        b2: 1 - alpha * A,
        a0: 1 + alpha / A,
        a1: -2 * cos,
        a2: 1 - alpha / A,
      };
    case "lowShelf":
      return {
        b0: A * (A + 1 - (A - 1) * cos + 2 * sqrtA * alpha),
        b1: 2 * A * (A - 1 - (A + 1) * cos),
        b2: A * (A + 1 - (A - 1) * cos - 2 * sqrtA * alpha),
        a0: A + 1 + (A - 1) * cos + 2 * sqrtA * alpha,
        a1: -2 * (A - 1 + (A + 1) * cos),
        a2: A + 1 + (A - 1) * cos - 2 * sqrtA * alpha,
      };
    case "highShelf":
      return {
        b0: A * (A + 1 + (A - 1) * cos + 2 * sqrtA * alpha),
        b1: -2 * A * (A - 1 + (A + 1) * cos),
        b2: A * (A + 1 + (A - 1) * cos - 2 * sqrtA * alpha),
        a0: A + 1 - (A - 1) * cos + 2 * sqrtA * alpha,
        a1: 2 * (A - 1 - (A + 1) * cos),
        a2: A + 1 - (A - 1) * cos - 2 * sqrtA * alpha,
      };
    case "lowPass":
      return {
        b0: (1 - cos) / 2,
        b1: 1 - cos,
        b2: (1 - cos) / 2,
        a0: 1 + alpha,
        a1: -2 * cos,
        a2: 1 - alpha,
      };
    case "highPass":
      return {
        b0: (1 + cos) / 2,
        b1: -(1 + cos),
        b2: (1 + cos) / 2,
        a0: 1 + alpha,
        a1: -2 * cos,
        a2: 1 - alpha,
      };
  }
}

/** |H(e^jw)| in dB. The a0 term stays in the denominator, so no normalisation. */
function magnitudeDb(c: Biquad, hz: number, fs: number): number {
  const w = (2 * Math.PI * Math.min(hz, fs / 2)) / fs;
  const cos1 = Math.cos(w);
  const sin1 = Math.sin(w);
  const cos2 = Math.cos(2 * w);
  const sin2 = Math.sin(2 * w);

  const numRe = c.b0 + c.b1 * cos1 + c.b2 * cos2;
  const numIm = -(c.b1 * sin1 + c.b2 * sin2);
  const denRe = c.a0 + c.a1 * cos1 + c.a2 * cos2;
  const denIm = -(c.a1 * sin1 + c.a2 * sin2);

  const num = Math.hypot(numRe, numIm);
  const den = Math.hypot(denRe, denIm);
  if (den === 0) return 0;
  // Floored rather than allowed to reach -Infinity, which no plot can draw.
  return Math.max(-90, 20 * Math.log10(Math.max(num / den, 1e-6)));
}

/** One band's contribution, for drawing it on its own when selected. */
export function bandGainAt(band: EqBand, hz: number, fs = DEFAULT_SAMPLE_RATE): number {
  return magnitudeDb(coefficients(band, fs), hz, fs);
}

/**
 * The combined response. Built once per edit and then queried per band per
 * frame, which is why the coefficients are computed up front.
 */
export class EqCurve {
  private sections: Biquad[];

  constructor(
    bands: EqBand[],
    private fs: number = DEFAULT_SAMPLE_RATE,
  ) {
    this.sections = bands.map((b) => coefficients(b, fs));
  }

  get isFlat(): boolean {
    return this.sections.length === 0;
  }

  /** Total gain in dB at a frequency — cascaded filters add in dB. */
  gainAt(hz: number): number {
    let total = 0;
    for (const section of this.sections) total += magnitudeDb(section, hz, this.fs);
    return total;
  }
}
