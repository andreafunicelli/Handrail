import { en, type MessageKey } from "./en";

const messages: Record<MessageKey, string> = en;

/** Translate `key`, replacing `{name}` placeholders from `params`. */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const msg = messages[key] ?? key;
  if (!params) return msg;
  return msg.replace(/\{(\w+)\}/g, (m, k: string) => (k in params ? String(params[k]) : m));
}

export type { MessageKey };
