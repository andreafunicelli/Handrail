import { terminals } from "../terminal/host";
import { strToB64 } from "../util/b64";
import { parseEditorCommand, resolveEditorPath } from "../util/editorCommand";
import { sftpBackend } from "../util/fileBackends";
import { editors } from "./editors.svelte";
import { sessions } from "./sessions.svelte";
import { settings } from "./settings.svelte";
import { sudo } from "./sudo.svelte";
import { toasts } from "./toasts.svelte";

/** `nano file` typed in an SSH terminal: the file, ready to open in Handrail's editor. */
export interface EditorOffer {
  sessionId: string;
  editor: string;
  path: string;
  /** Opened as root: through sudo, or in a root shell the login user switched to. */
  root: boolean;
}

/** Offers Handrail's editor when a terminal editor is about to open a file on the server. */
class EditorOffers {
  /** The question on screen ("ask" setting). */
  offer = $state<EditorOffer | null>(null);

  /**
   * Enter pressed in `sessionId`: returns true (and keeps the Enter) when the typed line opens a
   * file in a terminal editor and Handrail can open the same file. Only at the prompt of the
   * session's own server, with shell integration telling where the prompt ends and which folder
   * the shell is in; anything uncertain goes to the shell as typed.
   */
  enter(sessionId: string, line: () => string | null): boolean {
    if (settings.terminalEditor === "terminal" || this.offer) return false;
    const info = sessions.infos[sessionId];
    const home = sessions.homes[sessionId];
    if (!info?.connection_id || info.integration !== "full" || info.state !== "idle" || !home) return false;
    // A shell on another machine (ssh from this one): the file is not where SFTP looks.
    if (info.host !== home.host) return false;
    if (sessions.inputTargets(sessionId).length > 1) return false;
    const cmd = parseEditorCommand(line() ?? "");
    const path = cmd && resolveEditorPath(cmd.file, info.cwd);
    if (!cmd || !path) return false;
    const offer = { sessionId, editor: cmd.editor, path, root: cmd.sudo || (info.user === "root" && home.user !== "root") };
    if (settings.terminalEditor === "editor") void this.openInEditor(offer);
    else this.offer = offer;
    return true;
  }

  /** Open the file in the editor and clear the typed command (it never runs). */
  async openInEditor(o: EditorOffer, remember = false) {
    this.offer = null;
    if (remember) settings.terminalEditor = "editor";
    try {
      const fs = o.root ? await sudo.elevate(sftpBackend(o.sessionId)) : sftpBackend(o.sessionId);
      // Password dialog cancelled: the command stays typed, to run or edit.
      if (!fs) return;
      // End of line, then erase it (bash, zsh and fish alike).
      terminals.write(o.sessionId, strToB64("\x05\x15"));
      sessions.openEditor(await editors.open(fs, o.path));
    } catch (e) {
      toasts.error(e);
    }
  }

  /** Run the command in the terminal after all. */
  runInTerminal(o: EditorOffer, remember = false) {
    this.offer = null;
    if (remember) settings.terminalEditor = "terminal";
    terminals.write(o.sessionId, strToB64("\r"));
  }

  /** Neither: the command stays typed at the prompt. */
  dismiss() {
    this.offer = null;
  }
}

export const editorOffers = new EditorOffers();
terminals.onEnter = (sessionId, line) => editorOffers.enter(sessionId, line);
