import { load, save } from "../util/storage";
import { schemeById } from "../terminal/schemes";
import { terminals } from "../terminal/host";

export type Theme = "dark" | "light" | "system";

interface Prefs {
  theme: Theme;
  fontSize: number;
  scheme: string;
  autoLock: number;
  sidebarWidth: number;
  aiWidth: number;
  aiOpen: boolean;
}

const DEFAULTS: Prefs = {
  theme: "dark",
  fontSize: 13,
  scheme: "quiet",
  autoLock: 15,
  sidebarWidth: 264,
  aiWidth: 380,
  aiOpen: true,
};

/** Local, per-device UI preferences (persisted to localStorage). */
class Settings {
  theme = $state<Theme>(DEFAULTS.theme);
  fontSize = $state(DEFAULTS.fontSize);
  scheme = $state(DEFAULTS.scheme);
  autoLock = $state(DEFAULTS.autoLock);
  sidebarWidth = $state(DEFAULTS.sidebarWidth);
  aiWidth = $state(DEFAULTS.aiWidth);
  aiOpen = $state(DEFAULTS.aiOpen);
  systemDark = $state(true);

  resolvedTheme = $derived<"dark" | "light">(this.theme === "system" ? (this.systemDark ? "dark" : "light") : this.theme);
  terminalTheme = $derived.by(() => {
    // The default scheme follows the app theme.
    if (this.scheme === "quiet" && this.resolvedTheme === "light") return schemeById("quiet-light").theme;
    return schemeById(this.scheme).theme;
  });

  constructor() {
    const p = { ...DEFAULTS, ...load<Partial<Prefs>>("prefs", {}) };
    this.theme = p.theme;
    this.fontSize = p.fontSize;
    this.scheme = p.scheme;
    this.autoLock = p.autoLock;
    this.sidebarWidth = p.sidebarWidth;
    this.aiWidth = p.aiWidth;
    this.aiOpen = p.aiOpen;
    if (typeof window !== "undefined" && window.matchMedia) {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      this.systemDark = mq.matches;
      mq.addEventListener?.("change", (e) => (this.systemDark = e.matches));
    }
    $effect.root(() => {
      $effect(() => {
        save("prefs", {
          theme: this.theme,
          fontSize: this.fontSize,
          scheme: this.scheme,
          autoLock: this.autoLock,
          sidebarWidth: this.sidebarWidth,
          aiWidth: this.aiWidth,
          aiOpen: this.aiOpen,
        } satisfies Prefs);
      });
      $effect(() => {
        if (typeof document !== "undefined") document.documentElement.dataset.theme = this.resolvedTheme;
      });
      $effect(() => {
        terminals.configure(this.fontSize, this.terminalTheme);
      });
    });
  }
}

export const settings = new Settings();
