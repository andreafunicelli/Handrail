import type { ApprovalMode, Environment } from "../api/types";

/**
 * Guard for switching approval mode. Auto on a prod connection requires the user to type the
 * connection name exactly (spec §4/§14: prod cannot be Auto silently).
 */
export function needsTypedConfirm(env: Environment, mode: ApprovalMode): boolean {
  return env === "prod" && mode === "auto";
}

/** The mode the backend enforces (and reports in SessionInfo): Auto on production runs as Ask writes. */
export function effectiveMode(env: Environment, mode: ApprovalMode): ApprovalMode {
  return env === "prod" && mode === "auto" ? "confirm_writes" : mode;
}

export function canSetApprovalMode(
  env: Environment,
  mode: ApprovalMode,
  connectionName: string,
  typed: string | null,
): boolean {
  if (!needsTypedConfirm(env, mode)) return true;
  return typed != null && connectionName.length > 0 && typed.trim() === connectionName;
}

/** How long (ms) a just-shown approval or trust prompt ignores the keys that would accept it: a
 *  keystroke meant for the terminal or the chat must not land on it. */
export const ARM_MS = 500;

/** Whether keys pressed now may accept something shown at `shownAt` (a `performance.now()`). */
export function armed(shownAt: number, now = performance.now()): boolean {
  return now - shownAt >= ARM_MS;
}

const NON_TEXT_INPUTS = ["button", "checkbox", "radio", "submit", "reset", "range", "color", "file", "image"];

/** Whether `el` is where the user types (a text field, text area, editable region, the
 *  terminal's hidden text area): a card that pops up never takes focus from it. */
export function takesText(el: Element | null): boolean {
  if (!el) return false;
  if (el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
  if (el.tagName === "INPUT") return !NON_TEXT_INPUTS.includes((el as HTMLInputElement).type);
  return (el as HTMLElement).isContentEditable === true;
}

/** Characters that change how text looks without showing up: bidi embeddings, overrides and
 *  isolates, zero-width and invisible format characters, Unicode tag characters. */
const HIDDEN = /[­؜᠎​-‏‪-‮⁠-⁤⁦-⁩﻿\u{E0000}-\u{E007F}]/gu;

/** `text` with every hidden character shown as `⟨U+202E⟩`, and whether there was any: what an
 *  approval shows must be what runs. */
export function revealHidden(text: string): { text: string; found: boolean } {
  let found = false;
  const out = text.replace(HIDDEN, (c) => {
    found = true;
    return `⟨U+${c.codePointAt(0)!.toString(16).toUpperCase().padStart(4, "0")}⟩`;
  });
  return { text: out, found };
}

/** Line breaks in pasted text (each one can run what precedes it in a shell). */
export function lineBreaks(text: string): number {
  return text.match(/\r\n|\r|\n/g)?.length ?? 0;
}
