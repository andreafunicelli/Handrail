import { en, type MessageKey } from "./en";
import { it } from "./it";
import { locale, type Lang } from "./locale.svelte";

const catalogs: Record<Lang, Partial<Record<MessageKey, string>>> = { en, it };

/** Translate `key` in the current language (English when it has no translation), replacing
 *  `{name}` placeholders from `params`. Reading the language here makes callers re-render when
 *  it changes. */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const msg = catalogs[locale.lang][key] ?? en[key] ?? key;
  if (!params) return msg;
  // Numbers stay as they are (ids, ports); counts are formatted by the caller (fmtNumber).
  return msg.replace(/\{(\w+)\}/g, (m, k: string) => (k in params ? String(params[k]) : m));
}

/** A number in the current language's format. */
export function fmtNumber(n: number, opts?: Intl.NumberFormatOptions): string {
  return n.toLocaleString(locale.tag, opts);
}

/** A date and time in the current language's format. */
export function fmtDateTime(d: Date | number, opts?: Intl.DateTimeFormatOptions): string {
  return new Date(d).toLocaleString(locale.tag, opts ?? { dateStyle: "medium", timeStyle: "short" });
}

export { locale, LANGS, type Lang } from "./locale.svelte";
export type { MessageKey };
