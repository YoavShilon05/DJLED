import { Alert, Anchor, List, Paper, Select, Stack, Text } from "@mantine/core";

import type { InputDevice, InputSource, InputState, SourceKind } from "../engine";
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
  /** Null until the engine has been heard from; the editor stays usable
   *  offline, but there is nothing to choose between. */
  input: InputState | null;
  /** Notes dropped for falling outside the note range, from the live frame. */
  outOfRange: number;
  onSource: (source: InputSource) => void;
  onRefresh: () => void;
}

export function SourcePanel({ input, outOfRange, onSource, onRefresh }: Props) {
  const devices = input?.devices ?? [];
  const selected = input ? (input.source.id ?? DEFAULT_KEYS[input.source.kind]) : null;
  const midi = input?.kind === "midi";
  const noMidiPorts = input !== null && !devices.some((d) => d.kind === "midi");

  const change = (key: string | null) => {
    if (!key) return;
    // A channel index means nothing on a different device, so it resets rather
    // than carrying over as a stale number.
    const preset = (Object.entries(DEFAULT_KEYS) as Array<[SourceKind, string]>).find(
      ([, value]) => value === key,
    );
    if (preset) onSource({ id: null, kind: preset[0], channel: null });
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
            <Anchor component="button" type="button" size="xs" onClick={onRefresh} disabled={!input}>
              rescan
            </Anchor>
          }
          hint={describe(input)}
        >
          <Select
            data={group(devices)}
            value={selected}
            onChange={change}
            disabled={!input}
            placeholder={input ? "select a source" : "engine offline"}
            allowDeselect={false}
            comboboxProps={{ withinPortal: true }}
          />
        </Field>

        {/* Only worth showing where there is a choice to make: a mono
            microphone has nothing to pick between. */}
        {input && input.channels > 1 && (
          <Field
            label={midi ? "MIDI channel" : "Channel"}
            hint={
              midi
                ? "A DAW can send several parts down one cable. Pick one to light the strip from that part alone."
                : "An instrument in input 1 only exists on one channel — mixing it with a silent neighbour costs 6 dB and adds that neighbour's noise."
            }
          >
            <Select
              data={[
                {
                  value: "mix",
                  label: midi ? `All ${input.channels} channels` : `Mix all ${input.channels}`,
                },
                ...Array.from({ length: input.channels }, (_, i) => ({
                  value: String(i),
                  label: `Channel ${i + 1}`,
                })),
              ]}
              value={input.source.channel === null ? "mix" : String(input.source.channel)}
              onChange={(value) =>
                onSource({
                  ...input.source,
                  channel: value === null || value === "mix" ? null : Number(value),
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
              {outOfRange} note{outOfRange === 1 ? "" : "s"} arrived outside the note range and
              were dropped. Widen the range under MIDI, or transpose what is playing.
            </Text>
          </Alert>
        )}

        {noMidiPorts && <LoopMidiHint />}

        {input?.error && (
          <Alert color="red" variant="light" title="Source">
            <Text size="xs" lh={1.4}>
              {input.error}
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
 * What is actually being listened to. The selection alone does not say: "the
 * system default" names no device, and the rate comes from the endpoint rather
 * than from anything that was asked for.
 */
function describe(input: InputState | null): string {
  if (!input) return "Start the engine to choose what to listen to.";

  if (input.kind === "midi") {
    const channel =
      input.source.channel === null
        ? "all channels"
        : `channel ${input.source.channel + 1} of ${input.channels}`;
    return `${input.deviceName} · midi · ${channel}. Notes land across the strip by pitch and velocity sets brightness; the range and glow are under MIDI.`;
  }

  const parts = [
    input.deviceName,
    // Second, not buried at the end: which half of a device is being read is the
    // thing most likely to be wrong, and the least visible once chosen.
    input.kind,
    `${(input.sampleRate / 1000).toFixed(1)} kHz`,
    input.source.channel === null
      ? `${input.channels} ch mixed`
      : `channel ${input.source.channel + 1} of ${input.channels}`,
  ];

  // The failure this whole control exists for, described by its symptom rather
  // than its cause — "Spotify shows up, my DAW doesn't" is how it is met.
  const hint =
    input.kind === "loopback"
      ? " Loopback hears anything Windows mixes, but never a DAW on an ASIO driver — ASIO bypasses Windows entirely. Select this interface's input for that, or send it MIDI instead."
      : "";

  return `${parts.join(" · ")}.${hint}`;
}
