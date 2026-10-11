<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import ContextMenu, { type MenuEntry } from "../common/ContextMenu.svelte";
  import { t } from "../../i18n";
  import type { Snippet } from "../../api/types";
  import { blankSnippet, snippets } from "../../stores/snippets.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { fuzzyFilter } from "../../util/fuzzy";
  import type { SuggestedSnippet } from "../../util/snippets";

  let query = $state("");
  let showSuggested = $state(true);
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);

  const keys = (s: SuggestedSnippet) => [s.name, s.command, s.description, ...s.tags];
  const saved = $derived(query.trim() ? fuzzyFilter(snippets.list, query, keys) : snippets.sorted);
  const suggested = $derived(query.trim() ? fuzzyFilter(snippets.suggestions, query, keys) : snippets.suggestions);
  const canRun = $derived(!!sessions.activeSessionId);

  const use = (s: Snippet | SuggestedSnippet, run: boolean) => snippets.use(s, run).catch(toasts.error);

  async function remove(s: Snippet) {
    const ok = await ui.ask(t("snip.deleteTitle", { name: s.name }), { confirmLabel: t("common.delete"), danger: true });
    if (ok) await snippets.remove(s.id).catch(toasts.error);
  }

  function openMenu(e: MouseEvent, s: Snippet) {
    e.preventDefault();
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: t("snip.run"), icon: "play", disabled: !canRun, action: () => use(s, true) },
        { label: t("snip.insert"), icon: "terminal", disabled: !canRun, action: () => use(s, false) },
        "sep",
        { label: t("common.edit"), icon: "edit", action: () => snippets.edit(s) },
        { label: t("common.delete"), icon: "trash", danger: true, action: () => remove(s) },
      ],
    };
  }

  async function add(s: SuggestedSnippet) {
    try {
      await snippets.add(s);
      toasts.success(t("snip.added", { name: s.name }));
    } catch (e) {
      toasts.error(e);
    }
  }
</script>

<aside class="panel" aria-label={t("rail.snippets")}>
  <div class="head">
    <div class="search">
      <Icon name="search" size={14} />
      <input type="search" placeholder={t("snip.search")} bind:value={query} aria-label={t("snip.search")} onkeydown={(e) => e.key === "Escape" && (query = "")} />
    </div>
    <button class="icon-btn" title={t("snip.new")} aria-label={t("snip.new")} onclick={() => snippets.edit(blankSnippet(), true)}><Icon name="plus" /></button>
  </div>

  {#if !canRun}<p class="note">{t("snip.noTerminal")}</p>{/if}

  <div class="scroll">
    {#each saved as s (s.id)}
      <div class="item" role="listitem" oncontextmenu={(e) => openMenu(e, s)}>
        <div class="text">
          <strong>{s.name}</strong>
          <code class="mono" title={s.command}>{s.command}</code>
        </div>
        <div class="acts">
          <button class="icon-btn" title={t("snip.run")} aria-label={`${t("snip.run")}: ${s.name}`} disabled={!canRun} onclick={() => use(s, true)}><Icon name="play" size={14} /></button>
          <button class="icon-btn" title={t("snip.more")} aria-label={`${t("snip.more")}: ${s.name}`} onclick={(e) => openMenu(e, s)}><Icon name="more" size={14} /></button>
        </div>
      </div>
    {:else}
      {#if !query.trim()}
        <div class="empty">
          <strong>{t("snip.emptyTitle")}</strong>
          <p>{t("snip.emptyBody")}</p>
        </div>
      {:else}
        <p class="empty-line">{t("snip.noMatch")}</p>
      {/if}
    {/each}

    {#if suggested.length}
      <button class="section" aria-expanded={showSuggested} onclick={() => (showSuggested = !showSuggested)}>
        <Icon name={showSuggested ? "chevronDown" : "chevronRight"} size={12} />{t("snip.suggested")}<span class="count">{suggested.length}</span>
      </button>
      {#if showSuggested}
        {#each suggested as s (s.name)}
          <div class="item suggested" role="listitem">
            <div class="text">
              <strong>{s.name}</strong>
              <span class="desc">{s.description}</span>
              <code class="mono" title={s.command}>{s.command}</code>
            </div>
            <div class="acts">
              <button class="icon-btn" title={t("snip.run")} aria-label={`${t("snip.run")}: ${s.name}`} disabled={!canRun} onclick={() => use(s, true)}><Icon name="play" size={14} /></button>
              <button class="icon-btn" title={t("snip.add")} aria-label={`${t("snip.add")}: ${s.name}`} onclick={() => add(s)}><Icon name="plus" size={14} /></button>
            </div>
          </div>
        {/each}
      {/if}
    {/if}
  </div>
</aside>

{#if menu}<ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}

<style>
  .panel {
    min-width: 0;
    overflow: hidden;
    background: var(--panel);
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 8px 6px 10px;
    height: var(--header-h);
    box-sizing: content-box;
  }
  .search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 8px;
    border-radius: var(--r);
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--faint);
  }
  .search:focus-within {
    border-color: var(--accent-line);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
  }
  .note {
    margin: 0 10px 6px;
    font-size: var(--fs-meta);
    color: var(--faint);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0 6px 10px;
  }
  .item {
    display: flex;
    align-items: flex-start;
    gap: 4px;
    padding: 7px 6px 7px 8px;
    border-radius: var(--r);
  }
  .item:hover {
    background: var(--hover);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 2px;
  }
  .text strong {
    font-weight: 550;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .text code,
  .desc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-small);
    color: var(--faint);
  }
  .desc {
    color: var(--muted);
  }
  .acts {
    display: flex;
    gap: 2px;
    opacity: 0.55;
  }
  .item:hover .acts,
  .item:focus-within .acts {
    opacity: 1;
  }
  .section {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    padding: 14px 8px 6px;
    border: 0;
    background: none;
    color: var(--faint);
    font-size: var(--fs-small);
    text-align: left;
  }
  .count {
    margin-left: auto;
  }
  .empty {
    display: grid;
    gap: 6px;
    padding: 14px 8px;
    color: var(--muted);
  }
  .empty strong {
    color: var(--text);
    font-weight: 550;
  }
  .empty p,
  .empty-line {
    margin: 0;
    font-size: var(--fs-meta);
    color: var(--faint);
  }
  .empty-line {
    padding: 10px 8px;
  }
</style>
