import { load, save } from "../util/storage";

export type Lang = "en" | "it";
export const LANGS: { id: Lang; name: string }[] = [
  { id: "en", name: "English" },
  { id: "it", name: "Italiano" },
];

function initial(): Lang {
  const saved = load<string | null>("lang", null);
  if (saved === "en" || saved === "it") return saved;
  return "en";
}

/** The interface language: reactive, so text re-renders when it changes. Kept per device
 *  (localStorage), like the other display preferences. */
class Locale {
  lang = $state<Lang>(initial());

  set(lang: Lang) {
    this.lang = lang;
    save("lang", lang);
    if (typeof document !== "undefined") document.documentElement.lang = lang;
  }

  /** BCP 47 tag for number and date formatting. */
  tag = $derived(this.lang === "it" ? "it-IT" : "en-US");
}

export const locale = new Locale();
if (typeof document !== "undefined") document.documentElement.lang = locale.lang;
