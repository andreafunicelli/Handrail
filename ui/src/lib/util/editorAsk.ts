/** Longest file text sent along with a question (the agent can `cat` the rest). */
export const ASK_MAX_CHARS = 60_000;

/**
 * The agent message for a question asked from the editor: where the file is, what the user is
 * looking at (selection, or the editor text including unsaved changes) and how to change it.
 */
export function editorQuestion(
  o: { path: string; root: boolean; text: string; dirty: boolean },
  question: string,
  selection: { text: string; fromLine: number; toLine: number } | null,
): string {
  const where = `I have \`${o.path}\` open in Handrail's editor on this server${o.root ? " (as root, through sudo)" : ""}${o.dirty ? ", with unsaved changes" : ""}.`;
  let body: string;
  if (selection) {
    body = `Selected lines ${selection.fromLine}–${selection.toLine}:\n\n${fence(selection.text)}`;
  } else {
    const cut = o.text.length > ASK_MAX_CHARS;
    body = `Editor contents${cut ? ` (first ${ASK_MAX_CHARS} characters; read the rest with cat)` : ""}:\n\n${fence(cut ? o.text.slice(0, ASK_MAX_CHARS) : o.text)}`;
  }
  return `${where}\n\n${body}\n\nMy request: ${question.trim()}\n\nIf this needs a change to the file, call propose_file_edit with the complete new contents, based on the editor contents (not an older copy).`;
}

/** A fenced block that the text cannot close early. */
function fence(text: string): string {
  let ticks = "```";
  while (text.includes(ticks)) ticks += "`";
  return `${ticks}\n${text.replace(/\n$/, "")}\n${ticks}`;
}
