import { errorMessage } from "../api/errors";
import { needsSudoPassword, type FileBackend } from "../util/fileBackends";

/** Switching a file system to root, asking for the sudo password when sudo needs one. */
class Sudo {
  /** The password dialog (one at a time). */
  ask = $state<{ password: string; error: string | null; busy: boolean } | null>(null);
  private answer: ((password: string | null) => void) | null = null;
  /** Requests wait for the one before: there is a single dialog and a single answer. */
  private queue: Promise<unknown> = Promise.resolve();

  /** `fs` as root, or null when the user cancelled the password dialog. */
  elevate(fs: FileBackend): Promise<FileBackend | null> {
    const run = this.queue.then(() => this.elevateNow(fs));
    this.queue = run.catch(() => {});
    return run;
  }

  private async elevateNow(fs: FileBackend): Promise<FileBackend | null> {
    if (!fs.asRoot) return null;
    try {
      return await fs.asRoot(null);
    } catch (e) {
      if (!needsSudoPassword(e)) throw e;
    }
    this.ask = { password: "", error: null, busy: false };
    try {
      for (;;) {
        const password = await new Promise<string | null>((resolve) => (this.answer = resolve));
        if (password === null) return null;
        try {
          return await fs.asRoot(password);
        } catch (e) {
          this.ask = { password: "", error: errorMessage(e), busy: false };
        }
      }
    } finally {
      this.ask = null;
      this.answer = null;
    }
  }

  submit() {
    if (!this.ask?.password) return;
    this.ask.busy = true;
    this.answer?.(this.ask.password);
  }

  cancel() {
    this.answer?.(null);
  }
}

export const sudo = new Sudo();
