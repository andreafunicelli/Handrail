import { api } from "../api";
import type { S3Account } from "../api/types";
import { uuid } from "../util/id";

export function blankS3Account(): S3Account {
  return { id: uuid(), name: "", endpoint: "", region: "us-east-1", bucket: null, access_key_id: "", secret: null, path_style: false };
}

/** S3 accounts (vault items, synced like connections). Secret keys never reach the UI. */
class S3Accounts {
  list = $state<S3Account[]>([]);
  /** Account in the editor; `secret` is a new secret key to store (empty keeps the old one). */
  editing = $state<{ account: S3Account; isNew: boolean; secret: string } | null>(null);

  sorted = $derived([...this.list].sort((a, b) => a.name.localeCompare(b.name)));

  async load() {
    this.list = await api.listS3Accounts();
  }

  byId(id: string) {
    return this.list.find((a) => a.id === id) ?? null;
  }

  async save(a: S3Account, secret: string) {
    const clean = $state.snapshot(a) as S3Account;
    clean.bucket = clean.bucket?.trim() || null;
    const saved = await api.saveS3Account(clean, secret.trim() || null);
    const i = this.list.findIndex((x) => x.id === saved.id);
    if (i >= 0) this.list[i] = saved;
    else this.list.push(saved);
    return saved;
  }

  async remove(id: string) {
    await api.deleteS3Account(id);
    this.list = this.list.filter((a) => a.id !== id);
  }

  edit(a: S3Account = blankS3Account(), isNew = false) {
    this.editing = { account: structuredClone($state.snapshot(a) as S3Account), isNew, secret: "" };
  }
}

export const s3 = new S3Accounts();
