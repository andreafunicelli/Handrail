import type { SyncStatus } from "../api/types";
import { agent } from "./agent.svelte";
import { connections } from "./connections.svelte";
import { credentials } from "./credentials.svelte";
import { s3 } from "./s3.svelte";
import { snippets } from "./snippets.svelte";

/**
 * Reload every store that caches vault items (connections and groups, saved keys and passwords,
 * identities, snippets, S3 accounts, providers, harness profiles). Call after anything changes the vault behind the UI's back: a sync that pulled,
 * a snapshot or conflict restore.
 */
export function reloadVaultData(): Promise<unknown> {
  return Promise.all([connections.load(), credentials.load(), snippets.load(), s3.load(), agent.reload()]);
}

/**
 * A `sync://status` event after which the cached vault items are stale: a sync completed (its
 * time changed; robust to having missed the "started" event) and pulled or merged something.
 */
export function syncChangedVault(prev: SyncStatus | null, next: SyncStatus): boolean {
  const completed = !next.syncing && next.last_sync_ms != null && next.last_sync_ms !== prev?.last_sync_ms;
  const r = next.last_report;
  return completed && !next.last_error && !!r && r.pulled + r.conflicts > 0;
}
