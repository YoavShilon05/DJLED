import { describe, expect, it } from "vitest";

import { PRESET_SLOTS, hotkeyLabel, keyLabel, presetName, presetSlots } from "./presets";
import type { PresetInfo } from "../engine";

const info = (name: string, stored: boolean): PresetInfo => ({ name, stored });

describe("preset labels", () => {
  /** Slots are 0-based and function keys are 1-based. Off by one here and the
   *  dropdown names the wrong key for every slot in the list. */
  it("numbers function keys from one", () => {
    expect(hotkeyLabel(0)).toBe("Ctrl+Alt+F1");
    expect(hotkeyLabel(11)).toBe("Ctrl+Alt+F12");
    expect(keyLabel(0)).toBe("F1");
    expect(keyLabel(PRESET_SLOTS - 1)).toBe("F12");
  });

  /** Matches `presets::SLOTS`, and is fixed by the keyboard rather than chosen:
   *  there is no thirteenth function key to reach a thirteenth slot with. */
  it("has one slot per function key", () => {
    expect(PRESET_SLOTS).toBe(12);
  });

  it("falls back to the slot number when the engine has not named it", () => {
    expect(presetName(0)).toBe("Preset 1");
    expect(presetName(4, info("   ", true))).toBe("Preset 5");
    expect(presetName(4, info("Warm", true))).toBe("Warm");
  });
});

describe("the preset bar", () => {
  /**
   * Always twelve chips, in hotkey order.
   *
   * Hiding the empty slots would renumber the row against keys that do not
   * move — the fourth chip would be ctrl+alt+F7 — and the row is also how
   * somebody learns which key reaches which show.
   */
  it("lists every slot in hotkey order, however few are used", () => {
    const slots = presetSlots([info("Warm", true)]);
    expect(slots).toHaveLength(PRESET_SLOTS);
    expect(slots.map((s) => s.slot)).toEqual([...Array(PRESET_SLOTS).keys()]);
    expect(slots[0].key).toBe("F1");
    expect(slots[11].key).toBe("F12");
    expect(slots[0].name).toBe("Warm");
  });

  it("says which slots have nothing in them", () => {
    const slots = presetSlots([info("Warm", true), info("Preset 2", false)]);
    expect(slots[0].stored).toBe(true);
    expect(slots[1].stored).toBe(false);
  });

  /**
   * Offline the engine has told the editor nothing, and a row of twelve blanks
   * is worse than one that counts. "Empty" is a claim about the engine's store,
   * so it is not made when there is no engine to have made it.
   */
  it("counts rather than blanks while the engine is away", () => {
    const slots = presetSlots([]);
    expect(slots).toHaveLength(PRESET_SLOTS);
    expect(slots[0].name).toBe("Preset 1");
    expect(slots[11].name).toBe("Preset 12");
    expect(slots.every((s) => s.stored)).toBe(true);
  });
});
