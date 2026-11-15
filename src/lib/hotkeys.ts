// Hotkey accelerators such as "CmdOrCtrl+Alt+R", and how to show them.

import type { Hotkeys } from "./types";

/** Must match `hotkeys::Bindings::default` in the backend. */
export const DEFAULT_HOTKEYS: Hotkeys = { toggle: "CmdOrCtrl+Alt+R", pause: "CmdOrCtrl+Alt+P", marker: "CmdOrCtrl+Alt+M" };

const mac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);

export function hotkeyLabel(accel: string): string {
  return accel
    .split("+")
    .map((p) => {
      const k = p.trim();
      if (/^(cmdorctrl|commandorcontrol)$/i.test(k)) return mac ? "⌘" : "Ctrl";
      if (/^(alt|option)$/i.test(k)) return mac ? "⌥" : "Alt";
      if (/^shift$/i.test(k)) return mac ? "⇧" : "Shift";
      if (/^(super|cmd|command|meta)$/i.test(k)) return mac ? "⌘" : "Super";
      return k.replace(/^Key(?=.$)/, "");
    })
    .join(mac ? "" : "+");
}

/** The accelerator for a key press, or null while only modifiers are held. */
export function accelFromEvent(e: { key: string; code: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean }): string | null {
  if (["Control", "Meta", "Alt", "Shift", "OS"].includes(e.key)) return null;
  const parts: string[] = [];
  if (mac ? e.metaKey : e.ctrlKey) parts.push("CmdOrCtrl");
  if (mac && e.ctrlKey) parts.push("Ctrl");
  if (!mac && e.metaKey) parts.push("Super");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  // A hotkey on its own key would fire while typing.
  if (parts.length === 0 || (parts.length === 1 && parts[0] === "Shift")) return null;
  const key = /^Key[A-Z]$/.test(e.code) ? e.code.slice(3) : /^Digit\d$/.test(e.code) ? e.code.slice(5) : e.key.length === 1 ? e.key.toUpperCase() : e.key;
  parts.push(key);
  return parts.join("+");
}
