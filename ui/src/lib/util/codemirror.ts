// CodeMirror setup for the file editor. Loaded on demand (the editor is a separate chunk).
import { basicSetup } from "codemirror";
import { Compartment, EditorSelection, EditorState, type Extension } from "@codemirror/state";
import { EditorView, highlightTrailingWhitespace, highlightWhitespace, keymap, lineNumbers } from "@codemirror/view";
import { indentWithTab, selectLine, toggleComment } from "@codemirror/commands";
import { HighlightStyle, indentUnit, StreamLanguage, syntaxHighlighting, type StreamParser } from "@codemirror/language";
import { gotoLine, openSearchPanel, selectSelectionMatches } from "@codemirror/search";
import { tags as t } from "@lezer/highlight";
import { unifiedMergeView } from "@codemirror/merge";
import { showMinimap } from "@replit/codemirror-minimap";
import { indentationMarkers } from "@replit/codemirror-indentation-markers";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { php } from "@codemirror/lang-php";
import { python } from "@codemirror/lang-python";
import { sql } from "@codemirror/lang-sql";
import { xml } from "@codemirror/lang-xml";
import { yaml } from "@codemirror/lang-yaml";
import { c, cpp, csharp, java, kotlin, scala } from "@codemirror/legacy-modes/mode/clike";
import { cmake } from "@codemirror/legacy-modes/mode/cmake";
import { less, sCSS } from "@codemirror/legacy-modes/mode/css";
import { diff } from "@codemirror/legacy-modes/mode/diff";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";
import { go } from "@codemirror/legacy-modes/mode/go";
import { groovy } from "@codemirror/legacy-modes/mode/groovy";
import { jinja2 } from "@codemirror/legacy-modes/mode/jinja2";
import { lua } from "@codemirror/legacy-modes/mode/lua";
import { nginx } from "@codemirror/legacy-modes/mode/nginx";
import { perl } from "@codemirror/legacy-modes/mode/perl";
import { powerShell } from "@codemirror/legacy-modes/mode/powershell";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { protobuf } from "@codemirror/legacy-modes/mode/protobuf";
import { r } from "@codemirror/legacy-modes/mode/r";
import { ruby } from "@codemirror/legacy-modes/mode/ruby";
import { rust } from "@codemirror/legacy-modes/mode/rust";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { swift } from "@codemirror/legacy-modes/mode/swift";
import { toml } from "@codemirror/legacy-modes/mode/toml";

const legacy = (p: StreamParser<unknown>) => () => StreamLanguage.define(p);

interface Lang {
  id: string;
  name: string;
  load: () => Extension;
  /** File extensions, lower case, without the dot. */
  ext?: string[];
  /** Whole file names, lower case. */
  names?: string[];
  /** Interpreters named on a `#!` line. */
  shebang?: RegExp;
}

/** Every language the editor highlights, in menu order. */
export const LANGUAGES: Lang[] = [
  { id: "text", name: "Plain text", load: () => [] },
  { id: "c", name: "C", load: legacy(c), ext: ["c", "h"] },
  { id: "cpp", name: "C++", load: legacy(cpp), ext: ["cc", "cpp", "cxx", "hpp", "hh", "hxx", "ino"] },
  { id: "csharp", name: "C#", load: legacy(csharp), ext: ["cs"] },
  { id: "cmake", name: "CMake", load: legacy(cmake), ext: ["cmake"], names: ["cmakelists.txt"] },
  { id: "css", name: "CSS", load: () => css(), ext: ["css"] },
  { id: "diff", name: "Diff", load: legacy(diff), ext: ["diff", "patch"] },
  { id: "dockerfile", name: "Dockerfile", load: legacy(dockerFile), ext: ["dockerfile"], names: ["dockerfile", "containerfile"] },
  { id: "go", name: "Go", load: legacy(go), ext: ["go"] },
  { id: "groovy", name: "Groovy", load: legacy(groovy), ext: ["groovy", "gradle"], names: ["jenkinsfile"] },
  { id: "html", name: "HTML", load: () => html(), ext: ["html", "htm", "xhtml"] },
  { id: "ini", name: "INI / config", load: legacy(properties), ext: ["ini", "conf", "cfg", "cnf", "env", "properties", "service", "timer", "socket", "mount", "target", "desktop"] },
  { id: "java", name: "Java", load: legacy(java), ext: ["java"] },
  { id: "javascript", name: "JavaScript", load: () => javascript({ jsx: true }), ext: ["js", "mjs", "cjs", "jsx"], shebang: /^(node|deno|bun)$/ },
  { id: "jinja", name: "Jinja", load: legacy(jinja2), ext: ["j2", "jinja", "jinja2"] },
  { id: "json", name: "JSON", load: () => json(), ext: ["json", "jsonc", "json5", "map", "webmanifest"], names: [".babelrc", ".eslintrc", "composer.lock"] },
  { id: "kotlin", name: "Kotlin", load: legacy(kotlin), ext: ["kt", "kts"] },
  { id: "less", name: "Less", load: legacy(less), ext: ["less"] },
  { id: "lua", name: "Lua", load: legacy(lua), ext: ["lua"], shebang: /^lua/ },
  { id: "markdown", name: "Markdown", load: () => markdown(), ext: ["md", "markdown", "mdx"] },
  { id: "nginx", name: "Nginx", load: legacy(nginx), ext: ["nginx"], names: ["nginx.conf"] },
  { id: "perl", name: "Perl", load: legacy(perl), ext: ["pl", "pm"], shebang: /^perl/ },
  { id: "php", name: "PHP", load: () => php(), ext: ["php", "phtml", "php3", "php4", "php5", "phps"], shebang: /^php/ },
  { id: "powershell", name: "PowerShell", load: legacy(powerShell), ext: ["ps1", "psm1", "psd1"], shebang: /^pwsh$/ },
  { id: "protobuf", name: "Protocol Buffers", load: legacy(protobuf), ext: ["proto"] },
  { id: "python", name: "Python", load: () => python(), ext: ["py", "pyw", "pyi"], shebang: /^python/ },
  { id: "r", name: "R", load: legacy(r), ext: ["r"], shebang: /^Rscript$/ },
  { id: "ruby", name: "Ruby", load: legacy(ruby), ext: ["rb", "rake", "gemspec"], names: ["gemfile", "rakefile", "vagrantfile"], shebang: /^ruby/ },
  { id: "rust", name: "Rust", load: legacy(rust), ext: ["rs"] },
  { id: "scala", name: "Scala", load: legacy(scala), ext: ["scala", "sc"] },
  { id: "scss", name: "SCSS", load: legacy(sCSS), ext: ["scss"] },
  {
    id: "shell",
    name: "Shell",
    load: legacy(shell),
    ext: ["sh", "bash", "zsh", "ksh"],
    names: [".bashrc", ".bash_profile", ".bash_login", ".bash_logout", ".bash_aliases", ".profile", ".zshrc", ".zprofile", ".zshenv", "profile", "bashrc"],
    shebang: /^(ba|z|k|da|a)?sh$/,
  },
  { id: "sql", name: "SQL", load: () => sql(), ext: ["sql"] },
  { id: "swift", name: "Swift", load: legacy(swift), ext: ["swift"] },
  { id: "toml", name: "TOML", load: legacy(toml), ext: ["toml"], names: ["cargo.lock", "pipfile"] },
  // Not JSX: `<Foo>value` is a type assertion in .ts files.
  { id: "typescript", name: "TypeScript", load: () => javascript({ typescript: true }), ext: ["ts", "mts", "cts"] },
  { id: "tsx", name: "TypeScript JSX", load: () => javascript({ typescript: true, jsx: true }), ext: ["tsx"] },
  { id: "xml", name: "XML", load: () => xml(), ext: ["xml", "svg", "xsd", "xsl", "xslt", "plist", "csproj", "xaml", "pom"] },
  { id: "yaml", name: "YAML", load: () => yaml(), ext: ["yml", "yaml"] },
];

const byId = (id: string) => LANGUAGES.find((l) => l.id === id) ?? LANGUAGES[0];
export const languageName = (id: string) => byId(id).name;

/** The language of a file, from its name, its folder, or a `#!` line. */
export function detectLanguage(path: string, text = ""): string {
  const name = (path.split("/").pop() ?? path).toLowerCase();
  const ext = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1) : "";
  const hit = LANGUAGES.find((l) => l.names?.includes(name)) ?? LANGUAGES.find((l) => ext && l.ext?.includes(ext));
  if (hit && !(hit.id === "ini" && path.includes("/nginx/"))) return hit.id;
  if (path.includes("/nginx/") && (ext === "conf" || !ext)) return "nginx";
  if (name.startsWith("dockerfile.")) return "dockerfile";
  if (name === ".env" || name.startsWith(".env.")) return "ini";
  if (text.startsWith("#!")) {
    // "#!/bin/bash", "#!/usr/bin/env -S python3 -u": the interpreter's own name.
    const words = text.slice(2, text.indexOf("\n") >>> 0).trim().split(/\s+/);
    let prog = words[0]?.split("/").pop() ?? "";
    if (prog === "env") prog = words.slice(1).find((w) => !w.startsWith("-") && !w.includes("=")) ?? "";
    const l = LANGUAGES.find((x) => x.shebang?.test(prog));
    if (l) return l.id;
  }
  if (path.startsWith("/etc/") && !ext && /^\s*[#;[]/m.test(text.slice(0, 2000))) return "ini";
  return "text";
}

/** Syntax support for a file, from its path. */
export function languageFor(path: string, text = ""): Extension {
  return byId(detectLanguage(path, text)).load();
}

export interface Indent {
  tabs: boolean;
  size: number;
}

/** How the file is indented: tabs, or the most common step between space-indented lines. */
export function detectIndent(text: string, fallback: Indent = { tabs: false, size: 4 }): Indent {
  let tabs = 0;
  let spaces = 0;
  const steps = new Map<number, number>();
  let prev = 0;
  const lines = text.split("\n", 2000);
  for (const line of lines) {
    if (!line.trim()) continue;
    const ws = /^[ \t]*/.exec(line)![0];
    if (ws.startsWith("\t")) tabs++;
    else if (ws.length > 0) spaces++;
    if (!ws.includes("\t")) {
      const d = Math.abs(ws.length - prev);
      if (d > 0 && d <= 8) steps.set(d, (steps.get(d) ?? 0) + 1);
      prev = ws.length;
    }
  }
  if (tabs > spaces) return { tabs: true, size: fallback.size };
  if (!spaces) return fallback;
  let best = fallback.size;
  let count = 0;
  for (const [d, n] of steps) if (n > count || (n === count && d < best)) [best, count] = [d, n];
  return { tabs: false, size: best };
}

export type Eol = "LF" | "CRLF";
export const detectEol = (text: string): Eol => (text.includes("\r\n") ? "CRLF" : "LF");

// Colors come from CSS variables, so the editor follows the light and dark themes.
const theme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "var(--bg)", color: "var(--text)" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.55" },
  // Its own stacking layer: the indentation guides (z-index -1) draw above the editor background.
  ".cm-content": { caretColor: "var(--accent)", isolation: "isolate" },
  ".cm-gutters": { backgroundColor: "var(--panel)", color: "var(--faint)", border: "none", borderRight: "1px solid var(--border)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--hover)", color: "var(--muted)" },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--hover) 55%, transparent)" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)", borderLeftWidth: "2px" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": { backgroundColor: "var(--accent-soft) !important" },
  ".cm-selectionMatch": { backgroundColor: "var(--accent-soft)" },
  ".cm-searchMatch": { backgroundColor: "var(--warn-soft)", outline: "1px solid var(--warn)" },
  ".cm-searchMatch-selected": { backgroundColor: "var(--accent-soft)" },
  ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": { backgroundColor: "var(--accent-soft)", outline: "1px solid var(--border-strong)" },
  ".cm-panels": { backgroundColor: "var(--raised)", color: "var(--text)", borderColor: "var(--border)" },
  ".cm-panels input, .cm-panels button": { color: "var(--text)" },
  ".cm-panel.cm-search": { fontFamily: "inherit", padding: "6px 8px" },
  ".cm-panel.cm-search input, .cm-panel.cm-goto-line input": {
    backgroundColor: "var(--bg)",
    border: "1px solid var(--border-strong)",
    borderRadius: "4px",
    padding: "2px 6px",
  },
  ".cm-panel.cm-search button": { backgroundImage: "none", backgroundColor: "var(--hover)", border: "1px solid var(--border)", borderRadius: "4px" },
  ".cm-tooltip": { backgroundColor: "var(--raised)", color: "var(--text)", border: "1px solid var(--border-strong)" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--accent-soft)", color: "var(--text)" },
  ".cm-foldPlaceholder": { backgroundColor: "var(--hover)", border: "none", color: "var(--muted)" },
  ".cm-highlightSpace": { backgroundImage: "radial-gradient(circle at 50% 55%, var(--faint) 12%, transparent 15%)" },
  ".cm-highlightTab": { opacity: "0.6" },
  ".cm-trailingSpace": { backgroundColor: "var(--danger-soft)" },
  ".cm-minimap-gutter": { backgroundColor: "var(--bg)", borderLeft: "1px solid var(--border)" },
  ".cm-minimap-overlay": { backgroundColor: "var(--text) !important", opacity: "0.08 !important" },
  ".cm-changedLine, .cm-insertedLine": { backgroundColor: "var(--diff-add) !important" },
  ".cm-changedText": { background: "none" },
  ".cm-deletedChunk": { backgroundColor: "var(--diff-del) !important" },
  ".cm-deletedChunk .cm-deletedText, .cm-deletedLine del": { textDecoration: "none" },
  ".cm-changeGutter": { width: "3px", paddingLeft: "0" },
  ".cm-changedLineGutter": { backgroundColor: "var(--ok)" },
  ".cm-deletedLineGutter": { backgroundColor: "var(--danger)" },
});

const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier, t.controlKeyword, t.definitionKeyword], color: "var(--syn-keyword)" },
  { tag: [t.string, t.special(t.string), t.regexp, t.url], color: "var(--syn-string)" },
  { tag: [t.number, t.bool, t.null, t.atom, t.unit], color: "var(--syn-number)" },
  { tag: [t.comment, t.lineComment, t.blockComment], color: "var(--syn-comment)", fontStyle: "italic" },
  { tag: [t.propertyName, t.attributeName, t.variableName, t.definition(t.variableName), t.labelName], color: "var(--syn-name)" },
  { tag: [t.typeName, t.className, t.tagName, t.namespace], color: "var(--syn-type)" },
  { tag: [t.meta, t.processingInstruction, t.heading, t.annotation], color: "var(--syn-meta)" },
  { tag: [t.inserted], color: "var(--ok)" },
  { tag: [t.deleted], color: "var(--danger)" },
  { tag: t.strong, fontWeight: "600" },
  { tag: t.emphasis, fontStyle: "italic" },
]);

/** View options shared by every editor (the user's preferences). */
export interface ViewPrefs {
  wrap: boolean;
  whitespace: boolean;
  minimap: boolean;
  guides: boolean;
  fontSize: number;
}

/** What the status bar shows. */
export interface EditorStatus {
  line: number;
  col: number;
  /** Selected characters, over all selections. */
  selected: number;
  cursors: number;
  lines: number;
}

export interface Selection {
  text: string;
  fromLine: number;
  toLine: number;
}

export type EditorCommand = "find" | "replace" | "gotoLine" | "comment" | "selectMatches" | "selectLine" | "selectAll" | "cut" | "copy" | "paste";

export interface EditorHandle {
  /** Replace the whole text (reload, accepted proposal). */
  setDoc(text: string): void;
  /** The selected text, or null when nothing is selected. */
  selection(): Selection | null;
  setPrefs(p: ViewPrefs): void;
  setLanguage(id: string): void;
  setIndent(i: Indent): void;
  /** Convert every line ending; reported through `onChange` like an edit. */
  setEol(eol: Eol): void;
  run(cmd: EditorCommand): Promise<void>;
  focus(): void;
  destroy(): void;
}

const GUIDE = "color-mix(in srgb, var(--faint) 50%, transparent)";
const GUIDE_ACTIVE = "color-mix(in srgb, var(--faint) 100%, transparent)";
const minimapConf = { create: () => ({ dom: document.createElement("div") }), displayText: "blocks" as const, showOverlay: "always" as const };

function viewExtensions(p: ViewPrefs): Extension {
  return [
    EditorView.theme({ "&": { fontSize: `${p.fontSize}px` } }),
    p.wrap ? EditorView.lineWrapping : [],
    p.whitespace ? [highlightWhitespace(), highlightTrailingWhitespace()] : [],
    p.guides ? indentationMarkers({ thickness: 1, colors: { light: GUIDE, dark: GUIDE, activeLight: GUIDE_ACTIVE, activeDark: GUIDE_ACTIVE } }) : [],
    showMinimap.of(p.minimap ? minimapConf : null),
  ];
}

const indentExtensions = (i: Indent): Extension => [EditorState.tabSize.of(i.size), indentUnit.of(i.tabs ? "\t" : " ".repeat(i.size))];
/** Windows line endings stay Windows line endings. */
const eolExtension = (eol: Eol): Extension => (eol === "CRLF" ? EditorState.lineSeparator.of("\r\n") : []);

function status(s: EditorState): EditorStatus {
  const main = s.selection.main;
  const line = s.doc.lineAt(main.head);
  return {
    line: line.number,
    col: main.head - line.from + 1,
    selected: s.selection.ranges.reduce((n, r) => n + r.to - r.from, 0),
    cursors: s.selection.ranges.length,
    lines: s.doc.lines,
  };
}

export function createEditor(
  parent: HTMLElement,
  o: {
    doc: string;
    path: string;
    prefs: ViewPrefs;
    language: string;
    indent: Indent;
    onChange: (text: string) => void;
    onSave: () => void;
    onStatus: (s: EditorStatus) => void;
    /** Keyboard view toggles: word wrap (Alt+Z) and zoom (Ctrl +, -, 0). */
    onView: (action: "wrap" | "zoomIn" | "zoomOut" | "zoomReset") => void;
    onContextMenu: (ev: MouseEvent) => void;
  },
): EditorHandle {
  const lang = new Compartment();
  const view_ = new Compartment();
  const indent = new Compartment();
  const eol = new Compartment();
  const fire = (a: Parameters<typeof o.onView>[0]) => () => (o.onView(a), true);
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: o.doc,
      extensions: [
        keymap.of([
          { key: "Mod-s", preventDefault: true, run: () => (o.onSave(), true) },
          { key: "Mod-g", preventDefault: true, run: gotoLine },
          { key: "Mod-h", preventDefault: true, run: openSearchPanel },
          { key: "Alt-z", preventDefault: true, run: fire("wrap") },
          { key: "Mod-=", preventDefault: true, run: fire("zoomIn") },
          { key: "Mod-+", preventDefault: true, run: fire("zoomIn") },
          { key: "Mod--", preventDefault: true, run: fire("zoomOut") },
          { key: "Mod-0", preventDefault: true, run: fire("zoomReset") },
          indentWithTab,
        ]),
        basicSetup,
        theme,
        syntaxHighlighting(highlight),
        lang.of(byId(o.language).load()),
        view_.of(viewExtensions(o.prefs)),
        indent.of(indentExtensions(o.indent)),
        eol.of(eolExtension(detectEol(o.doc))),
        // sliceDoc keeps the file's own line separator (toString would not).
        EditorView.updateListener.of((u) => {
          if (u.docChanged) o.onChange(u.state.sliceDoc());
          if (u.docChanged || u.selectionSet) o.onStatus(status(u.state));
        }),
        EditorView.domEventHandlers({
          contextmenu: (ev) => (ev.preventDefault(), o.onContextMenu(ev), true),
          wheel: (ev) => {
            if (!(ev.ctrlKey || ev.metaKey)) return false;
            ev.preventDefault();
            o.onView(ev.deltaY < 0 ? "zoomIn" : "zoomOut");
            return true;
          },
        }),
      ],
    }),
  });
  o.onStatus(status(view.state));

  const replaceSelection = (text: string) => view.dispatch(view.state.replaceSelection(text), { scrollIntoView: true, userEvent: "input.paste" });
  const selectedText = () =>
    view.state.selection.ranges
      .filter((r) => !r.empty)
      .map((r) => view.state.sliceDoc(r.from, r.to))
      .join(view.state.lineBreak);

  return {
    setDoc(text) {
      if (view.state.sliceDoc() === text) return;
      view.dispatch({ effects: eol.reconfigure(eolExtension(detectEol(text))) });
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
    },
    selection() {
      const { from, to } = view.state.selection.main;
      if (from === to) return null;
      const doc = view.state.doc;
      return { text: view.state.sliceDoc(from, to), fromLine: doc.lineAt(from).number, toLine: doc.lineAt(to).number };
    },
    setPrefs: (p) => view.dispatch({ effects: view_.reconfigure(viewExtensions(p)) }),
    setLanguage: (id) => view.dispatch({ effects: lang.reconfigure(byId(id).load()) }),
    setIndent: (i) => view.dispatch({ effects: indent.reconfigure(indentExtensions(i)) }),
    setEol(next) {
      view.dispatch({ effects: eol.reconfigure(eolExtension(next)) });
      o.onChange(view.state.sliceDoc());
    },
    async run(cmd) {
      view.focus();
      switch (cmd) {
        case "find":
        case "replace":
          openSearchPanel(view);
          if (cmd === "replace") requestAnimationFrame(() => view.dom.querySelector<HTMLInputElement>(".cm-search input[name=replace]")?.focus());
          return;
        case "gotoLine":
          gotoLine(view);
          return;
        case "comment":
          toggleComment(view);
          return;
        case "selectMatches":
          selectSelectionMatches(view);
          return;
        case "selectLine":
          selectLine(view);
          return;
        case "selectAll":
          view.dispatch({ selection: EditorSelection.single(0, view.state.doc.length) });
          return;
        case "copy":
        case "cut": {
          const text = selectedText();
          if (!text) return;
          await navigator.clipboard.writeText(text);
          if (cmd === "cut") view.dispatch(view.state.replaceSelection(""), { userEvent: "delete.cut" });
          return;
        }
        case "paste":
          replaceSelection(await navigator.clipboard.readText());
          return;
      }
    },
    focus: () => view.focus(),
    destroy: () => view.destroy(),
  };
}

/** Read-only unified diff: `proposed` with the removed lines of `original` shown inline. */
export function createDiff(parent: HTMLElement, o: { original: string; proposed: string; path: string; fontSize?: number }): { destroy(): void } {
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: o.proposed,
      extensions: [
        lineNumbers(),
        EditorState.readOnly.of(true),
        EditorView.editable.of(false),
        theme,
        EditorView.theme({ "&": { fontSize: `${o.fontSize ?? 13}px` } }),
        syntaxHighlighting(highlight),
        languageFor(o.path, o.original),
        eolExtension(detectEol(o.proposed)),
        unifiedMergeView({ original: o.original, mergeControls: false, gutter: true, highlightChanges: false, syntaxHighlightDeletions: true }),
      ],
    }),
  });
  return { destroy: () => view.destroy() };
}
