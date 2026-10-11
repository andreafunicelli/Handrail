// CodeMirror setup for the file editor. Loaded on demand (the editor is a separate chunk).
import { basicSetup } from "codemirror";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { indentWithTab } from "@codemirror/commands";
import { HighlightStyle, StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { unifiedMergeView } from "@codemirror/merge";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";
import { sql } from "@codemirror/lang-sql";
import { xml } from "@codemirror/lang-xml";
import { yaml } from "@codemirror/lang-yaml";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";
import { nginx } from "@codemirror/legacy-modes/mode/nginx";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";

/** Syntax support for a file, from its path. */
export function languageFor(path: string): Extension | null {
  const name = (path.split("/").pop() ?? path).toLowerCase();
  const ext = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1) : "";
  if (name === "dockerfile" || name.startsWith("dockerfile.") || ext === "dockerfile") return StreamLanguage.define(dockerFile);
  if (path.includes("/nginx/") || ext === "nginx") return StreamLanguage.define(nginx);
  if ([".bashrc", ".bash_profile", ".profile", ".zshrc", ".bash_aliases"].includes(name)) return StreamLanguage.define(shell);
  switch (ext) {
    case "json":
      return json();
    case "yml":
    case "yaml":
      return yaml();
    case "js":
    case "mjs":
    case "cjs":
      return javascript();
    case "ts":
      return javascript({ typescript: true });
    case "py":
      return python();
    case "md":
      return markdown();
    case "xml":
    case "svg":
      return xml();
    case "css":
      return css();
    case "html":
    case "htm":
      return html();
    case "sql":
      return sql();
    case "sh":
    case "bash":
    case "zsh":
      return StreamLanguage.define(shell);
    case "toml":
      return StreamLanguage.define(toml);
    case "ini":
    case "conf":
    case "cfg":
    case "env":
    case "properties":
      return StreamLanguage.define(properties);
  }
  return name === ".env" || name.startsWith(".env.") ? StreamLanguage.define(properties) : null;
}

// Colors come from CSS variables, so the editor follows the light and dark themes.
const theme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "var(--bg)", color: "var(--text)", fontSize: "13px" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.55" },
  ".cm-content": { caretColor: "var(--accent)" },
  ".cm-gutters": { backgroundColor: "var(--panel)", color: "var(--faint)", border: "none", borderRight: "1px solid var(--border)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--hover)", color: "var(--muted)" },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--hover) 55%, transparent)" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)", borderLeftWidth: "2px" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": { backgroundColor: "var(--accent-soft) !important" },
  ".cm-selectionMatch": { backgroundColor: "var(--accent-soft)" },
  ".cm-searchMatch": { backgroundColor: "var(--warn-soft)", outline: "1px solid var(--warn)" },
  ".cm-panels": { backgroundColor: "var(--raised)", color: "var(--text)", borderColor: "var(--border)" },
  ".cm-panels input, .cm-panels button": { color: "var(--text)" },
  ".cm-tooltip": { backgroundColor: "var(--raised)", color: "var(--text)", border: "1px solid var(--border-strong)" },
  ".cm-foldPlaceholder": { backgroundColor: "var(--hover)", border: "none", color: "var(--muted)" },
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
  { tag: t.strong, fontWeight: "600" },
  { tag: t.emphasis, fontStyle: "italic" },
]);

/** Windows line endings stay Windows line endings. */
const lineBreaks = (text: string): Extension => (text.includes("\r\n") ? EditorState.lineSeparator.of("\r\n") : []);

export interface Selection {
  text: string;
  fromLine: number;
  toLine: number;
}

export interface EditorHandle {
  /** Replace the whole text (reload, accepted proposal). */
  setDoc(text: string): void;
  /** The selected text, or null when nothing is selected. */
  selection(): Selection | null;
  focus(): void;
  destroy(): void;
}

export function createEditor(
  parent: HTMLElement,
  o: { doc: string; path: string; onChange: (text: string) => void; onSave: () => void },
): EditorHandle {
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: o.doc,
      extensions: [
        keymap.of([{ key: "Mod-s", preventDefault: true, run: () => (o.onSave(), true) }, indentWithTab]),
        basicSetup,
        theme,
        syntaxHighlighting(highlight),
        languageFor(o.path) ?? [],
        lineBreaks(o.doc),
        // sliceDoc keeps the file's own line separator (toString would not).
        EditorView.updateListener.of((u) => u.docChanged && o.onChange(u.state.sliceDoc())),
      ],
    }),
  });
  return {
    setDoc(text) {
      if (view.state.sliceDoc() !== text) view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
    },
    selection() {
      const { from, to } = view.state.selection.main;
      if (from === to) return null;
      const doc = view.state.doc;
      return { text: view.state.sliceDoc(from, to), fromLine: doc.lineAt(from).number, toLine: doc.lineAt(to).number };
    },
    focus: () => view.focus(),
    destroy: () => view.destroy(),
  };
}

/** Read-only unified diff: `proposed` with the removed lines of `original` shown inline. */
export function createDiff(parent: HTMLElement, o: { original: string; proposed: string; path: string }): { destroy(): void } {
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: o.proposed,
      extensions: [
        lineNumbers(),
        EditorState.readOnly.of(true),
        EditorView.editable.of(false),
        theme,
        syntaxHighlighting(highlight),
        languageFor(o.path) ?? [],
        lineBreaks(o.proposed),
        unifiedMergeView({ original: o.original, mergeControls: false, gutter: true, highlightChanges: false, syntaxHighlightDeletions: true }),
      ],
    }),
  });
  return { destroy: () => view.destroy() };
}
