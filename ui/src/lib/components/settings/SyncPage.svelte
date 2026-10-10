<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { GithubDeviceStart, SyncStatus } from "../../api/types";
  import { toasts } from "../../stores/toasts.svelte";

  let status = $state<SyncStatus | null>(null);
  let device = $state<GithubDeviceStart | null>(null);
  let folder = $state("");
  let busy = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  const target = $derived(
    !status?.config ? null : status.config.kind === "github" ? `${status.config.owner}/${status.config.repo}` : status.config.path,
  );

  async function refresh() {
    try {
      status = await api.syncStatus();
    } catch (e) {
      toasts.error(e);
    }
  }
  onMount(refresh);
  onDestroy(() => timer && clearTimeout(timer));

  async function startGithub() {
    try {
      device = await api.syncGithubStart();
      poll(device.interval);
    } catch (e) {
      toasts.error(e);
    }
  }
  function poll(interval: number) {
    timer = setTimeout(async () => {
      try {
        const r = await api.syncGithubPoll();
        if (r.done || r.status === "ok") {
          device = null;
          toasts.success(t("sync.connected"));
          await refresh();
        } else if (r.status === "expired" || r.status === "denied") {
          device = null;
          toasts.error(r.status);
        } else poll(r.status === "slow_down" ? interval + 5 : interval);
      } catch (e) {
        device = null;
        toasts.error(e);
      }
    }, Math.max(1, interval) * 1000);
  }
  async function useFolder(e: SubmitEvent) {
    e.preventDefault();
    if (!folder.trim()) return;
    try {
      await api.syncConfigureFolder(folder.trim());
      toasts.success(t("sync.connected"));
      await refresh();
    } catch (err) {
      toasts.error(err);
    }
  }
  async function syncNow() {
    busy = true;
    try {
      await api.syncNow();
      await refresh();
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
  }
  async function join() {
    busy = true;
    try {
      await api.syncJoin();
      await refresh();
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
  }
  const when = (ms: number | null | undefined) => (ms ? new Date(ms).toLocaleString() : t("sync.never"));
</script>

<p class="intro">{t("sync.body")}</p>

<div class="card status">
  <div class="list-row">
    <Icon name="cloud" size={15} />
    <div class="st">
      <strong>{target ?? t("sync.off")}</strong>
      {#if status?.config}<span class="hint">{t("sync.last", { when: when(status.last_sync_ms) })}</span>{/if}
      {#if status?.last_report}
        <span class="hint">↓ {status.last_report.pulled} · ↑ {status.last_report.pushed}{status.last_report.conflicts ? ` · ${status.last_report.conflicts} conflicts` : ""}</span>
      {/if}
    </div>
    {#if status?.can_join}<button class="btn sm" onclick={join} disabled={busy}>{t("sync.join")}</button>{/if}
    {#if status?.config}<button class="btn sm" onclick={syncNow} disabled={busy}><Icon name="refresh" size={13} />{t("sync.now")}</button>{/if}
  </div>
</div>

<h3>{t("sync.github")}</h3>
{#if device}
  <div class="device">
    <span class="hint">{t("sync.githubCode")}</span>
    <a class="mono uri" href={device.verification_uri} target="_blank" rel="noreferrer noopener">{device.verification_uri}</a>
    <div class="code mono">{device.user_code}</div>
    <span class="hint waiting">{t("sync.githubWaiting")}</span>
  </div>
{:else}
  <button class="btn" onclick={startGithub}><Icon name="link" size={14} />{t("sync.githubConnect")}</button>
{/if}

<h3>{t("sync.folder")}</h3>
<p class="hint sub">{t("sync.folderHint")}</p>
<form class="row" onsubmit={useFolder}>
  <input class="input mono" bind:value={folder} placeholder="~/Dropbox/AIHubTerm" aria-label={t("sync.folderPath")} />
  <button class="btn" disabled={!folder.trim()}>{t("sync.folderUse")}</button>
</form>

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
  .device {
    display: grid;
    justify-items: start;
    gap: 6px;
    padding: 16px;
    border-radius: var(--r-lg);
    border: 1px dashed var(--border-strong);
  }
  .uri {
    color: var(--accent-text);
    font-size: 12px;
  }
  .code {
    font-size: 22px;
    letter-spacing: 0.12em;
    color: var(--text);
    user-select: all;
  }
  .waiting {
    animation: pulse 1.6s var(--ease) infinite;
  }
  .sub {
    margin: -4px 0 10px;
  }
  .row {
    display: flex;
    gap: 8px;
  }
</style>
