import { t } from "../i18n";

/** "just now", "5 min ago", "3 h ago", "2 days ago"; older than a week: the date. */
export function relativeTime(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 45) return t("time.justNow");
  const m = Math.round(s / 60);
  if (m < 60) return t("time.minutes", { n: m });
  const h = Math.round(m / 60);
  if (h < 24) return t("time.hours", { n: h });
  const d = Math.round(h / 24);
  if (d < 7) return d === 1 ? t("time.yesterday") : t("time.days", { n: d });
  return new Date(ms).toLocaleDateString();
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}
