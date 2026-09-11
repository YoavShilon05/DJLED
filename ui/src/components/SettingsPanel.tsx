import { memo } from "react";
import { Checkbox, Divider, Paper, Select, Slider, Stack, Text } from "@mantine/core";

import { CURVE_OPTIONS, type CurveType } from "../config/curve";
import { SAMPLE_LENGTHS, type EditorLayer } from "../config/editor";
import { Field } from "./Field";

interface Props {
  /** The layer being edited. Analysis is per layer: two layers on one device
   *  still get their own hop, EQ and ballistics. */
  layer: EditorLayer;
  onChange: (layer: EditorLayer) => void;
  brightness: number;
  onBrightness: (value: number) => void;
  /** The rate this layer's device is running at, for reading the hop in ms. */
  sampleRate: number;
}

/**
 * Memoised: nothing in here is driven by a frame, and a frame arrives thirty
 * times a second. See the note on `frame` in `App.tsx`.
 */
export const SettingsPanel = memo(function SettingsPanel({
  layer,
  onChange,
  brightness,
  onBrightness,
  sampleRate,
}: Props) {
  const rate = sampleRate || 48_000;

  return (
    <Paper>
      <Stack gap="md">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Analysis · {layer.name}
        </Text>

        <Field
          label="Decay"
          value={layer.decay.toFixed(2)}
          hint="How much of the previous frame a band keeps. Higher falls away more slowly."
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
          hint="New samples between analysis frames. Shorter reacts faster and costs more CPU. The FFT window is picked per band, so there is no single length to set."
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
              ? "Drag the two handles in the curve box on the right."
              : "Fixed shape — switch to Bézier for handles."
          }
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

        <Divider />

        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Output
        </Text>

        <Field
          label="Blend radius"
          value={layer.blend.toFixed(2)}
          hint="Reach of each colour keyframe. Smaller is crisper, larger blurs neighbours together."
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

        <Stack gap="xs">
          <Checkbox
            label="Reverse"
            checked={layer.reverse}
            onChange={(e) => onChange({ ...layer, reverse: e.currentTarget.checked })}
          />
          <Checkbox
            label="Mirror"
            checked={layer.mirror}
            onChange={(e) => onChange({ ...layer, mirror: e.currentTarget.checked })}
          />
          <Text size="xs" c="dimmed" lh={1.35}>
            {describeLayout(layer.mirror, layer.reverse)}
          </Text>
        </Stack>

        <Divider />

        {/*
          Below the divider because it is the only control on this panel that is
          not per layer. Brightness is the power budget for the whole strip —
          a layer that dimmed the ones under it would be a blend mode, not a
          brightness — so it is deliberately grouped apart from everything above.
        */}
        <Field
          label="Master brightness"
          value={`${Math.round(brightness * 100)}%`}
          hint="The whole strip, every layer. Also the software half of the power budget."
        >
          <Slider
            min={0}
            max={1}
            step={0.01}
            value={brightness}
            onChange={onBrightness}
            label={(v) => `${Math.round(v * 100)}%`}
          />
        </Field>
      </Stack>
    </Paper>
  );
});

/**
 * Spelled out rather than left to the checkbox labels: the combination of the
 * two is not something anyone should have to work out from first principles.
 */
function describeLayout(mirror: boolean, reverse: boolean): string {
  if (mirror && reverse) return "Bass in the middle, treble at both ends.";
  if (mirror) return "Bass at both ends, treble in the middle.";
  if (reverse) return "Treble at the start of the strip, bass at the end.";
  return "Bass at the start of the strip, treble at the end.";
}
