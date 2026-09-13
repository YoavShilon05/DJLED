import { memo, useEffect, useRef, useState } from "react";
import { Box, Group, Paper, Text, TextInput, Tooltip, UnstyledButton } from "@mantine/core";

import { hotkeyLabel, presetSlots, type PresetSlot } from "../config/presets";
import type { PresetInfo } from "../engine";
import { InfoDot } from "./Field";

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
 * All twelve shows, on screen at once.
 *
 * This used to be a dropdown in a column of panels, which is the wrong shape
 * for the only control anyone touches with the lights on: switching show is a
 * live action, and a live action must not cost a scroll, a click to open and a
 * read of twelve rows to find the one you already know the name of. Twelve
 * chips fit across the top, so the whole set is both the control and the
 * legend — which key reaches which show is readable without pressing anything.
 *
 * It is a *selector*, not a loader: choosing a slot puts that show on the wall
 * and points every subsequent edit at it. There is no save button because there
 * is nothing to save — every edit is already in the live slot by the time the
 * pointer comes up, which is what makes a hotkey switch safe. See
 * `engine/src/presets.rs`.
 *
 * Renaming happens in place, on the chip, because that is where the name is.
 * The old panel had a text box somewhere below a dropdown, so the thing being
 * typed and the thing being changed were never on screen together.
 *
 * Memoised: nothing in here is driven by a frame, and a frame arrives thirty
 * times a second. See the note on `frame` in `App.tsx`.
 */
export const PresetBar = memo(function PresetBar({
  presets,
  active,
  onSelect,
  onRename,
  connected,
}: Props) {
  const slots = presetSlots(presets);
  /** The slot whose chip has turned into a text box, if any. */
  const [editing, setEditing] = useState<number | null>(null);

  // Renaming a slot that is no longer on screen, or that the engine has just
  // swapped out from under us, would commit a name into whatever took its
  // place.
  useEffect(() => setEditing(null), [active]);

  return (
    <Paper p={6}>
      <Group gap={6} wrap="nowrap" align="stretch">
        <Group gap={2} wrap="nowrap" align="center" style={{ flexShrink: 0 }}>
          <Text size="xs" c="dimmed" fw={700} tt="uppercase" lts="0.08em">
            Shows
          </Text>
          <InfoDot label="Shows">
            {connected
              ? "Twelve shows, saved as you edit them. Ctrl+Alt+F1…F12 switch between them from any window — this page does not need to be open, or in front. Click the live chip to rename it; double-click any chip to select and rename in one go."
              : "Presets live in the engine, so that a hotkey still works with this page closed. Nothing to select until it is running."}
          </InfoDot>
        </Group>
        {slots.map((slot) => (
          <PresetChip
            key={slot.slot}
            slot={slot}
            active={slot.slot === active}
            connected={connected}
            editing={editing === slot.slot}
            onSelect={() => onSelect(slot.slot)}
            onEdit={() => setEditing(slot.slot)}
            onCommit={(name) => {
              setEditing(null);
              if (name !== slot.name) onRename(slot.slot, name);
            }}
            onCancel={() => setEditing(null)}
          />
        ))}
      </Group>
    </Paper>
  );
});

interface ChipProps {
  slot: PresetSlot;
  active: boolean;
  connected: boolean;
  editing: boolean;
  onSelect: () => void;
  onEdit: () => void;
  onCommit: (name: string) => void;
  onCancel: () => void;
}

function PresetChip({
  slot,
  active,
  connected,
  editing,
  onSelect,
  onEdit,
  onCommit,
  onCancel,
}: ChipProps) {
  const [draft, setDraft] = useState(slot.name);
  // Committed on blur, so a click on another chip both saves this name and
  // moves on — the same gesture doing the obvious thing twice.
  const draftRef = useRef(draft);
  draftRef.current = draft;

  useEffect(() => {
    if (editing) setDraft(slot.name);
  }, [editing, slot.name]);

  if (editing) {
    return (
      <TextInput
        size="xs"
        autoFocus
        flex={1}
        miw={0}
        value={draft}
        onChange={(e) => setDraft(e.currentTarget.value)}
        onBlur={() => onCommit(draftRef.current.trim())}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") {
            // Reset first: the blur that follows commits whatever is in the
            // draft, and Escape has to mean the name it started with.
            setDraft(slot.name);
            draftRef.current = slot.name;
            onCancel();
          }
        }}
      />
    );
  }

  return (
    <Tooltip
      label={`${hotkeyLabel(slot.slot)} · ${slot.name}${slot.stored ? "" : " · empty"}${
        active ? " · click again to rename" : ""
      }`}
      openDelay={500}
    >
      <UnstyledButton
        flex={1}
        miw={0}
        onClick={() => (active ? onEdit() : onSelect())}
        // The name is on the chip, so renaming is on the chip too — and a
        // double-click reaches it from any slot, not only the live one.
        onDoubleClick={() => {
          onSelect();
          onEdit();
        }}
        disabled={!connected}
        aria-current={active}
        p={6}
        bg={active ? "brand.6" : "dark.6"}
        c={active ? "white" : slot.stored ? "dark.0" : "dark.2"}
        opacity={connected ? 1 : 0.45}
        style={{
          borderRadius: "var(--mantine-radius-sm)",
          textAlign: "center",
          overflow: "hidden",
          cursor: connected ? "pointer" : "not-allowed",
        }}
      >
        <Text size="10px" fw={700} ff="monospace" lh={1.2} opacity={0.75}>
          {slot.key}
        </Text>
        <Box style={{ overflow: "hidden" }}>
          <Text size="xs" fw={active ? 700 : 500} lh={1.3} truncate fs={slot.stored ? undefined : "italic"}>
            {slot.name}
          </Text>
        </Box>
      </UnstyledButton>
    </Tooltip>
  );
}
