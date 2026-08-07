import { Checkbox, Divider, Paper, Select, Slider, Stack, Text } from "@mantine/core";

import { SAMPLE_LENGTHS, type MasterConfig } from "../config/show";
import { Field } from "./Field";

interface Props {
  master: MasterConfig;
  onChange: (master: MasterConfig) => void;
  brightness: number;
  onBrightness: (value: number) => void;
  sampleRate: number;
}

/**
 * The wall, and the one analyser feeding it.
 *
 * Everything here is deliberately *not* per-layer. Reverse and mirror describe
 * how the strip is physically mounted — two layers disagreeing about which end
 * is which is not an effect anyone wants — and there is one analyser, so decay
 * and frame hop would need one per layer to be per-layer. Master brightness is
 * a dimmer on the finished composite.
 */
export function MasterPanel({ master, onChange, brightness, onBrightness, sampleRate }: Props) {
  const rate = sampleRate || 48_000;

  return (
    <Paper>
      <Stack gap="md">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Analysis
        </Text>

        <Field
          label="Decay"
          value={master.decay.toFixed(2)}
          hint="How much of the previous frame a band keeps. Higher falls away more slowly."
        >
          <Slider
            min={0}
            max={0.99}
            step={0.01}
            value={master.decay}
            onChange={(decay) => onChange({ ...master, decay })}
            label={(v) => v.toFixed(2)}
          />
        </Field>

        <Field
          label="Frame hop"
          value={`${((master.sampleLength / rate) * 1000).toFixed(1)} ms`}
          hint="New samples between analysis frames. Shorter reacts faster and costs more CPU. The FFT window is picked per band, so there is no single length to set."
        >
          <Select
            data={SAMPLE_LENGTHS.map((n) => ({ value: String(n), label: `${n} samples` }))}
            value={String(master.sampleLength)}
            onChange={(value) =>
              onChange({ ...master, sampleLength: Number(value) || master.sampleLength })
            }
            allowDeselect={false}
            comboboxProps={{ withinPortal: true }}
          />
        </Field>

        <Divider />

        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Wall
        </Text>

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
            checked={master.reverse}
            onChange={(e) => onChange({ ...master, reverse: e.currentTarget.checked })}
          />
          <Checkbox
            label="Mirror"
            checked={master.mirror}
            onChange={(e) => onChange({ ...master, mirror: e.currentTarget.checked })}
          />
          <Text size="xs" c="dimmed" lh={1.35}>
            {describeLayout(master.mirror, master.reverse)} Applied to the finished
            composite, so no layer can see it.
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
