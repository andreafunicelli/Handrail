import { api } from "../api";
import type { Identity, SecretInfo, SshAuth } from "../api/types";
import { connections } from "./connections.svelte";

/** Saved keys and passwords (no values) and identities, shared between connections. */
class Credentials {
  list = $state<SecretInfo[]>([]);
  identities = $state<Identity[]>([]);

  keys = $derived(this.list.filter((s) => s.kind === "private_key").sort(byLabel));
  passwords = $derived(this.list.filter((s) => s.kind === "password").sort(byLabel));

  async load() {
    [this.list, this.identities] = await Promise.all([api.listSecrets(), api.listIdentities()]);
    this.identities.sort(byLabel);
  }

  /** Names of what logs in with secret `id` (as password, key or passphrase): connections
   *  with their own login, and identities (a connection on an identity counts through it). */
  usedBy(id: string, except?: string): string[] {
    return [
      ...connections.list.filter((c) => c.id !== except && !c.identity && uses(c.auth, id)).map((c) => c.name),
      ...this.identities.filter((i) => uses(i.auth, id)).map((i) => i.label),
    ];
  }

  /** Names of the connections that log in as identity `id`. */
  identityUsers(id: string): string[] {
    return connections.list.filter((c) => c.identity === id).map((c) => c.name);
  }

  identity(id: string | null | undefined): Identity | undefined {
    return id ? this.identities.find((i) => i.id === id) : undefined;
  }

  /** Save an identity; connections using it get its user name, so lists show who they log in
   *  as (the backend reads the identity itself when connecting). */
  async saveIdentity(identity: Identity) {
    const saved = await api.saveIdentity(identity);
    const i = this.identities.findIndex((x) => x.id === saved.id);
    if (i >= 0) this.identities[i] = saved;
    else this.identities.push(saved);
    this.identities.sort(byLabel);
    for (const c of connections.list.filter((c) => c.identity === saved.id && c.user !== saved.user)) {
      await connections.save({ ...$state.snapshot(c), user: saved.user });
    }
    return saved;
  }

  async removeIdentity(id: string) {
    await api.deleteIdentity(id);
    this.identities = this.identities.filter((i) => i.id !== id);
  }

  /** The passphrase stored with key `keyId` by a connection or identity using it. */
  passphraseFor(keyId: string): string | null {
    for (const a of [...connections.list.map((c) => c.auth), ...this.identities.map((i) => i.auth)]) {
      if (a.type === "key" && a.key === keyId && a.passphrase) return a.passphrase;
    }
    return null;
  }

  label(id: string) {
    return this.list.find((s) => s.id === id)?.label ?? "";
  }
}

const uses = (a: SshAuth, id: string) => (a.type === "password" && a.secret === id) || (a.type === "key" && (a.key === id || a.passphrase === id));
const byLabel = (a: { label: string }, b: { label: string }) => a.label.localeCompare(b.label);

export const credentials = new Credentials();
