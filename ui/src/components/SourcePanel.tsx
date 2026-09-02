import { Alert, Anchor, List, Paper, Select, Stack, Text } from "@mantine/core";

import type { EditorLayer } from "../config/editor";
import type { InputDevice, InputSource, LayerStatus, SourceKind } from "../engine";
import { Field } from "./Field";

/**
 * Keys for the entries that name no device.
 *
 * "Follow the system default" is a selection in its own right, not shorthand for
 * whichever endpoint it resolves to today — swapping headphones should not
 * strand it on the old ones. None collides with a device id, which is always
 * `host:something`.
 */
const DEFAULT_KEYS: Record<SourceKind, string> = {
  loopback: "default:loopback",
  input: "default:input",
  midi: "default:midi",
};

interface Props {
  /** The layer being edited. Each layer listens to its own device. */
  layer: EditorLayer;
  onChange: (layer: EditorLayer) => void;
  /** Everything selectable, as of the last scan. Empty until the engine has
   *  been heard from; the editor stays usable offline, but there is nothing to
   *  choose between. */
  devices: InputDevice[];
  /** What this layer's selection actually resolved to. Null while offline. */
  status: LayerStatus | null;
  connected: boolean;
  onRefresh: () => void;
}

/**
 * What one layer listens to.
 *
 * The selection is authored here and travels with the layer, so it survives a
 * reorder, a duplicate and a save. Whether it *opened* is the engine's answer,
 * and that is what the hint underneath reports — showing a selection that
 * failed would be a lie the size of a dark row.
 */
export function SourcePanel({ layer, onChange, devices, status, connected, onRefresh }: Props) {
  const selected = layer.source.id ?? DEFAULT_KEYS[layer.source.kind];
  const midi = layer.source.kind === "midi";
  const noMidiPorts = connected && !devices.some((d) => d.kind === "midi");
  const channels = status?.channels ?? 0;
  const outOfRange = status?.notesOutOfRange ?? 0;

  const change = (key: string | null) => {
    if (!key) return;
    // A channel index means nothing on a different device, so it resets rather
    // than carrying over as a stale number.
    const preset = (Object.entries(DEFAULT_KEYS) as Array<[SourceKind, string]>).find(
      ([, value]) => value === key,
    );
    const source: InputSource = preset
      ? { id: null, kind: preset[0], channel: null }
      : { id: key, kind: devices.find((d) => d.id === key)?.kind ?? "input", channel: null };
    onChange({ ...layer, source });
  };

  return (
    <Paper>
      <Stack gap="md">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Source · {layer.name}
        </Text>

        <Field
          label="Listening to"
          value={
            <Anchor
              component="button"
              type="button"
              size="xs"
              onClick={onRefresh}
              disabled={!connected}
            >
              rescan
            </Anchor>
          }
          hint={describe(layer, status, connected)}
        >
          <Select
            data={group(devices)}
            value={selected}
            onChange={change}
            placeholder="select a source"
            allowDeselect={false}
            comboboxProps={{ withinPortal: true }}
          />
        </Field>

        {/* Only worth showing where there is a choice to make: a mono
            microphone has nothing to pick between. */}
        {channels > 1 && (
          <Field
            label={midi ? "MIDI channel" : "Channel"}
            hint={
              midi
                ? "A DAW can send several parts down one cable. Pick one to light this layer from that part alone — another layer can take a different one, and they share the port."
                : "An instrument in input 1 only exists on one channel — mixing it with a silent neighbour costs 6 dB and adds that neighbour's noise."
            }
          >
            <Select
              data={[
                {
                  value: "mix",
                  label: midi ? `All ${channels} channels` : `Mix all ${channels}`,
                },
                ...Array.from({ length: channels }, (_, i) => ({
                  value: String(i),
                  label: `Channel ${i + 1}`,
                })),
              ]}
              value={layer.source.channel === null ? "mix" : String(layer.source.channel)}
              onChange={(value) =>
                onChange({
                  ...layer,
                  source: {
                    ...layer.source,
                    channel: value === null || value === "mix" ? null : Number(value),
                  },
                })
              }
              allowDeselect={false}
              comboboxProps={{ withinPortal: true }}
            />
          </Field>
        )}

        {/* The symptom this explains is "the strip is dark and nothing is
            wrong" — notes arriving in an octave the axis does not cover. */}
        {midi && outOfRange > 0 && (
          <Alert color="yellow" variant="light" title="Notes outside the range">
            <Text size="xs" lh={1.4}>
              {outOfRange} note{outOfRange === 1 ? "" : "s"} arrived outside this layer's note
              range and were dropped. Widen the range under MIDI, or transpose what is playing.
            </Text>
          </Alert>
        )}

        {noMidiPorts && <LoopMidiHint />}

        {status?.error && (
          <Alert color="red" variant="light" title="Source">
            <Text size="xs" lh={1.4}>
              {status.error}
            </Text>
            <Text size="xs" c="dimmed" lh={1.4} mt={4}>
              Only this layer is affected — the rest of the stack is still running.
            </Text>
          </Alert>
        )}
      </Stack>
    </Paper>
  );
}

/**
 * The one thing about MIDI on Windows that cannot be discovered by looking.
 *
 * FL's MIDI Out plugin sends to a MIDI *output* and this listens on a MIDI
 * *input*; Windows ships nothing that joins the two, and no amount of clicking
 * in either application will. Shown whenever there are no MIDI ports at all,
 * because that is exactly when someone is looking for the missing step.
 */
function LoopMidiHint() {
  return (
    <Alert color="gray" variant="light" title="No MIDI inputs">
      <Text size="xs" lh={1.5}>
        A keyboard appears here as soon as it is plugged in. FL Studio needs one setup step
        first — its MIDI Out plugin sends to a MIDI <i>output</i>, and this listens on a MIDI{" "}
        <i>input</i>, which Windows cannot join on its own:
      </Text>
      <List size="xs" type="ordered" mt={6} spacing={2}>
        <List.Item>
          Install{" "}
          <Anchor href="https://www.tobias-erichsen.de/software/loopmidi.html" target="_blank">
            loopMIDI
          </Anchor>{" "}
          and create a port in it
        </List.Item>
        <List.Item>
          In FL: Options → MIDI settings → Output, enable that port and note the port number it is
          given
        </List.Item>
        <List.Item>Set the MIDI Out plugin's Port to that number</List.Item>
      </List>
      <Text size="xs" lh={1.5} mt={6}>
        Then rescan — the loopMIDI port appears here like any other input.
      </Text>
    </Alert>
  );
}

/**
 * Grouped by kind, which is doing real work rather than tidying: an interface
 * presents its playback and capture halves under the *same name*, so the group
 * heading is the only thing telling the two entries apart.
 */
function group(devices: InputDevice[]) {
  const of = (kind: SourceKind) =>
    devices.filter((d) => d.kind === kind).map((d) => ({ value: d.id, label: label(d, devices) }));

  const loopback = of("loopback");
  const input = of("input");
  const midi = of("midi");

  return [
    {
      group: "System default",
      items: [
        { value: DEFAULT_KEYS.loopback, label: "PC audio — whatever is playing" },
        { value: DEFAULT_KEYS.input, label: "Default recording device" },
        // Offered even with no ports, so the reason there are none is one
        // click away rather than an absence to puzzle over.
        { value: DEFAULT_KEYS.midi, label: "First MIDI input" },
      ],
    },
    ...(loopback.length ? [{ group: "Playback, captured as loopback", items: loopback }] : []),
    ...(input.length ? [{ group: "Inputs", items: input }] : []),
    ...(midi.length ? [{ group: "MIDI", items: midi }] : []),
  ];
}

/**
 * The group heading carries the kind inside the open dropdown, and then
 * disappears with it — leaving two entries that read identically in the closed
 * control. So the name is qualified for exactly the devices where it is
 * ambiguous, which is any interface presenting both halves under one name.
 */
function label(device: InputDevice, all: InputDevice[]): string {
  const parts = [device.name];
  if (all.some((d) => d.name === device.name && d.kind !== device.kind)) parts.push(device.kind);
  // A missing rate means an endpoint that would not open. MIDI has no rate to
  // miss, so the same absence there means nothing at all.
  if (device.kind !== "midi" && device.sampleRate === null) parts.push("in use");
  return parts.join(" · ");
}

/**
 * What this layer is actually listening to. The selection alone does not say:
 * "the system default" names no device, and the rate comes from the endpoint
 * rather than from anything that was asked for.
 */
function describe(layer: EditorLayer, status: LayerStatus | null, connected: boolean): string {
  if (!connected) {
    return "Start the engine to see what this resolves to. The selection is saved with the layer either way.";
  }
  if (!status) return "Waiting for the engine to report on this layer.";

  if (status.kind === "midi") {
    const channel =
      layer.source.channel === null
        ? "all channels"
        : `channel ${layer.source.channel + 1} of ${status.channels}`;
    return `${status.deviceName} · midi · ${channel}. Notes land across the strip by pitch and velocity sets brightness; the range and glow are under MIDI. Other layers can share this port on other channels.`;
  }

  const parts = [
    status.deviceName,
    // Second, not buried at the end: which half of a device is being read is the
    // thing most likely to be wrong, and the least visible once chosen.
    status.kind,
    `${(status.sampleRate / 1000).toFixed(1)} kHz`,
    layer.source.channel === null
      ? `${status.channels} ch mixed`
      : `channel ${layer.source.channel + 1} of ${status.channels}`,
  ];

  // The failure this whole control exists for, described by its symptom rather
  // than its cause — "Spotify shows up, my DAW doesn't" is how it is met.
  const hint =
    status.kind === "loopback"
      ? " Loopback hears anything Windows mixes, but never a DAW on an ASIO driver — ASIO bypasses Windows entirely. Select this interface's input for that, or send it MIDI instead."
      : "";

  return `${parts.join(" · ")}.${hint}`;
}
