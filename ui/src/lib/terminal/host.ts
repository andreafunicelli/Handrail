// Keeps one xterm instance per session alive independently of the Svelte component tree, so
// switching tabs or re-arranging splits never loses scrollback. TerminalView attaches `el`.
import { Terminal, type IMarker, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api } from "../api";
import { strToB64 } from "../util/b64";

export const MONO_STACK = '"JetBrains Mono", "Cascadia Code", "SF Mono", Menlo, Consolas, "Liberation Mono", monospace';

interface AgentMark {
  command: string;
  actor: string;
  marker: IMarker;
  decorated: boolean;
}

export class TermHost {
  readonly term: Terminal;
  readonly fit = new FitAddon();
  readonly el: HTMLDivElement;
  opened = false;
  marks: AgentMark[] = [];
  private webgl: { dispose(): void } | null = null;

  constructor(
    readonly sessionId: string,
    opts: { fontSize: number; theme: ITheme },
  ) {
    this.el = document.createElement("div");
    this.el.className = "term-host";
    this.term = new Terminal({
      fontFamily: MONO_STACK,
      fontSize: opts.fontSize,
      lineHeight: 1.2,
      theme: opts.theme,
      cursorBlink: true,
      cursorStyle: "bar",
      cursorWidth: 2,
      scrollback: 10000,
      allowProposedApi: true,
      overviewRuler: { width: 6 },
      macOptionIsMeta: true,
    });
    this.term.loadAddon(this.fit);
    this.term.onData((d) => void api.sessionWrite(sessionId, strToB64(d)).catch(() => {}));
    this.term.onBinary((d) => {
      const bytes = Uint8Array.from(d, (c) => c.charCodeAt(0) & 0xff);
      let s = "";
      bytes.forEach((b) => (s += String.fromCharCode(b)));
      void api.sessionWrite(sessionId, btoa(s)).catch(() => {});
    });
  }

  /** Open into the (already attached) host element. Tries WebGL, falls back to the DOM renderer. */
  open() {
    if (this.opened) return;
    this.term.open(this.el);
    this.opened = true;
    for (const m of this.marks) this.decorate(m);
    // Touch devices use the DOM renderer: WebGL rendered blank under mobile emulation in testing.
    if (typeof matchMedia !== "undefined" && matchMedia("(pointer: coarse)").matches) return;
    import("@xterm/addon-webgl")
      .then(({ WebglAddon }) => {
        try {
          const gl = new WebglAddon();
          gl.onContextLoss(() => {
            gl.dispose();
            this.webgl = null;
          });
          this.term.loadAddon(gl);
          this.webgl = gl;
        } catch {
          /* DOM renderer fallback */
        }
      })
      .catch(() => {});
  }

  write(bytes: Uint8Array) {
    this.term.write(bytes);
  }

  /** Mark the current cursor line as agent-run (✦ in the gutter + overview ruler tick). */
  mark(command: string, actor: string) {
    const marker = this.term.registerMarker(0);
    if (!marker) return;
    const m: AgentMark = { command, actor, marker, decorated: false };
    this.marks.push(m);
    marker.onDispose(() => (this.marks = this.marks.filter((x) => x !== m)));
    if (this.opened) this.decorate(m);
  }

  private decorate(m: AgentMark) {
    if (m.decorated) return;
    const deco = this.term.registerDecoration({
      marker: m.marker,
      anchor: "left",
      x: 0,
      width: 1,
      overviewRulerOptions: { color: "#7c9cff", position: "full" },
    });
    if (!deco) return;
    m.decorated = true;
    deco.onRender((el) => {
      if (el.dataset.agent) return;
      el.dataset.agent = "1";
      el.className += " agent-mark";
      el.textContent = "✦";
      el.title = `${m.actor}: ${m.command}`;
    });
  }

  /** Scroll so the last agent-run line for `command` is visible. Returns false if not found. */
  jumpTo(command: string): boolean {
    const m = [...this.marks].reverse().find((x) => x.command === command) ?? this.marks.at(-1);
    if (!m || m.marker.line < 0) return false;
    this.term.scrollToLine(Math.max(0, m.marker.line - 2));
    this.el.animate?.([{ opacity: 0.85 }, { opacity: 1 }], { duration: 240 });
    return true;
  }

  setOptions(fontSize: number, theme: ITheme) {
    this.term.options.fontSize = fontSize;
    this.term.options.theme = theme;
  }

  dispose() {
    this.webgl?.dispose();
    this.term.dispose();
    this.el.remove();
  }
}

class TermRegistry {
  private hosts = new Map<string, TermHost>();
  opts = { fontSize: 13, theme: {} as ITheme };

  get(sessionId: string): TermHost {
    let h = this.hosts.get(sessionId);
    if (!h) {
      h = new TermHost(sessionId, this.opts);
      this.hosts.set(sessionId, h);
    }
    return h;
  }
  peek(sessionId: string) {
    return this.hosts.get(sessionId);
  }
  dispose(sessionId: string) {
    this.hosts.get(sessionId)?.dispose();
    this.hosts.delete(sessionId);
  }
  configure(fontSize: number, theme: ITheme) {
    this.opts = { fontSize, theme };
    for (const h of this.hosts.values()) h.setOptions(fontSize, theme);
  }
}

export const terminals = new TermRegistry();

if (import.meta.env.DEV && typeof window !== "undefined") (window as unknown as { __ahtTerminals: TermRegistry }).__ahtTerminals = terminals;
