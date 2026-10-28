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
  /** Output received before the view opened (handed over when a reconnect adopts this host). */
  private early: Uint8Array[] = [];
  /** Where the shell's prompt ended (OSC 133;B): the command line starts there. */
  private promptEnd: { marker: IMarker; col: number } | null = null;
  /** Typed input not yet answered by output, and output not yet drawn: the screen may lag. */
  private lastInput = 0;
  private lastOutput = 0;
  private drawing = 0;

  constructor(
    /** Changes when a reconnect moves this terminal to the new session. */
    public sessionId: string,
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
    this.term.parser.registerOscHandler(133, (data) => {
      if (data.startsWith("B")) {
        this.promptEnd?.marker.dispose();
        const marker = this.term.registerMarker(0);
        this.promptEnd = marker ? { marker, col: this.term.buffer.active.cursorX } : null;
      } else if (data.startsWith("A") || data.startsWith("C")) {
        this.promptEnd?.marker.dispose();
        this.promptEnd = null;
      }
      return false;
    });
    this.term.onData((d) => {
      // Enter on a command the terminal hook takes over (e.g. opening a file in the editor).
      if (d === "\r" && terminals.onEnter?.(this.sessionId, () => this.commandLine())) return;
      this.lastInput = performance.now();
      terminals.input(this.sessionId, strToB64(d));
    });
    this.term.onBinary((d) => {
      const bytes = Uint8Array.from(d, (c) => c.charCodeAt(0) & 0xff);
      let s = "";
      bytes.forEach((b) => (s += String.fromCharCode(b)));
      terminals.input(this.sessionId, btoa(s));
    });
  }

  /** Open into the (already attached) host element. Tries WebGL, falls back to the DOM renderer. */
  open() {
    if (this.opened) return;
    this.term.open(this.el);
    this.opened = true;
    this.early = [];
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
    if (!this.opened) this.early.push(bytes);
    this.lastOutput = performance.now();
    this.drawing++;
    this.term.write(bytes, () => this.drawing--);
  }

  /**
   * What is typed at the shell prompt (after OSC 133;B, wrapped lines joined), or null without
   * shell integration or in a full-screen program.
   */
  commandLine(): string | null {
    const p = this.promptEnd;
    const buf = this.term.buffer.active;
    if (!p || p.marker.isDisposed || p.marker.line < 0 || buf.type !== "normal") return null;
    // Keys typed since the last echo, or output still being parsed: the screen is not the line
    // the shell has, so it cannot be trusted.
    if (this.drawing > 0 || this.lastInput > this.lastOutput) return null;
    let last = buf.baseY + buf.cursorY;
    if (last < p.marker.line) return null;
    // The cursor may sit on an earlier row of a wrapped command: read to its end.
    while (buf.getLine(last + 1)?.isWrapped) last++;
    let text = "";
    for (let y = p.marker.line; y <= last; y++) {
      const line = buf.getLine(y);
      if (!line) return null;
      if (y > p.marker.line && !line.isWrapped) return null;
      text += line.translateToString(y === last, y === p.marker.line ? p.col : 0);
    }
    return text;
  }

  /** Output that arrived before the view opened. */
  takeEarly(): Uint8Array[] {
    const e = this.early;
    this.early = [];
    return e;
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

/** Sends what the user typed in a terminal (base64). */
export type InputRouter = (sessionId: string, dataB64: string) => void;

const writeTo: InputRouter = (sessionId, dataB64) => void api.sessionWrite(sessionId, dataB64).catch(() => {});

class TermRegistry {
  private hosts = new Map<string, TermHost>();
  opts = { fontSize: 13, theme: {} as ITheme };
  /** Where typing goes: its own session, unless the sessions store mirrors it (typing in all panes). */
  input: InputRouter = writeTo;
  /** Write typed input to one session. */
  readonly write: InputRouter = writeTo;
  /**
   * Called on Enter with the typed command line (read on demand); returning true keeps the
   * Enter from the shell.
   */
  onEnter: ((sessionId: string, line: () => string | null) => boolean) | null = null;

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
  /**
   * Move the terminal of `from` (screen and scrollback) to session `to`. Output of `to` that
   * arrived first (it can beat the open call's reply) is replayed into it.
   */
  rekey(from: string, to: string) {
    const h = this.hosts.get(from);
    if (!h) return;
    const early = this.hosts.get(to);
    this.hosts.delete(from);
    h.sessionId = to;
    this.hosts.set(to, h);
    if (early && early !== h) {
      for (const b of early.takeEarly()) h.write(b);
      early.dispose();
    }
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

