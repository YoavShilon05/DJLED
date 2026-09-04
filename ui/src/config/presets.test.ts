import { describe, expect, it } from "vitest";

import { PRESET_SLOTS, hotkeyLabel, keyLabel, presetName, presetOptions } from "./presets";
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

describe("the dropdown", () => {
  /**
   * Always twelve entries, in hotkey order.
   *
   * Hiding the empty slots would renumber the list against keys that do not
   * move — the fourth row would be ctrl+alt+F7 — and the list is also how
   * somebody learns which key reaches which show.
   */
  it("lists every slot in hotkey order, however few are used", () => {
    const options = presetOptions([info("Warm", true)]);
    expect(options).toHaveLength(PRESET_SLOTS);
    expect(options[0].value).toBe("0");
    expect(options[11].value).toBe("11");
    expect(options[0].label).toBe("F1 · Warm");
  });

  it("says which slots have nothing in them", () => {
    const options = presetOptions([info("Warm", true), info("Preset 2", false)]);
    expect(options[0].label).toBe("F1 · Warm");
    expect(options[1].label).toBe("F2 · Preset 2 — empty");
  });

  /**
   * Offline the engine has told the editor nothing, and a dropdown of twelve
   * blanks is worse than one that counts. "Empty" is a claim about the engine's
   * store, so it is not made when there is no engine to have made it.
   */
  it("counts rather than blanks while the engine is away", () => {
    const options = presetOptions([]);
    expect(options).toHaveLength(PRESET_SLOTS);
    expect(options[0].label).toBe("F1 · Preset 1");
    expect(options[11].label).toBe("F12 · Preset 12");
  });
});
