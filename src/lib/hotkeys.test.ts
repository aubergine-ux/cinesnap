import { describe, expect, it } from "vitest";
import { accelFromEvent, hotkeyLabel } from "./hotkeys";

const press = (key: string, code: string, mods: Partial<Record<"ctrlKey" | "metaKey" | "altKey" | "shiftKey", boolean>> = {}) => ({
  key,
  code,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe("hotkeys", () => {
  it("shows accelerators the way the platform writes them", () => {
    expect(hotkeyLabel("CmdOrCtrl+Alt+R")).toBe("Ctrl+Alt+R");
    expect(hotkeyLabel("Shift+Super+KeyM")).toBe("Shift+Super+M");
  });

  it("turns a key press into an accelerator", () => {
    expect(accelFromEvent(press("r", "KeyR", { ctrlKey: true, altKey: true }))).toBe("CmdOrCtrl+Alt+R");
    expect(accelFromEvent(press("!", "Digit1", { ctrlKey: true, shiftKey: true }))).toBe("CmdOrCtrl+Shift+1");
    expect(accelFromEvent(press("F9", "F9", { altKey: true }))).toBe("Alt+F9");
  });

  it("refuses keys that would fire while typing", () => {
    expect(accelFromEvent(press("r", "KeyR"))).toBeNull();
    expect(accelFromEvent(press("R", "KeyR", { shiftKey: true }))).toBeNull();
    expect(accelFromEvent(press("Control", "ControlLeft", { ctrlKey: true }))).toBeNull();
  });
});
