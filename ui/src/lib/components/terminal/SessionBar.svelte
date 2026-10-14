<script lang="ts">
  import { api } from "../../api";
  import type { ServerStats, TunnelStatus } from "../../api/types";
  import { t } from "../../i18n";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { copyText } from "../../util/clipboard";
  import { formatBytes } from "../../util/time";
  import { tunnelLabel, tunnelAddress } from "../../util/tunnels";

  /** Server load and port forwards of an SSH session, under its terminal. */
  let { sessionId }: { sessionId: string } = $props();

  const POLL_MS = 5000;
  /** After this many failed reads in a row (no /proc, no `sh`), stop asking. */
  const MAX_FAILS = 3;

  const info = $derived(sessions.infos[sessionId]);
  const live = $derived(!!info && info.state !== "disconnected");
  const tunnels = $derived(sessions.tunnels[sessionId] ?? []);
  let stats = $state<ServerStats | null>(null);

  // Poll while the session is live and the window is visible; a new session starts over.
  $effect(() => {
    if (!live) return;
    const id = sessionId;
    let fails = 0;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      if (stopped) return;
      if (document.visibilityState === "visible") {
        try {
          const s = await api.sessionStats(id);
          if (stopped) return;
          fails = 0;
          stats = s;
        } catch {
          if (++fails >= MAX_FAILS) return;
        }
      }
      if (!stopped) timer = setTimeout(tick, POLL_MS);
    };
    void tick();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  });

  const pct = (used: number | null, total: number | null) => (used != null && total ? (used / total) * 100 : null);
  const mem = $derived(pct(stats?.mem_used ?? null, stats?.mem_total ?? null));
  const disk = $derived(pct(stats?.disk_used ?? null, stats?.disk_total ?? null));
  const hot = (p: number | null) => p != null && p >= 90;

  function uptime(s: number) {
    const d = Math.floor(s / 86400);
    const h = Math.floor((s % 86400) / 3600);
    if (d) return t("bar.upDays", { d, h });
    const m = Math.floor((s % 3600) / 60);
    return h ? t("bar.upHours", { h, m }) : t("bar.upMinutes", { m });
  }

  function tunnelState(x: TunnelStatus) {
    return x.error ? "bad" : x.port != null ? "ok" : "starting";
  }

  async function copy(x: TunnelStatus) {
    const addr = tunnelAddress(x);
    if (!addr) return;
    if (await copyText(addr)) toasts.success(t("bar.copied", { addr }));
    else toasts.error(t("copy.failed"));
  }

  const has = (v: number | null | undefined): v is number => v != null && Number.isFinite(v);
  const shown = $derived(live && ((stats && (has(stats.cpu) || has(mem) || has(disk))) || tunnels.length));
</script>

{#if shown}
  <div class="bar" role="status" aria-label={t("bar.label")}>
    {#if stats}
      {#if has(stats.cpu)}
        <span class="m" class:hot={hot(stats.cpu)} title={stats.cores ? t("bar.cores", { n: stats.cores }) : ""}>
          <span class="k">{t("bar.cpu")}</span>
          <span class="gauge"><span style:width="{Math.round(stats.cpu)}%"></span></span>
          <span class="v">{Math.round(stats.cpu)}%</span>
        </span>
      {/if}
      {#if has(mem)}
        <span class="m" class:hot={hot(mem)} title={t("bar.of", { used: formatBytes(stats.mem_used!), total: formatBytes(stats.mem_total!) })}>
          <span class="k">{t("bar.mem")}</span>
          <span class="gauge"><span style:width="{Math.round(mem)}%"></span></span>
          <span class="v">{formatBytes(stats.mem_used!)}</span>
        </span>
      {/if}
      {#if has(disk)}
        <span class="m" class:hot={hot(disk)} title={t("bar.diskOf", { used: formatBytes(stats.disk_used!), total: formatBytes(stats.disk_total!) })}>
          <span class="k">{t("bar.disk")}</span>
          <span class="gauge"><span style:width="{Math.round(disk)}%"></span></span>
          <span class="v">{Math.round(disk)}%</span>
        </span>
      {/if}
      {#if has(stats.load1)}<span class="m plain" title={t("bar.loadHint")}><span class="k">{t("bar.load")}</span><span class="v">{stats.load1.toFixed(2)}</span></span>{/if}
      {#if has(stats.uptime_s)}<span class="m plain wide"><span class="v">{uptime(stats.uptime_s)}</span></span>{/if}
    {/if}
    <span class="grow"></span>
    {#each tunnels as x, i (i)}
      <button class="tun {tunnelState(x)}" title={x.error ?? (x.port != null ? t("bar.tunnelCopy") : t("bar.tunnelStarting"))} onclick={() => copy(x)}>
        <span>{tunnelLabel(x)}</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .bar {
    container-type: inline-size;
    display: flex;
    align-items: center;
    gap: 14px;
    height: 22px;
    padding: 0 8px 0 14px;
    border-top: 1px solid var(--border);
    background: var(--panel);
    color: var(--muted);
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
  }
  .m {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .k {
    color: var(--faint);
  }
  .v {
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
  }
  .gauge {
    position: relative;
    width: 34px;
    height: 4px;
    border-radius: 2px;
    background: var(--hover);
    overflow: hidden;
  }
  .gauge span {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--accent);
    border-radius: 2px;
    transition: width 0.4s var(--ease);
  }
  .hot .gauge span {
    background: var(--danger);
  }
  .hot .v {
    color: var(--danger);
  }
  .grow {
    flex: 1;
  }
  .tun {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 18px;
    padding: 0 7px;
    border-radius: 9px;
    border: 1px solid var(--border-strong);
    background: none;
    color: var(--text-2);
    font: inherit;
    font-family: var(--font-mono);
    font-size: 10.5px;
  }
  .tun::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--faint);
  }
  .tun.ok::before {
    background: var(--ok);
  }
  .tun.bad {
    border-color: var(--danger);
    color: var(--danger);
  }
  .tun.bad::before {
    background: var(--danger);
  }
  .tun:hover {
    background: var(--hover);
  }
  @container (max-width: 520px) {
    .gauge,
    .wide {
      display: none;
    }
  }
  @container (max-width: 380px) {
    .plain {
      display: none;
    }
  }
</style>
