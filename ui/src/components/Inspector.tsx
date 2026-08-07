import {
  Checkbox,
  Divider,
  NumberInput,
  Paper,
  SegmentedControl,
  Select,
  Slider,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";

import { BLEND_HINT } from "../color/blend";
import { CURVE_OPTIONS, type CurveType } from "../config/curve";
import {
  BLEND_OPTIONS,
  type BlendMode,
  type GizmoFlags,
  type Layer,
  type SpectrumSource,
  type SpectrumState,
  type StaticState,
} from "../config/layers";
import { clamp } from "../config/scales";
import { Field } from "./Field";

interface Props {
  layer: Layer | null;
  /** The layer's state at the playhead — what an edit here writes back to. */
  state: SpectrumState | StaticState | null;
  isBottom: boolean;
  ledCount: number;
  gizmos: GizmoFlags;
  onLayer: (patch: Partial<Layer>) => void;
  onState: (state: SpectrumState | StaticState) => void;
  onGizmos: (gizmos: GizmoFlags) => void;
}

const GIZMO_ITEMS: Array<{ key: keyof GizmoFlags; label: string }> = [
  { key: "eq", label: "Parametric EQ" },
  { key: "thresholds", label: "Threshold handles" },
  { key: "colorKeyframes", label: "Colour keyframes" },
  { key: "ledKeyframes", label: "LED keyframes" },
  { key: "curve", label: "Intensity curve" },
];

/**
 * Everything about the focused layer that is not authored on the plot.
 *
 * Split deliberately along one line: how the layer *combines* (name, blend,
 * opacity, loop) sits at the top and never animates, and what the layer *is*
 * sits below it and does. That is the same split the data model makes, so a
 * control's position tells you whether the timeline will capture it.
 */
export function Inspector({
  layer,
  state,
  isBottom,
  ledCount,
  gizmos,
  onLayer,
  onState,
  onGizmos,
}: Props) {
  if (!layer || !state) {
    return (
      <Paper>
        <Text size="xs" c="dimmed">
          No layer selected.
        </Text>
      </Paper>
    );
  }

  return (
    <Paper>
      <Stack gap="md">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Layer
        </Text>

        <TextInput
          size="xs"
          label="Name"
          value={layer.name}
          onChange={(e) => onLayer({ name: e.currentTarget.value })}
        />

        <Field
          label="Blend mode"
          hint={
            isBottom
              ? "Nothing beneath this layer, so it lands on black whatever this says."
              : BLEND_HINT[layer.blendMode]
          }
        >
          <Select
            data={BLEND_OPTIONS}
            value={layer.blendMode}
            disabled={isBottom}
            allowDeselect={false}
            comboboxProps={{ withinPortal: true }}
            onChange={(v) => onLayer({ blendMode: (v as BlendMode) ?? layer.blendMode })}
          />
        </Field>

        <Field label="Opacity" value={`${Math.round(layer.opacity * 100)}%`}>
          <Slider
            min={0}
            max={1}
            step={0.01}
            value={layer.opacity}
            onChange={(opacity) => onLayer({ opacity })}
            label={(v) => `${Math.round(v * 100)}%`}
          />
        </Field>

        <Field
          label="Loop length"
          hint="This layer's own cycle, measured against the shared clock — so a 2 s layer stays in step with an 8 s one."
        >
          <NumberInput
            suffix=" s"
            min={0.1}
            max={600}
            step={0.5}
            decimalScale={2}
            value={layer.loopSeconds}
            onChange={(v) => onLayer({ loopSeconds: Math.max(0.1, Number(v) || 0.1) })}
          />
        </Field>

        <Divider />

        {layer.kind === "spectrum" ? (
          <SpectrumSection
            state={state as SpectrumState}
            gizmos={gizmos}
            onState={onState}
            onGizmos={onGizmos}
          />
        ) : (
          <StaticSection
            state={state as StaticState}
            ledCount={ledCount}
            onState={onState}
          />
        )}
      </Stack>
    </Paper>
  );
}

function SpectrumSection({
  state,
  gizmos,
  onState,
  onGizmos,
}: {
  state: SpectrumState;
  gizmos: GizmoFlags;
  onState: (state: SpectrumState) => void;
  onGizmos: (gizmos: GizmoFlags) => void;
}) {
  const selectedGizmos = GIZMO_ITEMS.filter((i) => gizmos[i.key]).map((i) => i.key);

  return (
    <>
      <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
        Spectrum
      </Text>

      <Field
        label="Source"
        hint={
          state.source === "midi"
            ? "Note number is the same log-frequency axis, relabelled — A440 is note 69. Colour keyframes and LED sectors carry across untouched. Needs engine support before it receives anything."
            : "The captured spectrum. Pick the device under Engine."
        }
      >
        <SegmentedControl
          fullWidth
          data={[
            { value: "audio", label: "Audio" },
            { value: "midi", label: "MIDI" },
          ]}
          value={state.source}
          onChange={(v) => onState({ ...state, source: v as SpectrumSource })}
        />
      </Field>

      <Field
        label="Blend radius"
        value={state.blendRadius.toFixed(2)}
        hint="Reach of each colour keyframe. Smaller is crisper, larger blurs neighbours together."
      >
        <Slider
          min={0.05}
          max={0.6}
          step={0.01}
          value={state.blendRadius}
          onChange={(blendRadius) => onState({ ...state, blendRadius })}
          label={(v) => v.toFixed(2)}
        />
      </Field>

      <Field
        label="Intensity curve"
        hint={
          state.curve.type === "bezier"
            ? "Drag the two handles in the curve box on the right of the plot."
            : "Fixed shape — switch to Bézier for handles."
        }
      >
        <Select
          data={CURVE_OPTIONS}
          value={state.curve.type}
          allowDeselect={false}
          comboboxProps={{ withinPortal: true }}
          onChange={(v) =>
            onState({ ...state, curve: { ...state.curve, type: (v as CurveType) ?? state.curve.type } })
          }
        />
      </Field>

      <Divider />

      <Stack gap="xs">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Gizmos
        </Text>
        <Text size="xs" c="dimmed">
          Visibility only — everything stays in the signal path.
        </Text>
        <Checkbox.Group
          value={selectedGizmos}
          onChange={(next) =>
            onGizmos(
              GIZMO_ITEMS.reduce(
                (acc, i) => ({ ...acc, [i.key]: next.includes(i.key) }),
                {} as GizmoFlags,
              ),
            )
          }
        >
          <Stack gap="xs">
            {GIZMO_ITEMS.map((i) => (
              <Checkbox key={i.key} value={i.key} label={i.label} />
            ))}
          </Stack>
        </Checkbox.Group>
      </Stack>
    </>
  );
}

function StaticSection({
  state,
  ledCount,
  onState,
}: {
  state: StaticState;
  ledCount: number;
  onState: (state: StaticState) => void;
}) {
  const last = Math.max(0, ledCount - 1);

  return (
    <>
      <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
        Static colour
      </Text>

      <Field
        label="Level"
        value={`${Math.round(state.level * 100)}%`}
        hint="This layer's own coverage. Under an additive layer it reads as how much of the wash survives between hits."
      >
        <Slider
          min={0}
          max={1}
          step={0.01}
          value={state.level}
          onChange={(level) => onState({ ...state, level })}
          label={(v) => `${Math.round(v * 100)}%`}
        />
      </Field>

      <Field
        label="LED span"
        hint="Which stretch of strip this layer paints. Outside it the layer is transparent, so whatever is beneath shows through unchanged."
      >
        <Stack gap="xs">
          <NumberInput
            label="From"
            min={0}
            max={last}
            value={state.from}
            onChange={(v) => onState({ ...state, from: clamp(Number(v) || 0, 0, last) })}
          />
          <NumberInput
            label="To"
            min={0}
            max={last}
            value={state.to}
            onChange={(v) => onState({ ...state, to: clamp(Number(v) || 0, 0, last) })}
          />
        </Stack>
      </Field>
    </>
  );
}
