<script lang="ts">
  import { api } from "../../api";
  import type { ServerStats, TunnelStatus } from "../../api/types";
  import { t } from "../../i18n";
  import Icon from "../icons/Icon.svelte";
  import ContextMenu, { type MenuEntry } from "../common/ContextMenu.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { copyText } from "../../util/clipboard";
  import { formatBytes } from "../../util/time";
  import { tunnelLabel, tunnelAddress } from "../../util/tunnels";
  import { connectionAddress } from "../../util/sessionContext";
  import { clockTime, metricLevel, metricsFreshness, secondsSince, showsNumbers, usedPct, type MetricLevel } from "../../util/sessionMetrics";
  import { jumpNames, loginUser, quickActions, sshConnectionOf } from "./sessionActions.svelte";

  /** Under an SSH terminal: who and where this session is (user@host, jump host, status), the
   *  server's load, its port forwards and the quick actions. */
  let { sessionId }: { sessionId: string } = $props();

  const POLL_MS = 5000;
  /** After this many failed reads in a row (no /proc, no `sh`), stop asking. */
  const MAX_FAILS = 3;

  const info = $derived(sessions.infos[sessionId]);
  const live = $derived(!!info && info.state !== "disconnected");
  const tunnels = $derived(sessions.tunnels[sessionId] ?? []);
  const conn = $derived(sshConnectionOf(sessionId));
  const jumps = $derived(conn ? jumpNames(conn) : []);
  const user = $derived(info?.user || (conn ? loginUser(conn) : ""));
  const host = $derived(conn?.host ?? info?.host ?? "");
  const env = $derived(info?.env ?? conn?.env ?? "none");

  let stats = $state<ServerStats | null>(null);
  let lastOk = $state<number | null>(null);
  let failed = $state(false);
  let now = $state(Date.now());

  // Poll while the session is live and the window is visible; a new session starts over.
  $effect(() => {
    if (!live) return;
    const id = sessionId;
    let fails = 0;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    failed = false;
    const tick = async () => {
      if (stopped) return;
      if (document.visibilityState === "visible") {
        try {
          const s = await api.sessionStats(id);
          if (stopped) return;
          fails = 0;
          stats = s;
          lastOk = Date.now();
        } catch {
          if (stopped) return;
          if (++fails >= MAX_FAILS) {
            failed = true;
            return;
          }
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

  // A clock for "updated 3 s ago" and for noticing stale numbers.
  $effect(() => {
    if (!live) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  const fresh = $derived(metricsFreshness({ live, failed, lastOk, now, pollMs: POLL_MS }));
  const numbers = $derived(showsNumbers(fresh) && !!stats);
  const mem = $derived(usedPct(stats?.mem_used, stats?.mem_total));
  const disk = $derived(usedPct(stats?.disk_used, stats?.disk_total));
  const has = (v: number | null | undefined): v is number => v != null && Number.isFinite(v);
  const at = $derived(lastOk != null ? clockTime(lastOk) : "");
  const updated = $derived(lastOk != null ? t("sess.updatedAt", { time: at }) : "");

  function uptime(s: number) {
    const d = Math.floor(s / 86400);
    const h = Math.floor((s % 86400) / 3600);
    if (d) return t("bar.upDays", { d, h });
    const m = Math.floor((s % 3600) / 60);
    return h ? t("bar.upHours", { h, m }) : t("bar.upMinutes", { m });
  }

  const levelText = (l: MetricLevel | null) => (l === "danger" ? t("sess.levelDanger") : l === "warn" ? t("sess.levelWarn") : "");
  /** Tooltip lines of one metric: what it is, the detailed values, the level and when it was read. */
  const tip = (...lines: (string | null | false | undefined)[]) => lines.filter(Boolean).join("\n");

  function tunnelState(x: TunnelStatus) {
    return x.error ? "bad" : x.port != null ? "ok" : "starting";
  }

  async function copy(x: TunnelStatus) {
    const addr = tunnelAddress(x);
    if (!addr) return;
    if (await copyText(addr)) toasts.success(t("bar.copied", { addr }));
    else toasts.error(t("copy.failed"));
  }

  const identityTip = $derived(
    conn
      ? tip(
          `${conn.name} · ${conn.kind.toUpperCase()}`,
          connectionAddress({ user, host: conn.host, port: conn.port }),
          `${t("sess.fEnv")}: ${t(`env.${env}`)}`,
          jumps.length ? `${t("sess.fJump")}: ${jumps.join(" → ")}` : null,
          info?.host && info.host !== conn.host ? t("sess.shellHost", { host: info.host }) : null,
        )
      : "",
  );

  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  function actionsMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { x: r.right - 240, y: r.top - 4, items: quickActions(sessionId) };
  }

  const shown = $derived(!!info && (!!conn || (live && ((stats && (has(stats.cpu) || has(mem) || has(disk))) || tunnels.length))));
</script>

{#snippet metric(key: string, label: string, pct: number | null, value: string, detail: string)}
  {@const lv = metricLevel(pct)}
  <span class="m lv-{lv ?? 'none'}" data-metric={key} title={tip(`${label}: ${value}`, detail, levelText(lv), updated)}>
    <span class="k">{label}</span>
    <span class="gauge" aria-hidden="true"><span style:width="{Math.min(100, Math.round(pct ?? 0))}%"></span></span>
    <span class="v">{value}</span>
    {#if lv === "warn" || lv === "danger"}<span class="flag" aria-hidden="true">{lv === "danger" ? "!!" : "!"}</span><span class="sr-only">{levelText(lv)}</span>{/if}
  </span>
{/snippet}

{#if shown}
  <div class="bar" class:stale={fresh === "stale"} role="group" aria-label={t("bar.label")}>
    {#if conn}
      <span class="who" title={identityTip}>
        {#if env === "prod"}
          <span class="env-prod-badge">{t("sess.prod")}</span>
        {:else if env !== "none"}
          <span class="tick env-{env}" aria-hidden="true"></span>
        {/if}
        <span class="sr-only">{t("sess.envLabel", { env: t(`env.${env}`) })}</span>
        <span class="addr mono">{user ? `${user}@` : ""}{host}</span>
        {#if jumps.length}<span class="via">{t("sess.via", { name: jumps[jumps.length - 1] })}</span>{/if}
      </span>
      <span class="status" title={t("sess.statusTip", { state: t(`state.${info.state}`) })}>
        <span class="dot {info.state}" aria-hidden="true"></span>
        <span class="st">{t(`state.${info.state}`)}</span>
      </span>
    {/if}
    <span class="metrics" role="status" aria-live="off">
      {#if numbers && stats}
        {#if has(stats.cpu)}
          {@render metric("cpu", t("bar.cpu"), stats.cpu, `${Math.round(stats.cpu)}%`, tip(t("sess.cpuTip"), stats.cores ? t("bar.cores", { n: stats.cores }) : null))}
        {/if}
        {#if has(mem)}
          {@render metric("mem", t("bar.mem"), mem, `${Math.round(mem)}%`, t("bar.of", { used: formatBytes(stats.mem_used!), total: formatBytes(stats.mem_total!) }))}
        {/if}
        {#if has(disk)}
          {@render metric("disk", t("bar.disk"), disk, `${Math.round(disk)}%`, t("bar.diskOf", { used: formatBytes(stats.disk_used!), total: formatBytes(stats.disk_total!) }))}
        {/if}
        {#if has(stats.load1)}
          <span class="m plain" title={tip(`${t("bar.load")}: ${stats.load1.toFixed(2)}`, t("bar.loadHint"), stats.cores ? t("bar.cores", { n: stats.cores }) : null, updated)}>
            <span class="k">{t("bar.load")}</span><span class="v">{stats.load1.toFixed(2)}</span>
          </span>
        {/if}
        {#if has(stats.uptime_s)}<span class="m plain wide" title={updated}><span class="v">{uptime(stats.uptime_s)}</span></span>{/if}
        {#if fresh === "stale"}
          <span class="fresh stale-pill" title={tip(t("sess.staleTip"), updated)}>{t("sess.stale")}</span>
        {:else}
          <span class="fresh ago" title={tip(updated, t("sess.pollTip", { s: POLL_MS / 1000 }))}>{t("sess.updatedAgo", { s: secondsSince(lastOk!, now) })}</span>
        {/if}
      {:else if fresh === "offline"}
        <span class="m none" title={t("sess.offlineTip")}>
          <span class="k">{t("bar.cpu")}</span><span class="v">—</span>
          <span class="k">{t("bar.mem")}</span><span class="v">—</span>
          <span class="k">{t("bar.disk")}</span><span class="v">—</span>
        </span>
      {:else if fresh === "unavailable"}
        <span class="fresh" title={t("sess.unavailableTip")}>{t("sess.unavailable")}</span>
      {:else}
        <span class="fresh">{t("sess.loading")}</span>
      {/if}
    </span>
    <span class="grow"></span>
    {#each tunnels as x, i (i)}
      <button class="tun {tunnelState(x)}" title={x.error ?? (x.port != null ? t("bar.tunnelCopy") : t("bar.tunnelStarting"))} onclick={() => copy(x)}>
        <span>{tunnelLabel(x)}</span>
      </button>
    {/each}
    {#if conn}
      <button class="more" title={t("sess.actions")} aria-label={t("sess.actions")} aria-haspopup="menu" aria-expanded={!!menu} onclick={actionsMenu}>
        <Icon name="more" size={16} />
      </button>
    {/if}
  </div>
{/if}
{#if menu}<ContextMenu {...menu} onclose={() => (menu = null)} />{/if}

<style>
  .bar {
    container-type: inline-size;
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    height: 24px;
    padding: 0 2px 0 var(--sp-3);
    border-top: 1px solid var(--border);
    background: var(--panel);
    color: var(--muted);
    font-size: var(--fs-small);
    white-space: nowrap;
    overflow: hidden;
  }
  .who {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 48px;
    flex: 0 1 auto;
    overflow: hidden;
  }
  .who .env-prod-badge {
    height: 16px;
    padding: 0 5px;
  }
  .tick {
    flex: none;
    width: 3px;
    height: 10px;
    border-radius: 2px;
  }
  .tick.env-staging {
    background: var(--env-staging);
  }
  .tick.env-dev {
    background: var(--env-dev);
    opacity: 0.7;
  }
  .addr {
    color: var(--text-2);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .via {
    color: var(--faint);
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 0 2 auto;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex: none;
  }
  /* The numbers keep their room; the address gives way (its tooltip has it in full). */
  .metrics {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
    flex: none;
    padding-left: var(--sp-3);
    border-left: 1px solid var(--border);
  }
  .bar > .metrics:first-child {
    padding-left: 0;
    border-left: 0;
  }
  .m {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .m.none {
    gap: 4px;
  }
  .m.none .v {
    margin-right: 6px;
    color: var(--faint);
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
  .lv-warn .gauge span {
    background: var(--warn);
  }
  .lv-warn .v,
  .lv-warn .flag {
    color: var(--warn);
  }
  .lv-danger .gauge span {
    background: var(--danger);
  }
  .lv-danger .v,
  .lv-danger .flag {
    color: var(--danger);
  }
  .flag {
    font-weight: 700;
    margin-left: -2px;
  }
  /* Old numbers: kept for context, but visibly not live. */
  .stale .m .v,
  .stale .m .flag {
    color: var(--faint);
  }
  .stale .m .gauge span {
    background: var(--faint);
  }
  .fresh {
    color: var(--faint);
  }
  .stale-pill {
    height: 16px;
    line-height: 16px;
    padding: 0 6px;
    border-radius: 8px;
    background: var(--warn-soft);
    color: var(--warn);
    font-size: var(--fs-tiny);
    font-weight: 600;
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
    font-size: var(--fs-small);
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
  .more {
    flex: none;
    display: grid;
    place-items: center;
    width: 22px;
    height: 20px;
    border-radius: var(--r-sm);
    color: var(--text-2);
  }
  .more:hover,
  .more[aria-expanded="true"] {
    background: var(--hover);
    color: var(--text);
  }
  @container (max-width: 1000px) {
    .wide {
      display: none;
    }
  }
  @container (max-width: 900px) {
    .gauge {
      display: none;
    }
  }
  @container (max-width: 640px) {
    .plain,
    .via {
      display: none;
    }
  }
  @container (max-width: 540px) {
    .ago,
    .st {
      display: none;
    }
  }
  /* Narrow (a split pane): memory and disk only while they need attention. */
  @container (max-width: 480px) {
    .m.lv-ok:not([data-metric="cpu"]) {
      display: none;
    }
  }
  @container (max-width: 340px) {
    .m .k {
      display: none;
    }
  }
</style>
