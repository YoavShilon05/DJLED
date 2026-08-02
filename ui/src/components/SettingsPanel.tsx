import { Checkbox, Divider, Paper, Select, Slider, Stack, Text } from "@mantine/core";

import { CURVE_OPTIONS, type CurveType } from "../config/curve";
import { SAMPLE_LENGTHS, type EditorConfig } from "../config/editor";
import { Field } from "./Field";

interface Props {
  config: EditorConfig;
  onChange: (config: EditorConfig) => void;
  brightness: number;
  onBrightness: (value: number) => void;
  sampleRate: number;
}

export function SettingsPanel({ config, onChange, brightness, onBrightness, sampleRate }: Props) {
  const rate = sampleRate || 48_000;

  return (
    <Paper>
      <Stack gap="md">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Analysis
        </Text>

        <Field
          label="Decay"
          value={config.decay.toFixed(2)}
          hint="How much of the previous frame a band keeps. Higher falls away more slowly."
        >
          <Slider
            min={0}
            max={0.99}
            step={0.01}
            value={config.decay}
            onChange={(decay) => onChange({ ...config, decay })}
            label={(v) => v.toFixed(2)}
          />
        </Field>

        <Field
          label="Sample length"
          value={`${((config.sampleLength / rate) * 1000).toFixed(0)} ms`}
          hint="Samples captured before each transform. Longer resolves bass; shorter reacts faster."
        >
          <Select
            data={SAMPLE_LENGTHS.map((n) => ({ value: String(n), label: `${n} samples` }))}
            value={String(config.sampleLength)}
            onChange={(value) =>
              onChange({ ...config, sampleLength: Number(value) || config.sampleLength })
            }
            allowDeselect={false}
            comboboxProps={{ withinPortal: true }}
          />
        </Field>

        <Field
          label="Intensity curve"
          hint={
            config.curve.type === "bezier"
              ? "Drag the two handles in the curve box on the right."
              : "Fixed shape — switch to Bézier for handles."
          }
        >
          <Select
            data={CURVE_OPTIONS}
            value={config.curve.type}
            onChange={(value) =>
              onChange({
                ...config,
                curve: { ...config.curve, type: (value as CurveType) ?? config.curve.type },
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
          value={config.blend.toFixed(2)}
          hint="Reach of each colour keyframe. Smaller is crisper, larger blurs neighbours together."
        >
          <Slider
            min={0.05}
            max={0.6}
            step={0.01}
            value={config.blend}
            onChange={(blend) => onChange({ ...config, blend })}
            label={(v) => v.toFixed(2)}
          />
        </Field>

        <Field label="Master brightness" value={`${Math.round(brightness * 100)}%`}>
          <Slider
            min={0}
            max={1}
            step={0.01}
            value={brightness}
            onChange={onBrightness}
            label={(v) => `${Math.round(v * 100)}%`}
          />
        </Field>

        <Stack gap="xs">
          <Checkbox
            label="Reverse"
            checked={config.reverse}
            onChange={(e) => onChange({ ...config, reverse: e.currentTarget.checked })}
          />
          <Checkbox
            label="Mirror"
            checked={config.mirror}
            onChange={(e) => onChange({ ...config, mirror: e.currentTarget.checked })}
          />
          <Text size="xs" c="dimmed" lh={1.35}>
            {describeLayout(config.mirror, config.reverse)}
          </Text>
        </Stack>
      </Stack>
    </Paper>
  );
}

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
