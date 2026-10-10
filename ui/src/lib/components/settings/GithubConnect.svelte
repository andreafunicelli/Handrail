<script lang="ts">
  import { onDestroy } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { GithubDeviceStart } from "../../api/types";
  import { toasts } from "../../stores/toasts.svelte";

  /** GitHub device-flow sign-in. Calls `onconnected` once the private vault repo is set up. */
  let { onconnected, disabled = false }: { onconnected: (repo: string) => void | Promise<void>; disabled?: boolean } = $props();

  let device = $state<GithubDeviceStart | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;
  onDestroy(() => timer && clearTimeout(timer));

  async function start() {
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
          await onconnected(r.repo ?? "");
        } else if (r.status === "expired" || r.status === "denied") {
          device = null;
          toasts.error(t(r.status === "expired" ? "sync.githubExpired" : "sync.githubDenied"));
        } else poll(r.status === "slow_down" ? interval + 5 : interval);
      } catch (e) {
        device = null;
        toasts.error(e);
      }
    }, Math.max(1, interval) * 1000);
  }

  function cancel() {
    if (timer) clearTimeout(timer);
    device = null;
  }
</script>

{#if device}
  <div class="device">
    <span class="hint">{t("sync.githubCode")}</span>
    <a class="mono uri" href={device.verification_uri} target="_blank" rel="noreferrer noopener">{device.verification_uri}</a>
    <div class="code mono">{device.user_code}</div>
    <span class="hint waiting">{t("sync.githubWaiting")}</span>
    <button class="btn sm ghost" onclick={cancel}>{t("common.cancel")}</button>
  </div>
{:else}
  <button class="btn" onclick={start} {disabled}><Icon name="link" size={14} />{t("sync.githubConnect")}</button>
  <p class="hint scope">{t("sync.githubScope")}</p>
{/if}

<style>
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
  .scope {
    margin: 6px 0 0;
  }
</style>
