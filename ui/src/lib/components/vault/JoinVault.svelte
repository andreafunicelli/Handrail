<script lang="ts">
  import { onMount } from "svelte";
  import VaultFrame from "./VaultFrame.svelte";
  import Icon from "../icons/Icon.svelte";
  import GithubConnect from "../settings/GithubConnect.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import { isIpcError } from "../../api/errors";
  import type { FolderPreset } from "../../api/types";
  import { toasts } from "../../stores/toasts.svelte";
  import { vault } from "../../stores/vault.svelte";

  /** New device: point sync at where another device already syncs, download the vault's
   *  manifest (`sync_join`), then unlock with master password + Secret Key. */
  let presets = $state<FolderPreset[]>([]);
  let folder = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  onMount(() => {
    api.syncFolderPresets().then((p) => (presets = p), toasts.error);
  });

  async function join() {
    busy = true;
    error = null;
    try {
      await api.syncJoin();
      await vault.afterJoin();
    } catch (e) {
      if (isIpcError(e) && e.kind === "not_found") error = t("vault.join.none");
      else toasts.error(e);
    } finally {
      busy = false;
    }
  }

  async function useFolder(path: string) {
    busy = true;
    try {
      await api.syncConfigureFolder(path);
    } catch (e) {
      toasts.error(e);
      busy = false;
      return;
    }
    await join();
  }

  function useCustom(e: SubmitEvent) {
    e.preventDefault();
    if (folder.trim()) void useFolder(folder.trim());
  }

  async function back() {
    // Forget the half-configured target so a new vault does not try to sync into it.
    await api.syncDisconnect().catch(() => {});
    vault.phase = "create";
  }
</script>

<VaultFrame>
  <h1>{t("vault.join.title")}</h1>
  <p class="lede">{t("vault.join.body")}</p>

  {#if busy}<p class="hint live">{t("vault.join.joining")}</p>{/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}

  <section>
    <h2>{t("vault.join.github")}</h2>
    <GithubConnect onconnected={join} disabled={busy} />
  </section>

  <section>
    <h2>{t("vault.join.drives")}</h2>
    {#if presets.length}
      <div class="card">
        {#each presets as p (p.path)}
          <div class="list-row">
            <Icon name={p.drive === "icloud" ? "cloud" : "folder"} size={15} />
            <div class="st">
              <strong>{p.label}</strong>
              {#if p.has_vault}<span class="hint found">{t("vault.join.found")}</span>{/if}
            </div>
            <button class="btn sm" class:primary={p.has_vault} onclick={() => useFolder(p.path)} disabled={busy}>{t("sync.join")}</button>
          </div>
        {/each}
      </div>
    {:else}
      <p class="hint">{t("vault.join.noDrives")}</p>
    {/if}
  </section>

  <section>
    <h2>{t("vault.join.folder")}</h2>
    <form class="row" onsubmit={useCustom}>
      <input class="input mono" bind:value={folder} placeholder="/Users/me/Sync/AIHubTerm" aria-label={t("sync.folderPath")} />
      <button class="btn" disabled={busy || !folder.trim()}>{t("sync.join")}</button>
    </form>
  </section>

  <button type="button" class="link" onclick={back} disabled={busy}>{t("vault.join.back")}</button>
</VaultFrame>

<style>
  .card {
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--bg);
  }
  .list-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-top: 1px solid var(--border);
    min-width: 0;
  }
  .list-row:first-child {
    border-top: 0;
  }
  section {
    margin-top: 20px;
  }
  h2 {
    font-size: var(--fs-small);
    font-weight: 550;
    color: var(--muted);
    margin: 0 0 8px;
  }
  .st {
    flex: 1;
    display: grid;
    min-width: 0;
  }
  .found {
    color: var(--accent-text);
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .row .input {
    flex: 1;
    min-width: 0;
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-meta);
  }
  .live {
    animation: pulse 1.6s var(--ease) infinite;
  }
  .link {
    display: block;
    margin: 22px auto 0;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  .link:hover {
    color: var(--text);
  }
</style>
