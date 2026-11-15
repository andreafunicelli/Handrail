export const isMac = typeof navigator !== "undefined" && /Mac|iP/.test(navigator.platform);
/** The shortcut modifier as shown in hints: "⌘" on a Mac, "Ctrl" elsewhere. */
export const MOD = isMac ? "⌘" : "Ctrl";
