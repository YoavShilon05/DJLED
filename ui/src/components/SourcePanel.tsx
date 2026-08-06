import { Alert, Anchor, Paper, Select, Stack, Text } from "@mantine/core";

import type { AudioDevice, AudioSource, AudioState, SourceKind } from "../engine";
import { Field } from "./Field";

/**
 * Keys for the two entries that name no device.
 *
 * "Follow the system default" is a selection in its own right, not shorthand for
 * whichever endpoint it resolves to today — swapping headphones should not
 * strand it on the old ones. Neither collides with a device id, which is always
 * `host:something`.
 */
const DEFAULT_KEYS: Record<SourceKind, string> = {
  loopback: "default:loopback",
  input: "default:input",
};

interface Props {
  /** Null until the engine has been heard from; the editor stays usable
   *  offline, but there is nothing to choose between. */
  audio: AudioState | null;
  onSource: (source: AudioSource) => void;
  onRefresh: () => void;
}

export function SourcePanel({ audio, onSource, onRefresh }: Props) {
  const devices = audio?.devices ?? [];
  const selected = audio ? (audio.source.id ?? DEFAULT_KEYS[audio.source.kind]) : null;

  const change = (key: string | null) => {
    if (!key) return;
    // A channel index means nothing on a different device, so it resets rather
    // than carrying over as a stale number.
    if (key === DEFAULT_KEYS.loopback) onSource({ id: null, kind: "loopback", channel: null });
    else if (key === DEFAULT_KEYS.input) onSource({ id: null, kind: "input", channel: null });
    else {
      const kind = devices.find((d) => d.id === key)?.kind ?? "input";
      onSource({ id: key, kind, channel: null });
    }
  };

  return (
    <Paper>
      <Stack gap="md">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Source
        </Text>

        <Field
          label="Listening to"
          value={
            <Anchor component="button" type="button" size="xs" onClick={onRefresh} disabled={!audio}>
              rescan
            </Anchor>
          }
          hint={describe(audio)}
        >
          <Select
            data={group(devices)}
            value={selected}
            onChange={change}
            disabled={!audio}
            placeholder={audio ? "select a source" : "engine offline"}
            allowDeselect={false}
            comboboxProps={{ withinPortal: true }}
          />
        </Field>

        {/* Only worth showing where there is a choice to make: a mono
            microphone has nothing to pick between. */}
        {audio && audio.channels > 1 && (
          <Field
            label="Channel"
            hint="An instrument in input 1 only exists on one channel — mixing it with a silent neighbour costs 6 dB and adds that neighbour's noise."
          >
            <Select
              data={[
                { value: "mix", label: `Mix all ${audio.channels}` },
                ...Array.from({ length: audio.channels }, (_, i) => ({
                  value: String(i),
                  label: `Channel ${i + 1}`,
                })),
              ]}
              value={audio.source.channel === null ? "mix" : String(audio.source.channel)}
              onChange={(value) =>
                onSource({
                  ...audio.source,
                  channel: value === null || value === "mix" ? null : Number(value),
                })
              }
              allowDeselect={false}
              comboboxProps={{ withinPortal: true }}
            />
          </Field>
        )}

        {audio?.error && (
          <Alert color="red" variant="light" title="Audio source">
            <Text size="xs" lh={1.4}>
              {audio.error}
            </Text>
          </Alert>
        )}
      </Stack>
    </Paper>
  );
}

/**
 * Grouped by direction, which is doing real work rather than tidying: an
 * interface presents its playback and capture halves under the *same name*, so
 * the group heading is the only thing telling the two entries apart.
 */
function group(devices: AudioDevice[]) {
  const of = (kind: SourceKind) =>
    devices.filter((d) => d.kind === kind).map((d) => ({ value: d.id, label: label(d, devices) }));

  const loopback = of("loopback");
  const input = of("input");

  return [
    {
      group: "System default",
      items: [
        { value: DEFAULT_KEYS.loopback, label: "PC audio — whatever is playing" },
        { value: DEFAULT_KEYS.input, label: "Default recording device" },
      ],
    },
    ...(loopback.length ? [{ group: "Playback, captured as loopback", items: loopback }] : []),
    ...(input.length ? [{ group: "Inputs", items: input }] : []),
  ];
}

/**
 * The group heading carries the direction inside the open dropdown, and then
 * disappears with it — leaving two entries that read identically in the closed
 * control. So the name is qualified for exactly the devices where it is
 * ambiguous, which is any interface presenting both halves under one name.
 */
function label(device: AudioDevice, all: AudioDevice[]): string {
  const parts = [device.name];
  if (all.some((d) => d.name === device.name && d.kind !== device.kind)) parts.push(device.kind);
  if (device.sampleRate === null) parts.push("in use");
  return parts.join(" · ");
}

/**
 * What is actually being captured. The selection alone does not say: "the system
 * default" names no device, and the rate comes from the endpoint rather than
 * from anything that was asked for.
 */
function describe(audio: AudioState | null): string {
  if (!audio) return "Start the engine to choose what to listen to.";

  const parts = [
    audio.deviceName,
    // Second, not buried at the end: which half of a device is being read is the
    // thing most likely to be wrong, and the least visible once chosen.
    audio.kind,
    `${(audio.sampleRate / 1000).toFixed(1)} kHz`,
    audio.source.channel === null
      ? `${audio.channels} ch mixed`
      : `channel ${audio.source.channel + 1} of ${audio.channels}`,
  ];

  // The failure this whole control exists for, described by its symptom rather
  // than its cause — "Spotify shows up, my DAW doesn't" is how it is met.
  const hint =
    audio.kind === "loopback"
      ? " Loopback hears anything Windows mixes, but never a DAW on an ASIO driver — ASIO bypasses Windows entirely. Select this interface's input for that."
      : "";

  return `${parts.join(" · ")}.${hint}`;
}
