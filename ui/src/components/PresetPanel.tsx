import { useEffect, useState } from "react";
import { Kbd, Paper, Select, Stack, Text, TextInput } from "@mantine/core";

import { PRESET_SLOTS, hotkeyLabel, presetName, presetOptions } from "../config/presets";
import type { PresetInfo } from "../engine";
import { Field } from "./Field";

interface Props {
  /** The twelve slots as the engine holds them. Empty while offline. */
  presets: PresetInfo[];
  /** Which slot is live, and therefore which one edits are saved into. */
  active: number;
  onSelect: (slot: number) => void;
  onRename: (slot: number, name: string) => void;
  connected: boolean;
}

/**
 * Which preset is being edited.
 *
 * The dropdown is a *selector*, not a loader: choosing a slot puts that show on
 * the wall and points every subsequent edit at it. There is no save button
 * because there is nothing to save — every edit is already in the live slot by
 * the time the pointer comes up, which is what makes a hotkey switch safe. See
 * `engine/src/presets.rs`.
 *
 * The keyboard does exactly the same thing from anywhere on the desktop, so
 * this panel is as much a legend as a control: it is where somebody learns that
 * ctrl+alt+F7 is the one with the slow red wash, without having to try it
 * during a set.
 */
export function PresetPanel({ presets, active, onSelect, onRename, connected }: Props) {
  const info = presets[active];

  // Renaming is typed a character at a time and each keystroke would otherwise
  // be a message, an announce and a re-render of the dropdown that the input is
  // inside. Held locally and committed on blur or Enter instead.
  const [draft, setDraft] = useState(() => presetName(active, info));
  useEffect(() => setDraft(presetName(active, info)), [active, info]);

  const commit = () => {
    const name = draft.trim();
    if (name !== presetName(active, info)) onRename(active, name);
  };

  return (
    <Paper>
      <Stack gap="md">
        <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
          Presets
        </Text>

        <Field
          label="Editing"
          value={<Kbd size="xs">{hotkeyLabel(active)}</Kbd>}
          hint={
            connected
              ? `${PRESET_SLOTS} shows, saved as you edit them. Ctrl+Alt+F1…F12 switch between them from any window — the browser does not need to be open, or in front.`
              : "Presets live in the engine, so that a hotkey still works with this page closed. Nothing to select until it is running."
          }
        >
          <Select
            data={presetOptions(presets)}
            value={String(active)}
            onChange={(value) => value !== null && onSelect(Number(value))}
            disabled={!connected}
            allowDeselect={false}
            comboboxProps={{ withinPortal: true }}
          />
        </Field>

        <Field
          label="Name"
          hint="Labels the dropdown and nothing else. Blank falls back to the slot number."
        >
          <TextInput
            size="xs"
            value={draft}
            onChange={(e) => setDraft(e.currentTarget.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") setDraft(presetName(active, info));
            }}
            disabled={!connected}
          />
        </Field>
      </Stack>
    </Paper>
  );
}
