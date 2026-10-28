/** Terminal text editors whose file can be opened in Handrail's editor instead. */
export const TERMINAL_EDITORS = ["nano", "pico", "vi", "vim", "nvim", "micro", "emacs", "mcedit", "joe", "hx", "sudoedit"];

export interface EditorCommand {
  /** The editor as typed (`nano`, `vim`...). */
  editor: string;
  /** The file argument as typed. */
  file: string;
  /** Run through sudo (or sudoedit): the file is opened as root. */
  sudo: boolean;
}

/**
 * Whether `line` (what the user typed at the prompt) just opens one file in a terminal editor:
 * `nano /etc/hosts`, `sudo vim +12 app.conf`, `sudoedit x`. Anything else returns null and runs
 * as usual: shell syntax (pipes, `;`, `$(...)`, redirections, globs, `~`), several files, or an
 * option that takes a value (its value would read as a second file).
 */
export function parseEditorCommand(line: string): EditorCommand | null {
  const words = shellWords(line.trim());
  if (!words?.length) return null;
  let sudo = false;
  if (words[0] === "sudo") {
    // Plain `sudo editor`: options of sudo itself (-u other, -e...) are left to the terminal.
    words.shift();
    sudo = true;
  }
  const editor = words.shift();
  if (!editor || !TERMINAL_EDITORS.includes(editor)) return null;
  if (editor === "sudoedit") {
    if (sudo) return null;
    sudo = true;
  }
  const files: string[] = [];
  let options = true;
  for (const w of words) {
    if (options && w === "--") options = false;
    else if (options && (w.startsWith("-") || w.startsWith("+"))) continue;
    else files.push(w);
  }
  if (files.length !== 1 || files[0].startsWith("~") || !files[0]) return null;
  return { editor, file: files[0], sudo };
}

/**
 * The absolute path of `file` typed in `cwd`; null when it cannot be known. `..` is left to the
 * terminal: after a symlink it leads elsewhere than the path with the folder removed.
 */
export function resolveEditorPath(file: string, cwd: string | null | undefined): string | null {
  if (file.startsWith("/")) return normalize(file);
  if (!cwd?.startsWith("/")) return null;
  return normalize(`${cwd}/${file}`);
}

function normalize(path: string): string | null {
  const out: string[] = [];
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") return null;
    out.push(part);
  }
  return out.length ? "/" + out.join("/") : null;
}

/**
 * Split a command line into words with the shell's quoting rules, or null when it uses any other
 * shell syntax (so it is not a plain "editor file" command).
 */
function shellWords(line: string): string[] | null {
  const words: string[] = [];
  let cur = "";
  let inWord = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === "'") {
      const end = line.indexOf("'", i + 1);
      if (end < 0) return null;
      cur += line.slice(i + 1, end);
      i = end;
      inWord = true;
    } else if (c === '"') {
      let j = i + 1;
      for (; j < line.length && line[j] !== '"'; j++) {
        if ("$`\\".includes(line[j])) return null;
        cur += line[j];
      }
      if (j >= line.length) return null;
      i = j;
      inWord = true;
    } else if (c === "\\") {
      if (i + 1 >= line.length) return null;
      cur += line[++i];
      inWord = true;
    } else if (c === " " || c === "\t") {
      if (inWord) words.push(cur);
      cur = "";
      inWord = false;
    } else if (";|&<>()$`*?[]{}!#=\n\r".includes(c)) {
      return null;
    } else {
      cur += c;
      inWord = true;
    }
  }
  if (inWord) words.push(cur);
  return words;
}
