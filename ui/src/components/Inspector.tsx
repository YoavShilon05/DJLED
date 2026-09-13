import { memo, useState } from "react";
import { Indicator, Paper, Stack, Tabs, Text } from "@mantine/core";

import type { EditorLayer } from "../config/editor";
import type { InputDevice, LayerStatus } from "../engine";
import { MidiPanel } from "./MidiPanel";
import { PanelHeading } from "./PanelHeading";
import { ShapePanel } from "./ShapePanel";
import { SourcePanel } from "./SourcePanel";

interface Props {
  /** The selected layer. Everything in here edits it and nothing else. */
  layer: EditorLayer;
  onChange: (layer: EditorLayer) => void;
  devices: InputDevice[];
  status: LayerStatus | null;
  connected: boolean;
  onRefresh: () => void;
  /** True when a MIDI port is driving this layer, not merely selected on it. */
  midi: boolean;
}

/**
 * Everything about the selected layer, in one box.
 *
 * This replaces three stacked panels — Source, Analysis and MIDI — each with
 * its own border, its own heading repeating the layer's name, and its own
 * paragraph of explanation. Three quarters of that column was chrome and prose
 * for settings nobody was looking at, and the MIDI panel was on screen at full
 * height for audio layers just to say that it did nothing.
 *
 * Tabs are the right shape because the three groups are genuinely alternatives:
 * choosing a device, shaping the response, and laying out a note range are
 * separate jobs done at separate times, and none of them needs the other two
 * visible. The tab persists across selecting a different layer, which makes
 * comparing one setting across a stack a matter of clicking rows.
 *
 * Memoised: nothing in here is driven by a frame, and a frame arrives thirty
 * times a second. See the note on `frame` in `App.tsx`.
 */
export const Inspector = memo(function Inspector({
  layer,
  onChange,
  devices,
  status,
  connected,
  onRefresh,
  midi,
}: Props) {
  const [tab, setTab] = useState<string | null>("source");

  return (
    <Paper p="sm">
      <Stack gap="xs">
        <PanelHeading
          title="Layer"
          info="Everything here belongs to the selected row of the stack, travels with it through a reorder or a duplicate, and is saved into the live preset as it is edited. Master brightness is not here — it is the whole strip, so it lives in the header."
          right={
            <Text size="xs" c="dimmed" truncate maw={140}>
              {layer.name}
            </Text>
          }
        />

        <Tabs value={tab} onChange={setTab} variant="default">
          <Tabs.List grow>
            <Tabs.Tab value="source">
              {/* A dead source is the one thing in here that is urgent, and it
                  is two clicks away on a tab that is not open. */}
              <Indicator size={6} color="red" disabled={!status?.error} offset={-4}>
                <Text size="xs">Source</Text>
              </Indicator>
            </Tabs.Tab>
            <Tabs.Tab value="shape">
              <Text size="xs">Shape</Text>
            </Tabs.Tab>
            <Tabs.Tab value="notes">
              <Indicator size={6} color="teal" disabled={!midi} offset={-4}>
                <Text size="xs">Notes</Text>
              </Indicator>
            </Tabs.Tab>
          </Tabs.List>

          <Tabs.Panel value="source" pt="md">
            <SourcePanel
              layer={layer}
              onChange={onChange}
              devices={devices}
              status={status}
              connected={connected}
              onRefresh={onRefresh}
            />
          </Tabs.Panel>

          <Tabs.Panel value="shape" pt="md">
            <ShapePanel
              layer={layer}
              onChange={onChange}
              sampleRate={status?.sampleRate ?? 0}
            />
          </Tabs.Panel>

          <Tabs.Panel value="notes" pt="md">
            <MidiPanel layer={layer} onChange={onChange} live={midi} />
          </Tabs.Panel>
        </Tabs>
      </Stack>
    </Paper>
  );
});
