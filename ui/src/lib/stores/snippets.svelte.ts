import { api } from "../api";
import type { Snippet } from "../api/types";
import { strToB64 } from "../util/b64";
import { uuid } from "../util/id";
import { fillPlaceholders, placeholders, snippetInput, SUGGESTED_SNIPPETS, type SuggestedSnippet } from "../util/snippets";
import { sessions } from "./sessions.svelte";
import { terminals } from "../terminal/host";
import { t } from "../i18n";

export function blankSnippet(): Snippet {
  return { id: uuid(), name: "", command: "", description: "", tags: [] };
}

/** Saved command snippets (vault items, synced like connections). */
class Snippets {
  list = $state<Snippet[]>([]);
  /** Snippet being edited (new or existing); `null` when the editor is closed. */
  editing = $state<{ snippet: Snippet; isNew: boolean } | null>(null);
  /** Snippet waiting for its `{{placeholder}}` values before it runs or is inserted. */
  filling = $state<{ snippet: Snippet; run: boolean } | null>(null);

  sorted = $derived([...this.list].sort((a, b) => a.name.localeCompare(b.name)));
  /** Suggestions not saved yet (matched by command). */
  suggestions = $derived(SUGGESTED_SNIPPETS.filter((s) => !this.list.some((x) => x.command.trim() === s.command.trim())));

  async load() {
    this.list = await api.listSnippets();
  }

  async save(s: Snippet) {
    const saved = await api.saveSnippet($state.snapshot(s) as Snippet);
    const i = this.list.findIndex((x) => x.id === saved.id);
    if (i >= 0) this.list[i] = saved;
    else this.list.push(saved);
    return saved;
  }

  async remove(id: string) {
    await api.deleteSnippet(id);
    this.list = this.list.filter((s) => s.id !== id);
  }

  add(s: SuggestedSnippet) {
    return this.save({ ...($state.snapshot(s) as SuggestedSnippet), id: uuid() });
  }

  edit(s: Snippet, isNew = false) {
    this.editing = { snippet: $state.snapshot(s) as Snippet, isNew };
  }

  /**
   * Type a snippet into the focused terminal (every terminal of its tab while "type in all
   * panes" is on), with Enter when `run`. Snippets with placeholders open the fill-in dialog
   * first. Throws when no terminal is focused.
   */
  async use(s: Snippet | SuggestedSnippet, run: boolean, values?: Record<string, string>) {
    const sessionId = sessions.activeSessionId;
    if (!sessionId) throw new Error("Open a terminal first.");
    if (!values && placeholders(s.command).length) {
      this.filling = { snippet: { id: "", ...($state.snapshot(s) as SuggestedSnippet) } as Snippet, run };
      return;
    }
    const command = fillPlaceholders(s.command, values ?? {});
    // "Type in all panes" on: every terminal of the tab gets it.
    const inputs = sessions.inputTargets(sessionId).map((target) => {
      const bracketedPaste = terminals.peek(target)?.term.modes.bracketedPasteMode ?? false;
      return { target, input: snippetInput(command, run, bracketedPaste) };
    });
    if (inputs.some((x) => x.input === null)) throw new Error(t("snip.multilineInsert"));
    await Promise.all(inputs.map((x) => api.sessionWrite(x.target, strToB64(x.input!))));
    sessions.focusSession(sessionId);
  }
}

export const snippets = new Snippets();
