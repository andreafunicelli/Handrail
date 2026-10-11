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
  /** File editor view options. */
  editorWrap: boolean;
  editorWhitespace: boolean;
  editorMinimap: boolean;
  editorGuides: boolean;
  editorFontSize: number;
  /** The bar under SSH terminals: server load and tunnels. */
  sessionBar: boolean;
}

const DEFAULTS: Prefs = {
  theme: "dark",
  fontSize: 13,
  scheme: "quiet",
  autoLock: 15,
  sidebarWidth: 264,
  aiWidth: 380,
  aiOpen: true,
  editorWrap: false,
  editorWhitespace: false,
  editorMinimap: true,
  editorGuides: true,
  editorFontSize: 13,
  sessionBar: true,
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
  editorWrap = $state(DEFAULTS.editorWrap);
  editorWhitespace = $state(DEFAULTS.editorWhitespace);
  editorMinimap = $state(DEFAULTS.editorMinimap);
  editorGuides = $state(DEFAULTS.editorGuides);
  editorFontSize = $state(DEFAULTS.editorFontSize);
  sessionBar = $state(DEFAULTS.sessionBar);
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
    this.editorWrap = p.editorWrap;
    this.editorWhitespace = p.editorWhitespace;
    this.editorMinimap = p.editorMinimap;
    this.editorGuides = p.editorGuides;
    this.editorFontSize = p.editorFontSize;
    this.sessionBar = p.sessionBar;
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
          editorWrap: this.editorWrap,
          editorWhitespace: this.editorWhitespace,
          editorMinimap: this.editorMinimap,
          editorGuides: this.editorGuides,
          editorFontSize: this.editorFontSize,
          sessionBar: this.sessionBar,
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

  /** Editor text size, kept between 9 and 32 px; `0` resets it. */
  zoomEditor(step: number) {
    this.editorFontSize = step === 0 ? DEFAULTS.editorFontSize : Math.min(32, Math.max(9, this.editorFontSize + step));
  }
}

export const settings = new Settings();
