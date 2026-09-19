import { memo } from "react";
import { Divider, Select, Slider, Stack } from "@mantine/core";

import { CURVE_OPTIONS, type CurveType } from "../config/curve";
import { SAMPLE_LENGTHS, type EditorLayer } from "../config/editor";
import { Field } from "./Field";

interface Props {
  /** The layer being edited. Analysis is per layer: two layers on one device
   *  still get their own hop, EQ and ballistics. */
  layer: EditorLayer;
  onChange: (layer: EditorLayer) => void;
  /** The rate this layer's device is running at, for reading the hop in ms. */
  sampleRate: number;
}

/**
 * How this layer turns what it hears into where it lands.
 *
 * Two groups, and the split is the one that matters: everything above the
 * divider is about *time* — how fast a band rises and falls, how often it is
 * measured, how a level maps to brightness — and everything below is about
 * *space*, where the result is painted. They used to be one run of sliders with
 * master brightness at the bottom of it, which put the only control that
 * touches the whole wall inside the panel for one layer.
 *
 * Memoised: nothing in here is driven by a frame, and a frame arrives thirty
 * times a second. See the note on `frame` in `App.tsx`.
 */
export const ShapePanel = memo(function ShapePanel({ layer, onChange, sampleRate }: Props) {
  const rate = sampleRate || 48_000;

  return (
    <Stack gap="md">
      <Divider label="Response" labelPosition="left" />

      <Field
        label="Decay"
        value={layer.decay.toFixed(2)}
        hint="How much of the previous frame a band keeps."
        info="Higher falls away more slowly, so a kick leaves a tail instead of a flash. Zero snaps to whatever the last frame measured; one never falls at all."
      >
        <Slider
          min={0}
          max={0.99}
          step={0.01}
          value={layer.decay}
          onChange={(decay) => onChange({ ...layer, decay })}
          label={(v) => v.toFixed(2)}
        />
      </Field>

      <Field
        label="Frame hop"
        value={`${((layer.sampleLength / rate) * 1000).toFixed(1)} ms`}
        info="New samples between analysis frames. Shorter reacts faster and costs more CPU. This is not an FFT window length — the engine draws every band from the smallest transform that can still resolve it, five tiers from 8192 down to 128, so there is no single window to set."
      >
        <Select
          data={SAMPLE_LENGTHS.map((n) => ({ value: String(n), label: `${n} samples` }))}
          value={String(layer.sampleLength)}
          onChange={(value) =>
            onChange({ ...layer, sampleLength: Number(value) || layer.sampleLength })
          }
          allowDeselect={false}
          comboboxProps={{ withinPortal: true }}
        />
      </Field>

      <Field
        label="Intensity curve"
        hint={
          layer.curve.type === "bezier"
            ? "Drag the two handles in the curve box on the graph."
            : "Fixed shape — switch to Bézier for handles."
        }
        info="Maps a level between the threshold and the clamp onto brightness. The two rail handles beside the graph set that window; this sets what happens inside it."
      >
        <Select
          data={CURVE_OPTIONS}
          value={layer.curve.type}
          onChange={(value) =>
            onChange({
              ...layer,
              curve: { ...layer.curve, type: (value as CurveType) ?? layer.curve.type },
            })
          }
          allowDeselect={false}
          comboboxProps={{ withinPortal: true }}
        />
      </Field>

      <Divider label="On the strip" labelPosition="left" />

      <Field
        label="Direction"
        hint={LAYOUTS.find((l) => l.value === layoutOf(layer))?.hint}
        info="Mirror folds this layer's whole range into each half of the strip; reverse then flips what that lookup returns. Both are spatial — they move where colours land and change nothing about the analysis, which is why the graph does not show them."
      >
        <Select
          data={LAYOUTS.map(({ value, label }) => ({ value, label }))}
          value={layoutOf(layer)}
          onChange={(value) => {
            const next = LAYOUTS.find((l) => l.value === value);
            if (next) onChange({ ...layer, mirror: next.mirror, reverse: next.reverse });
          }}
          allowDeselect={false}
          comboboxProps={{ withinPortal: true }}
        />
      </Field>

      <Field
        label="Blend radius"
        value={layer.blend.toFixed(2)}
        hint="How sharply colour keyframes hand over to each other."
        info="Smaller is crisper, larger blurs neighbouring keyframes together. This is the colour surface's sigma — it is what makes two keyframes a gradient rather than two stripes. It is not a keyframe's area of effect: this decides how two keyframes that both reach a point share it, and it is one number for the whole layer, so narrowing it to confine one colour sharpens every other one too. Confining a single colour is on the keyframe itself, in the plot."
      >
        <Slider
          min={0.05}
          max={0.6}
          step={0.01}
          value={layer.blend}
          onChange={(blend) => onChange({ ...layer, blend })}
          label={(v) => v.toFixed(2)}
        />
      </Field>
    </Stack>
  );
});

/**
 * The four ways a layer can be laid along the strip, named by what they look
 * like.
 *
 * This was two checkboxes, "Reverse" and "Mirror", with a line underneath
 * explaining what each of the four combinations does. Nobody reaches for this
 * wanting to toggle a fold — they want bass in the middle — so the combination
 * is the control and the mechanism is in the info note.
 */
const LAYOUTS = [
  {
    value: "forward",
    label: "Bass → treble",
    hint: "Bass at the start of the strip, treble at the end.",
    mirror: false,
    reverse: false,
  },
  {
    value: "reverse",
    label: "Treble → bass",
    hint: "Treble at the start of the strip, bass at the end.",
    mirror: false,
    reverse: true,
  },
  {
    value: "mirror",
    label: "Bass at both ends",
    hint: "Bass at both ends, treble in the middle.",
    mirror: true,
    reverse: false,
  },
  {
    value: "mirror-reverse",
    label: "Bass in the middle",
    hint: "Bass in the middle, treble at both ends.",
    mirror: true,
    reverse: true,
  },
] as const;

function layoutOf(layer: EditorLayer): string {
  return (
    LAYOUTS.find((l) => l.mirror === layer.mirror && l.reverse === layer.reverse)?.value ??
    "forward"
  );
}
