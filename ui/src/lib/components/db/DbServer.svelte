<script lang="ts">
  import { onDestroy } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import DbGrid from "./DbGrid.svelte";
  import { api } from "../../api";
  import type { DbResultSet, DbServerInfo } from "../../api/types";
  import { t } from "../../i18n";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";

  /** The server panel of a database tab: processes (stop a query or close a connection),
   *  variables, status counters and accounts. Read on the side connection. */
  let { id, prod }: { id: string; prod: boolean } = $props();

  const TABS: DbServerInfo[] = ["processes", "variables", "status", "users"];
  let what = $state<DbServerInfo>("processes");
  let result = $state<DbResultSet | null>(null);
  let error = $state<string | null>(null);
  let filter = $state("");
  let auto = $state(false);
  let selected = $state<{ r: number; c: number } | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  /** Rows holding the filter text anywhere (case-insensitive). */
  const shown = $derived.by(() => {
    if (!result) return null;
    const f = filter.trim().toLowerCase();
    if (!f) return result;
    return { ...result, rows: result.rows.filter((r) => r.some((v) => typeof v === "string" && v.toLowerCase().includes(f))) };
  });
  const pickedPid = $derived.by(() => {
    if (what !== "processes" || !shown || !selected) return null;
    const v = shown.rows[selected.r]?.[0];
    return typeof v === "string" && /^\d+$/.test(v) ? Number(v) : null;
  });

  async function load() {
    try {
      result = await api.dbServerInfo(id, what);
      error = null;
    } catch (e) {
      error = String((e as { message?: string })?.message ?? e);
      result = null;
    }
  }

  function show(w: DbServerInfo) {
    what = w;
    filter = "";
    selected = null;
    result = null;
    void load();
  }

  $effect(() => {
    if (timer) clearInterval(timer);
    timer = auto && what === "processes" ? setInterval(() => void load(), 5000) : null;
  });
  onDestroy(() => timer && clearInterval(timer));

  async function kill(connection: boolean) {
    const pid = pickedPid;
    if (pid === null) return;
    const ok = await ui.ask(t(connection ? "db.srv.killConnTitle" : "db.srv.killQueryTitle", { pid }), {
      body: t(connection ? "db.srv.killConnBody" : "db.srv.killQueryBody") + (prod ? ` ${t("db.srv.prodNote")}` : ""),
      confirmLabel: t(connection ? "db.srv.killConn" : "db.srv.killQuery"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.dbKill(id, pid, connection);
      toasts.success(t("db.srv.killed", { pid }));
      selected = null;
      await load();
    } catch (e) {
      toasts.error(e);
    }
  }

  void load();
</script>

<div class="server">
  <div class="bar">
    <div class="seg" role="tablist" aria-label={t("db.srv.title")}>
      {#each TABS as w (w)}
        <button role="tab" aria-selected={what === w} class:on={what === w} onclick={() => show(w)}>{t(`db.srv.${w}`)}</button>
      {/each}
    </div>
    <input class="input" bind:value={filter} placeholder={t("db.srv.filter")} aria-label={t("db.srv.filter")} spellcheck="false" />
    <button class="icon-btn" title={t("db.refresh")} aria-label={t("db.refresh")} onclick={load}><Icon name="refresh" size={12} /></button>
    {#if what === "processes"}
      <label class="auto"><input type="checkbox" bind:checked={auto} />{t("db.srv.auto")}</label>
      <span class="spacer"></span>
      <button class="btn sm" disabled={pickedPid === null} onclick={() => kill(false)}>{t("db.srv.killQuery")}</button>
      <button class="btn sm danger" disabled={pickedPid === null} onclick={() => kill(true)}>{t("db.srv.killConn")}</button>
    {/if}
  </div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {:else if shown}
    <div class="gridbox"><DbGrid result={shown} bind:selected /></div>
  {:else}
    <p class="hint">{t("db.loading")}</p>
  {/if}
</div>

<style>
  .server {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
    font-size: 12px;
  }
  .seg {
    display: flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg button {
    border: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    padding: 3px 8px;
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent-soft);
    color: var(--text);
  }
  .bar .input {
    width: 200px;
    height: 26px;
    font-size: 12px;
  }
  .auto {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--muted);
  }
  .spacer {
    flex: 1;
  }
  .gridbox {
    flex: 1;
    min-height: 0;
  }
  .error {
    margin: 0;
    padding: 8px 10px;
    color: var(--danger);
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .hint {
    color: var(--muted);
    padding: 10px;
  }
</style>
