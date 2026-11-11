<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { fmtDateTime, t } from "../../i18n";
  import { connections, blankConnection, blankDbConnection } from "../../stores/connections.svelte";
  import { sessions, type Workspace } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { relativeTime } from "../../util/time";
  import type { Connection } from "../../api/types";

  /** The home screen: resume the last workspace, favorites, recent activity and quick actions.
   *  Nothing connects until the user picks something. */
  const mod = typeof navigator !== "undefined" && /Mac|iP/.test(navigator.platform) ? "⌘" : "Ctrl";
  const pinned = $derived(connections.pinned.map((id) => connections.byId.get(id)).filter((c): c is Connection => !!c));
  const recent = $derived(
    connections.recent
      .filter((id) => !connections.pinned.includes(id))
      .map((id) => connections.byId.get(id))
      .filter((c): c is Connection => !!c)
      .slice(0, 6),
  );
  const last = $derived(sessions.lastWorkspace);
  const lastSummary = $derived(last ? sessions.summarize(last) : null);
  const otherWorkspaces = $derived(
    sessions.workspaces
      .filter((w) => w.id !== last?.id)
      .sort((a, b) => Math.max(b.openedAt ?? 0, b.savedAt ?? 0) - Math.max(a.openedAt ?? 0, a.savedAt ?? 0))
      .slice(0, 4),
  );
  const firstRun = $derived(connections.loaded && !connections.list.length && !sessions.workspaces.length);

  const open = (c: Connection) => sessions.open(c.id).catch(toasts.error);
  const resume = (w: Workspace) => sessions.openWorkspace(w.id).catch(toasts.error);

  function summaryText(w: Workspace): string {
    const s = sessions.summarize(w);
    const parts: string[] = [];
    if (s.ssh) parts.push(t("start.ws.ssh", { n: s.ssh }));
    if (s.db) parts.push(t("start.ws.db", { n: s.db }));
    if (s.rdp) parts.push(t("start.ws.rdp", { n: s.rdp }));
    if (s.local) parts.push(t("start.ws.local", { n: s.local }));
    parts.push(s.tabs === 1 ? t("start.ws.tab") : t("start.ws.tabs", { n: s.tabs }));
    return parts.join(" · ");
  }
  const when = (w: Workspace) => {
    const at = Math.max(w.openedAt ?? 0, w.savedAt ?? 0);
    return at ? relativeTime(at) : "";
  };
</script>

{#snippet card(c: Connection)}
  <button class="card" onclick={() => open(c)} title={t("start.connectTo", { name: c.name })}>
    <span class="env env-{c.env}"></span>
    <span class="cname">{c.name}{#if c.env === "prod"}<span class="env-prod-badge">PROD</span>{/if}</span>
    <span class="chost mono">{c.user}@{c.host}{c.port !== 22 ? `:${c.port}` : ""}</span>
    {#if c.env !== "none" && c.env !== "prod"}<span class="cenv">{t(`env.${c.env}`)}</span>{/if}
  </button>
{/snippet}

{#snippet action(icon: Parameters<typeof Icon>[1]["name"], label: string, run: () => void)}
  <button class="qa" onclick={run}><Icon name={icon} size={14} /><span>{label}</span></button>
{/snippet}

<div class="start">
  <div class="inner">
    {#if firstRun}
      <h1>{t("start.first.title")}</h1>
      <p class="lede">{t("start.first.body")}</p>
      <div class="paths">
        <div class="path">
          <Icon name="import" size={16} />
          <strong>{t("start.first.importTitle")}</strong>
          <p>{t("start.first.importBody")}</p>
          <button class="btn primary sm" onclick={() => ui.openSettings("import")}>{t("sidebar.empty.import")}</button>
        </div>
        <div class="path">
          <Icon name="server" size={16} />
          <strong>{t("start.first.addTitle")}</strong>
          <p>{t("start.first.addBody")}</p>
          <div class="row">
            <button class="btn sm" onclick={() => ui.edit(blankConnection(), true)}>{t("start.qa.ssh")}</button>
            <button class="btn sm ghost" onclick={() => sessions.openLocal().catch(toasts.error)}>{t("start.local")}</button>
          </div>
        </div>
        <div class="path">
          <Icon name="sparkle" size={16} />
          <strong>{t("start.first.aiTitle")}</strong>
          <p>{t("start.first.aiBody")}</p>
          <div class="row">
            <button class="btn sm" onclick={() => ui.openSettings("providers")} title={t("start.first.builtinHint")}>{t("start.first.builtin")}</button>
            <button class="btn sm" onclick={() => ui.openSettings("harnesses")} title={t("start.first.cliHint")}>{t("start.first.cli")}</button>
          </div>
          <p class="fine">{t("start.first.privacy")}</p>
        </div>
      </div>
    {:else}
      <h1>{t("start.title")}</h1>
      <p class="lede">
        {t("start.body").split("{kbd}")[0]}<span class="kbd">{mod}</span><span class="kbd">K</span>{t("start.body").split("{kbd}")[1] ?? ""}
      </p>

      <section aria-labelledby="start-resume">
        <h2 id="start-resume">{t("start.resume")}</h2>
        {#if last && lastSummary}
          <div class="resume">
            <div class="rinfo">
              <strong class="rname">{last.name}</strong>
              <span class="rsum">{summaryText(last)}</span>
              {#if lastSummary.missing}<span class="rwarn">{t("start.ws.missing", { n: lastSummary.missing })}</span>{/if}
              {#if when(last)}<span class="rwhen" title={fmtDateTime(Math.max(last.openedAt ?? 0, last.savedAt ?? 0))}>{when(last)}</span>{/if}
            </div>
            <button class="btn primary" onclick={() => resume(last)}><Icon name="play" size={13} />{t("start.resumeAction")}</button>
          </div>
          {#if otherWorkspaces.length}
            <ul class="wslist">
              {#each otherWorkspaces as w (w.id)}
                <li>
                  <button class="ws" onclick={() => resume(w)}>
                    <Icon name="layout" size={13} />
                    <span class="wname">{w.name}</span>
                    <span class="wsum">{summaryText(w)}</span>
                    {#if when(w)}<span class="wwhen">{when(w)}</span>{/if}
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        {:else}
          <p class="muted">{t("start.noWorkspace")}</p>
        {/if}
      </section>

      {#if pinned.length}
        <section aria-labelledby="start-fav">
          <h2 id="start-fav">{t("sidebar.favorites")}</h2>
          <div class="cards">{#each pinned as c (c.id)}{@render card(c)}{/each}</div>
        </section>
      {/if}

      <section aria-labelledby="start-recent">
        <h2 id="start-recent">{t("start.recentActivity")}</h2>
        {#if recent.length}
          <div class="cards">{#each recent as c (c.id)}{@render card(c)}{/each}</div>
        {:else}
          <p class="muted">{t("start.noRecent")}</p>
        {/if}
      </section>
    {/if}

    <section aria-labelledby="start-qa">
      <h2 id="start-qa">{t("start.quickActions")}</h2>
      <div class="qas">
        {@render action("server", t("start.qa.ssh"), () => ui.edit(blankConnection(), true))}
        {@render action("terminal", t("start.local"), () => void sessions.openLocal().catch(toasts.error))}
        {@render action("database", t("start.qa.db"), () => ui.edit(blankDbConnection(null), true))}
        {@render action("layout", t("start.qa.workspace"), () => (ui.paletteOpen = true))}
        {@render action("import", t("sidebar.empty.import"), () => ui.openSettings("import"))}
        {@render action("sparkle", t("start.qa.ai"), () => ui.openSettings("providers"))}
      </div>
    </section>

    <dl class="keys">
      <div><dt>{t("start.hint.palette")}</dt><dd><span class="kbd">{mod}</span><span class="kbd">K</span></dd></div>
      <div><dt>{t("start.hint.ai")}</dt><dd><span class="kbd">{mod}</span><span class="kbd">J</span></dd></div>
      <div><dt>{t("start.hint.newTab")}</dt><dd><span class="kbd">{mod}</span><span class="kbd">⇧</span><span class="kbd">T</span></dd></div>
      <div><dt>{t("start.hint.close")}</dt><dd><span class="kbd">{mod}</span><span class="kbd">⇧</span><span class="kbd">W</span></dd></div>
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
    width: min(720px, 100%);
  }
  h1 {
    margin: 0 0 6px;
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.02em;
  }
  .lede {
    margin: 0 0 32px;
    color: var(--muted);
    line-height: 1.8;
  }
  .lede .kbd {
    margin: 0 2px;
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
    font-size: var(--fs-meta);
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
    color: var(--muted);
  }
  .resume {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-lg);
    background: var(--panel);
  }
  .rinfo {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .rname {
    font-size: var(--fs-heading);
  }
  .rsum,
  .rwhen,
  .wsum,
  .wwhen {
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  .rwarn {
    color: var(--warn);
    font-size: var(--fs-meta);
  }
  .wslist {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: grid;
    gap: 2px;
  }
  .ws {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    border-radius: var(--r);
    text-align: left;
    color: var(--text-2);
  }
  .ws:hover {
    background: var(--hover);
    color: var(--text);
  }
  .wname {
    font-weight: 550;
  }
  .wsum {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .qas {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: 6px;
  }
  .qa {
    display: flex;
    align-items: center;
    gap: 8px;
    height: var(--ctl-h);
    padding: 0 var(--sp-3);
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--panel);
    color: var(--text-2);
    text-align: left;
  }
  .qa:hover {
    background: var(--raised);
    border-color: var(--border-strong);
    color: var(--text);
  }
  .paths {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 10px;
    margin-bottom: 32px;
  }
  .path {
    display: grid;
    align-content: start;
    gap: 6px;
    padding: var(--sp-4);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--panel);
  }
  .path :global(svg) {
    color: var(--accent-text);
  }
  .path p {
    margin: 0;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  .path .fine {
    font-size: var(--fs-small);
    color: var(--faint);
  }
  .row {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .cname .env-prod-badge {
    margin-left: 6px;
    vertical-align: 1px;
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
