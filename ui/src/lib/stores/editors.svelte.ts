import { api } from "../api";
import type { EditOutcome, EditProposal, FileStamp } from "../api/types";
import { errorMessage, isIpcError } from "../api/errors";
import { changedOnServer, sftpBackend, type FileBackend } from "../util/fileBackends";
import { uuid } from "../util/id";

/** New contents an agent proposed, waiting for the user. */
export interface Proposal {
  id: string;
  text: string;
  reason: string | null;
  actor: string;
  /** Accepted: the proposed text is in the editor and saving it answers the agent. */
  accepted: boolean;
}

/** One open file in the editor. */
export interface EditorDoc {
  id: string;
  fs: FileBackend;
  path: string;
  name: string;
  /** Text as last loaded or saved. */
  saved: string;
  text: string;
  /** What the file looked like when read; null for a file that does not exist yet. */
  stamp: FileStamp | null;
  loading: boolean;
  saving: boolean;
  error: string | null;
  /** The last save stopped because the file changed on the server meanwhile. */
  conflict: boolean;
  /** The last save was refused for lack of permission (saving as root may help). */
  denied: boolean;
  /** Only root may read the file (an agent's proposal for it): read it as root to review. */
  unreadable: boolean;
  /** The original went to `<path>.bak` during this editing session. */
  backedUp: boolean;
  proposal: Proposal | null;
  /** Bumped when `text` is replaced from outside the editor (reload, accepted proposal). */
  rev: number;
}

const sameFs = (a: FileBackend, b: FileBackend) => a.key === b.key && !!a.root === !!b.root;

/** Files open in the editor, and the agent's proposed edits to them. */
class Editors {
  docs = $state<Record<string, EditorDoc>>({});

  dirty(id: string) {
    const d = this.docs[id];
    return !!d && d.text !== d.saved;
  }

  private find(fs: FileBackend, path: string) {
    return Object.values(this.docs).find((d) => d.path === path && sameFs(d.fs, fs))?.id ?? null;
  }

  private create(fs: FileBackend, path: string): EditorDoc {
    const id = uuid();
    this.docs[id] = {
      id,
      fs,
      path,
      name: path.split("/").pop() || path,
      saved: "",
      text: "",
      stamp: null,
      loading: false,
      saving: false,
      error: null,
      conflict: false,
      denied: false,
      unreadable: false,
      backedUp: false,
      proposal: null,
      rev: 0,
    };
    return this.docs[id];
  }

  /**
   * Open `path` (or focus the doc already open); resolves with the doc id. `created`: the file
   * was just created empty, so there is no original worth a `.bak`.
   */
  async open(fs: FileBackend, path: string, created = false): Promise<string> {
    const found = this.find(fs, path);
    if (found) return found;
    const d = this.create(fs, path);
    d.backedUp = created;
    await this.load(d.id);
    return d.id;
  }

  /** (Re)read the file, dropping unsaved changes. */
  async load(id: string) {
    const d = this.docs[id];
    if (!d?.fs.readText) return;
    // An accepted proposal whose save failed is discarded by reloading: tell the agent now, or
    // a later save of unrelated edits would report it as saved.
    if (d.proposal?.accepted) this.answer(d, "rejected", "the user reloaded the file from the server instead of saving the change");
    d.loading = true;
    d.error = null;
    try {
      const f = await d.fs.readText(d.path);
      Object.assign(d, { saved: f.text, text: f.text, stamp: f.stamp, conflict: false, unreadable: false });
      d.rev++;
    } catch (e) {
      if (d.unreadable && d.proposal && isIpcError(e) && e.kind === "not_found") {
        // An agent's file in a folder only root can see, and it does not exist yet: the
        // proposal creates it (saving with no original makes a new file).
        Object.assign(d, { saved: "", text: "", stamp: null, conflict: false, unreadable: false });
        d.rev++;
      } else d.error = errorMessage(e);
    } finally {
      d.loading = false;
    }
  }

  /**
   * Save. Refuses to overwrite a file changed on the server since it was read (`conflict`)
   * unless `force`. The first save of an editing session keeps the original as `<path>.bak`.
   */
  async save(id: string, force = false): Promise<boolean> {
    const d = this.docs[id];
    if (!d?.fs.writeText) return false;
    d.saving = true;
    d.error = null;
    d.denied = false;
    try {
      let expected = d.stamp;
      if (force) expected = d.fs.readText ? ((await d.fs.readText(d.path).catch(() => null))?.stamp ?? null) : null;
      const text = d.text;
      d.stamp = await d.fs.writeText(d.path, text, expected, !d.backedUp);
      Object.assign(d, { saved: text, conflict: false, backedUp: true });
      if (d.proposal?.accepted) this.answer(d, "saved", null);
      return true;
    } catch (e) {
      if (changedOnServer(e)) d.conflict = true;
      else {
        d.error = errorMessage(e);
        d.denied = isIpcError(e) && e.kind === "denied";
      }
      return false;
    } finally {
      d.saving = false;
    }
  }

  /** Continue as `fs` (the same files as root), e.g. after "Permission denied". */
  switchFs(id: string, fs: FileBackend) {
    const d = this.docs[id];
    if (d) d.fs = fs;
  }

  close(id: string) {
    const d = this.docs[id];
    if (d?.proposal) this.answer(d, "rejected", "the user closed the editor");
    delete this.docs[id];
  }

  /** An agent proposed new contents: show them as a diff in the file's editor. */
  propose(p: EditProposal): string {
    const fs = sftpBackend(p.session_id, p.root);
    const found = this.find(fs, p.path);
    const d = found ? this.docs[found] : this.create(fs, p.path);
    // An open doc keeps its contents, unless it never loaded (e.g. its read was refused).
    if (!found || (!d.stamp && d.error && d.text === d.saved)) {
      Object.assign(d, { saved: p.original ?? "", text: p.original ?? "", stamp: p.stamp, unreadable: !!p.unreadable, error: null, denied: false });
      d.rev++;
    }
    if (d.proposal) this.answer(d, "rejected", "replaced by a newer proposal");
    d.proposal = { id: p.proposal_id, text: p.proposed, reason: p.reason, actor: p.actor_label, accepted: false };
    return d.id;
  }

  /** Accept the proposal: it replaces the editor text and is saved; the agent hears the result. */
  async accept(id: string) {
    const d = this.docs[id];
    // Unread, the file would be replaced blind: it has to be read (as root) first.
    if (!d?.proposal || d.unreadable) return;
    d.text = d.proposal.text;
    d.proposal.accepted = true;
    d.rev++;
    // On failure the proposal stays accepted: a later successful save still answers the agent.
    await this.save(id);
  }

  reject(id: string, note: string | null) {
    const d = this.docs[id];
    if (d?.proposal) this.answer(d, d.proposal.accepted ? "failed" : "rejected", note ?? (d.proposal.accepted ? d.error : null));
  }

  /** The agent stopped waiting (cancelled or timed out). */
  resolved(proposalId: string) {
    for (const d of Object.values(this.docs)) if (d.proposal?.id === proposalId) d.proposal = null;
  }

  private answer(d: EditorDoc, outcome: EditOutcome, note: string | null) {
    const p = d.proposal;
    if (!p) return;
    d.proposal = null;
    void api.editProposalDecide(p.id, outcome, note).catch(() => {});
  }
}

export const editors = new Editors();
