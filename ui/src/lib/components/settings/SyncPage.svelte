<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t, type MessageKey } from "../../i18n";
  import { api } from "../../api";
  import type { ConflictView, FolderPreset, SnapshotInfo, SyncStatus } from "../../api/types";
  import GithubConnect from "./GithubConnect.svelte";
  import { reloadVaultData } from "../../stores/vaultData";
  import { toasts } from "../../stores/toasts.svelte";
  import { formatBytes, relativeTime } from "../../util/time";

  let status = $state<SyncStatus | null>(null);
  let presets = $state<FolderPreset[]>([]);
  let snapshots = $state<SnapshotInfo[]>([]);
  let conflicts = $state<ConflictView[]>([]);
  let folder = $state("");
  let busy = $state(false);
  /** Shows the "where to sync" choices while a target is already configured. */
  let changing = $state(false);
  /** Two-step confirmation: the key of the action armed by the first click. */
  let armed = $state<string | null>(null);
  /** Re-renders relative times. */
  let now = $state(Date.now());
  let armTimer: ReturnType<typeof setTimeout> | null = null;
  let unlisten: (() => void) | null = null;
  const clock = setInterval(() => (now = Date.now()), 30_000);

  const configured = $derived(!!status?.config);
  const preset = $derived(
    status?.config?.kind === "folder" ? presets.find((p) => p.path === (status!.config as { path: string }).path) : undefined,
  );
  const target = $derived.by(() => {
    const c = status?.config;
    if (!c) return null;
    if (c.kind === "github") return { title: `GitHub · ${c.owner}/${c.repo}`, detail: c.branch };
    return { title: preset?.label ?? t("sync.folderTitle"), detail: c.path };
  });

  async function loadStatus() {
    try {
      status = await api.syncStatus();
    } catch (e) {
      toasts.error(e);
    }
  }
  async function loadHistory() {
    try {
      [snapshots, conflicts] = await Promise.all([api.vaultSnapshots(), api.vaultConflicts()]);
    } catch (e) {
      toasts.error(e);
    }
  }
  onMount(async () => {
    unlisten = await api.on("sync://status", (s) => {
      const finished = status?.syncing && !s.syncing;
      status = s;
      // A finished sync may have taken a snapshot or recorded conflicts.
      if (finished) void loadHistory();
    });
    await Promise.all([loadStatus(), loadHistory(), api.syncFolderPresets().then((p) => (presets = p), toasts.error)]);
  });
  onDestroy(() => {
    unlisten?.();
    clearInterval(clock);
    if (armTimer) clearTimeout(armTimer);
  });

  /** First click arms `key`; a second click within 4 s confirms. */
  function confirm(key: string): boolean {
    if (armed === key) {
      armed = null;
      return true;
    }
    armed = key;
    if (armTimer) clearTimeout(armTimer);
    armTimer = setTimeout(() => (armed = null), 4000);
    return false;
  }

  async function run(fn: () => Promise<unknown>, success?: string) {
    busy = true;
    try {
      await fn();
      if (success) toasts.success(success);
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
  }

  const useFolderPath = (path: string) =>
    run(async () => {
      await api.syncConfigureFolder(path);
      changing = false;
      await loadStatus();
      // A folder that already holds a vault: this device may need to join it first.
      if (!status?.can_join) await api.syncNow();
    }, t("sync.connected"));

  function useCustomFolder(e: SubmitEvent) {
    e.preventDefault();
    if (folder.trim()) void useFolderPath(folder.trim());
  }

  async function githubConnected() {
    changing = false;
    toasts.success(t("sync.connected"));
    await loadStatus();
  }

  const syncNow = () => run(() => api.syncNow());
  const join = () => run(async () => (await api.syncJoin(), await loadStatus()));
  const setAuto = (enabled: boolean) => run(() => api.syncSetAuto(enabled));
  function disconnect() {
    if (confirm("disconnect")) void run(() => api.syncDisconnect(), t("sync.disconnected"));
  }

  const snapshotNow = () => run(async () => (await api.vaultSnapshotCreate(), await loadHistory()), t("sync.snapshotTaken"));
  function restoreSnapshot(s: SnapshotInfo) {
    if (!confirm(`snap:${s.name}`)) return;
    void run(
      async () => (await api.vaultSnapshotRestore(s.name), await Promise.all([loadHistory(), reloadVaultData()])),
      t("sync.restored", { when: new Date(s.created_ms).toLocaleString() }),
    );
  }
  function keepConflict(c: ConflictView) {
    void run(
      async () => (await api.vaultConflictRestore(c.file_id, c.recorded_at_ms), await reloadVaultData()),
      t("sync.kept", { label: c.label }),
    );
  }
  function clearConflicts() {
    if (confirm("clear")) void run(async () => (await api.vaultConflictsClear(), await loadHistory()));
  }

  const typeLabel = (type: string) => {
    const key = `sync.type.${type}` as MessageKey;
    const label = t(key);
    return label === key ? type : label;
  };
  const ago = (ms: number) => (void now, relativeTime(ms));
  const driveIcon = (p: FolderPreset) => (p.drive === "icloud" ? "cloud" : "folder");
</script>

<p class="intro">{t("sync.body")}</p>

<!-- Status -->
<div class="card status" class:err={!!status?.last_error}>
  <div class="list-row">
    <Icon name="cloud" size={15} />
    <div class="st">
      {#if target}
        <strong>{target.title}</strong>
        <span class="hint mono path" title={target.detail}>{target.detail}</span>
        {#if status?.syncing}
          <span class="hint live">{t("sync.syncing")}</span>
        {:else if status?.last_error}
          <span class="hint errtext" role="status">{t("sync.failed", { error: status.last_error })}{status.auto ? ` ${t("sync.retrying")}` : ""}</span>
        {:else}
          <span class="hint">
            {status?.last_sync_ms ? t("sync.last", { when: ago(status.last_sync_ms) }) : t("sync.notYet")}
            {#if status?.last_report && (status.last_report.pulled || status.last_report.pushed)}
              · ↓ {status.last_report.pulled} ↑ {status.last_report.pushed}
            {/if}
          </span>
        {/if}
        {#each status?.last_report?.warnings ?? [] as w, i (i)}<span class="hint warn">{w}</span>{/each}
      {:else}
        <strong>{t("sync.off")}</strong>
        <span class="hint">{t("sync.offHint")}</span>
      {/if}
    </div>
    {#if status?.can_join}<button class="btn sm primary" onclick={join} disabled={busy}>{t("sync.join")}</button>{/if}
    {#if configured && !status?.can_join}
      <button class="btn sm" onclick={syncNow} disabled={busy || status?.syncing}><Icon name="refresh" size={13} />{t("sync.now")}</button>
    {/if}
  </div>
  {#if configured}
    <div class="list-row controls">
      <label class="check toggle">
        <input type="checkbox" checked={status?.auto} onchange={(e) => setAuto((e.currentTarget as HTMLInputElement).checked)} disabled={busy} />
        <span><strong>{t("sync.auto")}</strong><br /><span class="hint">{t("sync.autoHint")}</span></span>
      </label>
      <span class="grow"></span>
      <button class="btn sm ghost" onclick={() => (changing = !changing)}>{t(changing ? "common.cancel" : "sync.change")}</button>
      <button class="btn sm ghost danger-text" onclick={disconnect} disabled={busy}>
        {armed === "disconnect" ? t("sync.disconnectConfirm") : t("sync.disconnect")}
      </button>
    </div>
  {/if}
</div>

<!-- Where to sync -->
{#if !configured || changing}
  <h3>{t("sync.cloudDrives")}</h3>
  {#if presets.length}
    <div class="card">
      {#each presets as p (p.path)}
        <div class="list-row">
          <Icon name={driveIcon(p)} size={15} />
          <div class="st">
            <strong>{p.label}</strong>
            <span class="hint mono path" title={p.path}>{p.path}</span>
            {#if p.has_vault}<span class="hint accent">{t("sync.hasVault")}</span>{/if}
          </div>
          {#if p.path === preset?.path}
            <span class="pill">{t("sync.inUse")}</span>
          {:else}
            <button class="btn sm" onclick={() => useFolderPath(p.path)} disabled={busy}>{t("sync.use")}</button>
          {/if}
        </div>
      {/each}
    </div>
  {:else}
    <p class="hint sub">{t("sync.cloudNone")}</p>
  {/if}

  <h3>{t("sync.github")}</h3>
  <GithubConnect onconnected={githubConnected} disabled={busy} />

  <h3>{t("sync.folder")}</h3>
  <p class="hint sub">{t("sync.folderHint")}</p>
  <form class="row" onsubmit={useCustomFolder}>
    <input class="input mono" bind:value={folder} placeholder="/Users/me/Sync/Handrail" aria-label={t("sync.folderPath")} />
    <button class="btn" disabled={busy || !folder.trim()}>{t("sync.folderUse")}</button>
  </form>
{/if}

<!-- Snapshots -->
<div class="head-row">
  <h3>{t("sync.snapshots")}</h3>
  <button class="btn sm ghost" onclick={snapshotNow} disabled={busy}><Icon name="plus" size={13} />{t("sync.snapshotNow")}</button>
</div>
<p class="hint sub">{t("sync.snapshotsHint")}</p>
<div class="card">
  {#each snapshots as s (s.name)}
    <div class="list-row">
      <Icon name="clock" size={14} />
      <span class="grow">{new Date(s.created_ms).toLocaleString()} <span class="hint">· {ago(s.created_ms)} · {formatBytes(s.size)}</span></span>
      <button class="btn sm" class:danger={armed === `snap:${s.name}`} onclick={() => restoreSnapshot(s)} disabled={busy}>
        {armed === `snap:${s.name}` ? t("sync.restoreConfirm") : t("sync.restore")}
      </button>
    </div>
  {:else}
    <div class="list-row hint">{t("sync.noSnapshots")}</div>
  {/each}
</div>

<!-- Conflicts -->
<div class="head-row">
  <h3>{t("sync.conflicts")}</h3>
  {#if conflicts.length}
    <button class="btn sm ghost" onclick={clearConflicts} disabled={busy}>{armed === "clear" ? t("sync.clearConfirm") : t("sync.clear")}</button>
  {/if}
</div>
<p class="hint sub">{t("sync.conflictsHint")}</p>
<div class="card">
  {#each conflicts as c (c.file_id + c.recorded_at_ms)}
    <div class="list-row">
      <div class="st">
        <strong>{c.label} <span class="pill">{typeLabel(c.item_type)}</span></strong>
        <span class="hint">{t(c.loser_deleted ? "sync.conflictDeleted" : "sync.conflictLine", { loser: ago(c.loser_ms), winner: ago(c.winner_ms) })}</span>
      </div>
      <button class="btn sm" onclick={() => keepConflict(c)} disabled={busy}>{t(c.loser_deleted ? "sync.keepDeletion" : "sync.keep")}</button>
    </div>
  {:else}
    <div class="list-row hint">{t("sync.noConflicts")}</div>
  {/each}
</div>

<style>
  .st {
    flex: 1;
    display: grid;
    gap: 1px;
    min-width: 0;
  }
  .st strong {
    font-weight: 550;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .path {
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .danger-text {
    color: var(--danger);
  }
  .status.err {
    border-color: var(--danger);
  }
  .errtext {
    color: var(--danger);
  }
  .warn {
    color: var(--warn);
  }
  .accent {
    color: var(--accent-text);
  }
  .live {
    animation: pulse 1.6s var(--ease) infinite;
  }
  .controls {
    align-items: center;
  }
  .toggle {
    align-items: flex-start;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .head-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .sub {
    margin: -4px 0 10px;
  }
  .row {
    display: flex;
    gap: 8px;
  }
</style>
