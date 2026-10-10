<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { connections, blankConnection } from "../../stores/connections.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import type { Connection } from "../../api/types";

  const mod = typeof navigator !== "undefined" && /Mac|iP/.test(navigator.platform) ? "⌘" : "Ctrl";
  const pinned = $derived(connections.pinned.map((id) => connections.byId.get(id)).filter((c): c is Connection => !!c));
  const recent = $derived(
    connections.recent
      .filter((id) => !connections.pinned.includes(id))
      .map((id) => connections.byId.get(id))
      .filter((c): c is Connection => !!c)
      .slice(0, 6),
  );
  const fallback = $derived(!pinned.length && !recent.length ? connections.list.slice(0, 4) : []);

  const lede = t("start.body").split("{kbd}");
  const open = (c: Connection) => sessions.open(c.id).catch(toasts.error);
</script>

{#snippet card(c: Connection)}
  <button class="card" onclick={() => open(c)}>
    <span class="env env-{c.env}"></span>
    <span class="cname">{c.name}</span>
    <span class="chost mono">{c.user}@{c.host}{c.port !== 22 ? `:${c.port}` : ""}</span>
    {#if c.env !== "none"}<span class="cenv">{t(`env.${c.env}`)}</span>{/if}
  </button>
{/snippet}

<div class="start">
  <div class="inner">
    <h1>{t("start.title")}</h1>
    <p class="lede">
      {lede[0]}<span class="kbd">{mod}</span><span class="kbd">K</span>{lede[1] ?? ""}
    </p>

    {#if pinned.length}
      <section>
        <h2>{t("start.pinned")}</h2>
        <div class="cards">{#each pinned as c (c.id)}{@render card(c)}{/each}</div>
      </section>
    {/if}

    <section>
      <h2>{t("start.recent")}</h2>
      {#if recent.length || fallback.length}
        <div class="cards">{#each recent.length ? recent : fallback as c (c.id)}{@render card(c)}{/each}</div>
      {:else}
        <p class="muted">{t("start.noRecent")}</p>
      {/if}
    </section>

    <div class="actions">
      <button class="btn" onclick={() => sessions.openLocal().catch(toasts.error)}><Icon name="terminal" size={14} />{t("start.local")}</button>
      <button class="btn ghost" onclick={() => ui.edit(blankConnection(), true)}><Icon name="plus" size={14} />{t("sidebar.newConnection")}</button>
    </div>

    <dl class="keys">
      <div><dt>{t("start.hint.palette")}</dt><dd><span class="kbd">{mod}</span><span class="kbd">K</span></dd></div>
      <div><dt>{t("start.hint.ai")}</dt><dd><span class="kbd">{mod}</span><span class="kbd">J</span></dd></div>
      <div><dt>{t("start.hint.newTab")}</dt><dd><span class="kbd">{mod}</span><span class="kbd">T</span></dd></div>
      <div><dt>{t("start.hint.close")}</dt><dd><span class="kbd">{mod}</span><span class="kbd">W</span></dd></div>
    </dl>
  </div>
</div>

<style>
  .start {
    height: 100%;
    overflow: auto;
    display: grid;
    place-items: center;
    padding: 32px 20px;
  }
  .inner {
    width: min(620px, 100%);
  }
  h1 {
    margin: 0 0 6px;
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.02em;
  }
  .lede {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 0 0 32px;
    color: var(--muted);
  }
  .lede .kbd:first-of-type {
    margin-left: 4px;
  }
  section {
    margin-bottom: 24px;
  }
  h2 {
    margin: 0 0 10px;
    font-size: var(--fs-meta);
    font-weight: 500;
    color: var(--muted);
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 8px;
  }
  .card {
    position: relative;
    display: grid;
    gap: 2px;
    padding: 11px 12px 11px 16px;
    text-align: left;
    border-radius: var(--r-lg);
    border: 1px solid var(--border);
    background: var(--panel);
    transition: border-color var(--t-fast) var(--ease), background var(--t-fast) var(--ease);
  }
  .card:hover {
    border-color: var(--border-strong);
    background: var(--raised);
  }
  .env {
    position: absolute;
    left: 7px;
    top: 13px;
    width: 3px;
    height: 14px;
    border-radius: 2px;
    background: var(--border-strong);
  }
  .env-prod {
    background: var(--env-prod);
  }
  .env-staging {
    background: var(--env-staging);
  }
  .env-dev {
    background: var(--env-dev);
  }
  .cname {
    font-weight: 550;
    color: var(--text);
  }
  .chost {
    font-size: 11.5px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cenv {
    font-size: var(--fs-small);
    color: var(--faint);
    margin-top: 4px;
  }
  .muted {
    margin: 0;
    color: var(--faint);
  }
  .actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 36px;
  }
  .keys {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 8px 24px;
    margin: 0;
    padding-top: 18px;
    border-top: 1px solid var(--border);
  }
  .keys div {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  dt {
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  dd {
    display: flex;
    gap: 3px;
    margin: 0;
  }
</style>
