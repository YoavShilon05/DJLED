/**
 * The twelve preset slots, as the editor labels them.
 *
 * Everything about presets *happens* in the engine — the shows live on disk
 * there, and `ctrl+alt+F1`…`F12` are registered with Windows so they work with
 * this page closed. See `engine/src/presets.rs`. What is left over here is
 * naming: turning a slot index into something a dropdown can show, which is
 * exactly the kind of off-by-one that is invisible until somebody presses the
 * wrong key and gets the wrong show.
 *
 * Slots are 0-based and function keys are 1-based. This file is the only place
 * those two meet.
 */

import type { PresetInfo } from "../engine";

/** Slots, and therefore hotkeys. Matches `presets::SLOTS`; the keyboard fixes
 *  the number, because there is no thirteenth function key. */
export const PRESET_SLOTS = 12;

/** The combination that switches to a slot, written the way it is pressed. */
export function hotkeyLabel(slot: number): string {
  return `Ctrl+Alt+F${slot + 1}`;
}

/** Just the key, for a `Kbd` badge where the modifiers are already said once. */
export function keyLabel(slot: number): string {
  return `F${slot + 1}`;
}

/**
 * A slot's name, for a slot that may not exist yet.
 *
 * The engine names every slot, so `info` is normally there. It is not while
 * offline, and a dropdown that renders twelve blanks until the engine appears
 * would be worse than one that counts.
 */
export function presetName(slot: number, info?: PresetInfo): string {
  return info?.name?.trim() || `Preset ${slot + 1}`;
}

/** One slot, as the preset bar draws it. */
export interface PresetSlot {
  slot: number;
  /** `F7` — just the key; the modifiers are said once, above the row. */
  key: string;
  name: string;
  /**
   * Whether anything has been authored into it.
   *
   * An empty slot is drawn quietly rather than hidden, and claimed empty only
   * when the engine has said so: offline it has made no claim either way, and
   * a row of twelve "empty" chips would be a lie about shows that exist.
   */
  stored: boolean;
}

/**
 * All twelve slots, always, in hotkey order.
 *
 * Always twelve and always in order, because the row is also how somebody
 * learns which key reaches which show — hiding the empty ones would renumber
 * the rest against keys that do not move, and the fourth chip would be
 * ctrl+alt+F7.
 */
export function presetSlots(presets: PresetInfo[]): PresetSlot[] {
  return Array.from({ length: PRESET_SLOTS }, (_, slot) => {
    const info = presets[slot];
    return {
      slot,
      key: keyLabel(slot),
      name: presetName(slot, info),
      stored: info?.stored !== false,
    };
  });
}
