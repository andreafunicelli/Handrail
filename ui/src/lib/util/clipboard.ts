/** Copy `text`, falling back to a hidden textarea when the async Clipboard API is refused (an
 *  unfocused webview, a missing permission). Restores the user's selection afterwards.
 *  Fallback from Interior's copy-button (github.com/ddoemonn/interior, MIT). */
export async function copyText(text: string): Promise<boolean> {
  if (!text) return false;
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return copyFallback(text);
  }
}

function copyFallback(text: string): boolean {
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  Object.assign(area.style, { position: "fixed", top: "0", left: "0", opacity: "0" });
  document.body.appendChild(area);
  const selection = document.getSelection();
  const previous = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
  const focused = document.activeElement as HTMLElement | null;
  area.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  area.remove();
  if (selection && previous) {
    selection.removeAllRanges();
    selection.addRange(previous);
  }
  focused?.focus?.();
  return ok;
}
