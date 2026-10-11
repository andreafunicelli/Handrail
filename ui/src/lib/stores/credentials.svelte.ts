import { api } from "../api";
import type { SecretInfo } from "../api/types";
import { connections } from "./connections.svelte";

/** Saved keys and passwords (no values), shared between connections. */
class Credentials {
  list = $state<SecretInfo[]>([]);

  keys = $derived(this.list.filter((s) => s.kind === "private_key").sort(byLabel));
  passwords = $derived(this.list.filter((s) => s.kind === "password").sort(byLabel));

  async load() {
    this.list = await api.listSecrets();
  }

  /** Names of the connections that log in with secret `id` (as password, key or passphrase). */
  usedBy(id: string, except?: string): string[] {
    return connections.list
      .filter((c) => c.id !== except)
      .filter((c) => (c.auth.type === "password" && c.auth.secret === id) || (c.auth.type === "key" && (c.auth.key === id || c.auth.passphrase === id)))
      .map((c) => c.name);
  }

  /** The passphrase stored with key `keyId` by a connection using it. */
  passphraseFor(keyId: string): string | null {
    for (const c of connections.list) if (c.auth.type === "key" && c.auth.key === keyId && c.auth.passphrase) return c.auth.passphrase;
    return null;
  }

  label(id: string) {
    return this.list.find((s) => s.id === id)?.label ?? "";
  }
}

const byLabel = (a: SecretInfo, b: SecretInfo) => a.label.localeCompare(b.label);

export const credentials = new Credentials();
