import { memo } from "react";
import { Checkbox, RangeSlider, Slider, Stack, Text } from "@mantine/core";

import type { EditorLayer } from "../config/editor";
import {
  MAX_NOTE,
  MIN_NOTE,
  MIN_NOTE_SPAN,
  noteHz,
  noteName,
  noteRange,
} from "../config/notes";
import { Field } from "./Field";

interface Props {
  /** The layer being edited. Every layer has its own note axis, because every
   *  layer can be on its own port — or on the same port, on another channel. */
  layer: EditorLayer;
  onChange: (layer: EditorLayer) => void;
  /** True when a MIDI port is what is actually driving *this layer*. It stays
   *  editable otherwise — a look is authored, not discovered — but it says so
   *  rather than pretending the sliders are doing something. */
  live: boolean;
}

/**
 * The note axis, as a shape rather than as numbers.
 *
 * Only three controls, and the first is the one that matters: the range is
 * *stretched* across the whole strip, so narrowing it does not crop the display
 * — it magnifies. Two octaves across a wall is a legitimate and very different
 * look from eighty-eight keys, and the hint says so, because nothing about a
 * pair of note numbers suggests it.
 *
 * Memoised: nothing in here is driven by a frame, and a frame arrives thirty
 * times a second. See the note on `frame` in `App.tsx`.
 */
export const MidiPanel = memo(function MidiPanel({ layer, onChange, live }: Props) {
  const { midi } = layer;
  const [low, high] = noteRange(midi.lowNote, midi.highNote);
  const octaves = (high - low) / 12;

  const setRange = ([lowNote, highNote]: [number, number]) =>
    onChange({ ...layer, midi: { ...midi, lowNote, highNote } });

  return (
    <Stack gap="md">
      {!live && (
        <Text size="xs" c="dimmed" lh={1.35}>
          Saved with the layer, but nothing here changes the strip until a MIDI port is its
          source.
        </Text>
      )}

      <Field
        label="Note range"
        value={`${noteName(low)} – ${noteName(high)}`}
        hint={`${octaves.toFixed(1)} octaves end to end, ${high - low + 1} keys.`}
        info="Stretched across the whole strip, so this magnifies rather than crops: narrowing it gives each remaining note more wall, it does not hide the rest of it. Notes outside the range are dropped."
      >
        <RangeSlider
          min={MIN_NOTE}
          max={MAX_NOTE}
          step={1}
          minRange={MIN_NOTE_SPAN}
          value={[low, high]}
          onChange={setRange}
          label={(v) => `${noteName(v)} · ${noteHz(v).toFixed(0)} Hz`}
        />
      </Field>

      <Field
        label="Note glow"
        value={midi.spread === 0 ? "off" : `${midi.spread.toFixed(1)} st`}
        hint={midi.spread === 0 ? "One hard bar per note." : "Notes bleed into their neighbours."}
        info="How far a note spreads, in semitones. Wide enough and a chord reads as one block rather than as its notes."
      >
        <Slider
          min={0}
          max={6}
          step={0.1}
          value={midi.spread}
          onChange={(spread) => onChange({ ...layer, midi: { ...midi, spread } })}
          label={(v) => (v === 0 ? "off" : `${v.toFixed(1)} st`)}
        />
      </Field>

      <Stack gap={4}>
        <Checkbox
          label="Sustain pedal"
          checked={midi.sustain}
          onChange={(e) =>
            onChange({ ...layer, midi: { ...midi, sustain: e.currentTarget.checked } })
          }
        />
        <Text size="xs" c="dimmed" lh={1.35}>
          {midi.sustain
            ? "CC64 holds released notes lit, as it holds them sounding."
            : "CC64 ignored — notes go out when the key does."}
        </Text>
      </Stack>
    </Stack>
  );
});
