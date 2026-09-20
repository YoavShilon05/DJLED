import { memo } from "react";
import { Checkbox, RangeSlider, Slider, Stack, Text } from "@mantine/core";

import type { EditorLayer } from "../config/editor";
import {
  MAX_DECAY_CEILING,
  MAX_NOTE,
  MIN_NOTE,
  MIN_NOTE_SPAN,
  decayRange,
  noteHz,
  noteName,
  noteRange,
} from "../config/notes";
import { ChannelColors } from "./ChannelColors";
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
/** A decay end as the panel says it: "instant" reads as what it does, where
 *  "0.00 s" reads as a number someone forgot to set. */
const secs = (ms: number) => (ms === 0 ? "instant" : `${(ms / 1000).toFixed(2)} s`);

export const MidiPanel = memo(function MidiPanel({ layer, onChange, live }: Props) {
  const { midi } = layer;
  const [low, high] = noteRange(midi.lowNote, midi.highNote);
  const { minDecayMs, maxDecayMs } = decayRange(midi.minDecayMs, midi.maxDecayMs);
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

      <Field
        label="Channel colour"
        hint="Sixteen channels, sixteen colours."
        info="Every note is painted in the colour of the MIDI channel it arrived on, so a stack of parts down one cable reads as a stack of colours on one layer. Each colour carries an opacity: at full it is the note's colour outright, and below it the layer's own colour field shows through underneath. A channel is no longer something the layer filters on — all sixteen reach it, and what tells them apart is what they look like."
      >
        <ChannelColors
          colors={midi.channelColors}
          onChange={(channelColors) => onChange({ ...layer, midi: { ...midi, channelColors } })}
        />
      </Field>

      <Field
        label="Decay time"
        value={`${secs(minDecayMs)} – ${secs(maxDecayMs)}`}
        hint={
          minDecayMs === maxDecayMs
            ? "Every note-off fades the same, whatever its velocity."
            : `Min at a note-off of 0, max at 127.`
        }
        info="A note-off carries a velocity of its own, and it sets the fade: the left end at velocity 0, the right end at 127, linear in between. So this is the range a key can ask for rather than the fade itself — a floor above zero is what stops the softest release snapping, and closing the two ends together makes velocity stop mattering. A keyboard that does not sense release sends 64 for everything and lands in the middle; a sequencer spelling note-off as a note-on at velocity 0 carries no release velocity at all and falls back to the layer's Decay instead."
      >
        <RangeSlider
          min={0}
          max={MAX_DECAY_CEILING}
          step={50}
          minRange={0}
          value={[minDecayMs, maxDecayMs]}
          onChange={([min, max]) =>
            onChange({ ...layer, midi: { ...midi, minDecayMs: min, maxDecayMs: max } })
          }
          label={secs}
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
