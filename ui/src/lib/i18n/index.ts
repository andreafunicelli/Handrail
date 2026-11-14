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

/** Keys with plural forms: `<key>_one` and `<key>_other` (CLDR categories) in the catalogs. */
export type PluralKey = { [K in MessageKey]: K extends `${infer B}_other` ? B : never }[MessageKey];

/** `key` in the plural form for `n` (`{n}` is the count, formatted), e.g. "1 row" / "2 rows".
 *  A category the catalog lacks (Italian "many") falls back to `_other`. */
export function tn(key: PluralKey, n: number, params?: Record<string, string | number>): string {
  const form = `${key}_${new Intl.PluralRules(locale.tag).select(n)}`;
  const k = (form in catalogs[locale.lang] || form in en ? form : `${key}_other`) as MessageKey;
  return t(k, { n: fmtNumber(n), ...params });
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
