<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import ContextMenu, { type MenuEntry } from "../common/ContextMenu.svelte";
  import { t } from "../../i18n";
  import { sessions, type Tab } from "../../stores/sessions.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { leaves } from "../../util/layout";

  const DT_TAB = "application/x-aht-tab";
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  let strip: HTMLDivElement;

  const mod = typeof navigator !== "undefined" && /Mac|iP/.test(navigator.platform) ? "⌘" : "Ctrl+";

  function tabMenu(e: MouseEvent, tab: Tab) {
    e.preventDefault();
    const multi = leaves(tab.layout).length > 1;
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: t("tabs.splitRight"), icon: "splitRight", action: () => splitWith(tab, "row") },
        { label: t("tabs.splitDown"), icon: "splitDown", action: () => splitWith(tab, "col") },
        ...(multi ? [{ label: t("tabs.closePane"), icon: "x" as const, action: () => sessions.closePane(tab.id, tab.activeLeaf) }] : []),
        "sep",
        { label: t("tabs.close"), icon: "x", shortcut: `${mod}⇧W`, action: () => sessions.closeTab(tab.id) },
      ],
    };
  }

  async function splitWith(tab: Tab, dir: "row" | "col") {
    sessions.focus(tab.id);
    const info = sessions.tabInfo(tab);
    try {
      if (info?.connection_id) await sessions.open(info.connection_id, dir);
      else await sessions.openLocal(null, dir);
    } catch (e) {
      toasts.error(e);
    }
  }

  function workspacesMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const items: MenuEntry[] = sessions.workspaces.length
      ? sessions.workspaces.map((w) => ({ label: w.name, icon: "layout" as const, action: () => sessions.openWorkspace(w.id).catch(toasts.error) }))
      : [{ label: t("tabs.noWorkspaces"), disabled: true, action: () => {} }];
    items.push("sep", { label: t("tabs.saveWorkspace"), icon: "plus", disabled: !sessions.tabs.length, action: () => (ui.saveWorkspaceOpen = true) });
    menu = { x: r.right - 200, y: r.bottom + 4, items };
  }

  function tabKey(e: KeyboardEvent, tab: Tab, i: number) {
    if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
      const next = sessions.tabs[(i + (e.key === "ArrowRight" ? 1 : -1) + sessions.tabs.length) % sessions.tabs.length];
      sessions.focus(next.id);
      strip.querySelector<HTMLElement>(`[data-tab="${next.id}"]`)?.focus();
    }
    if (e.key === "Delete") sessions.closeTab(tab.id);
  }

  $effect(() => {
    const id = sessions.activeTabId;
    if (id) strip?.querySelector(`[data-tab="${id}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });
</script>

<div class="bar">
  {#if ui.narrow}
    <button class="icon-btn" aria-label={t("rail.connections")} onclick={() => (ui.sidebarDrawer = true)}><Icon name="menu" /></button>
  {/if}
  <div class="tabs" role="tablist" bind:this={strip}>
    {#each sessions.tabs as tab, i (tab.id)}
      {@const info = sessions.tabInfo(tab)}
      {@const active = sessions.activeTabId === tab.id}
      <div
        class="tab env-{info?.env ?? 'none'}"
        class:active
        role="tab"
        tabindex={active ? 0 : -1}
        aria-selected={active}
        data-tab={tab.id}
        draggable="true"
        title={info ? `${info.user ?? ""}@${info.host ?? ""}` : ""}
        onclick={() => sessions.focus(tab.id)}
        onauxclick={(e) => e.button === 1 && sessions.closeTab(tab.id)}
        oncontextmenu={(e) => tabMenu(e, tab)}
        onkeydown={(e) => tabKey(e, tab, i)}
        ondragstart={(e) => {
          e.dataTransfer?.setData(DT_TAB, tab.id);
          if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
        }}
      >
        <span class="dot {info?.state ?? ''}"></span>
        <span class="title">{sessions.tabTitle(tab)}</span>
        <button
          class="close"
          tabindex="-1"
          aria-label={t("tabs.close")}
          onclick={(e) => {
            e.stopPropagation();
            sessions.closeTab(tab.id);
          }}><Icon name="x" size={12} /></button>
      </div>
    {/each}
    <button class="icon-btn add" title="{t('tabs.new')} ({mod}⇧T)" aria-label={t("tabs.new")} onclick={() => sessions.openLocal().catch(toasts.error)}>
      <Icon name="plus" size={15} />
    </button>
  </div>
  <div class="tools">
    {#if sessions.activeTab}
      <button class="icon-btn" title={t("tabs.splitRight")} aria-label={t("tabs.splitRight")} onclick={() => splitWith(sessions.activeTab!, "row")}><Icon name="splitRight" /></button>
      <button class="icon-btn" title={t("tabs.splitDown")} aria-label={t("tabs.splitDown")} onclick={() => splitWith(sessions.activeTab!, "col")}><Icon name="splitDown" /></button>
    {/if}
    <button class="icon-btn" title={t("tabs.workspaces")} aria-label={t("tabs.workspaces")} aria-haspopup="menu" onclick={workspacesMenu}><Icon name="layout" /></button>
    <button
      class="icon-btn"
      class:on={ui.narrow ? ui.aiDrawer : settings.aiOpen}
      title="{t('rail.ai')} ({mod}J)"
      aria-label={t("rail.ai")}
      aria-pressed={ui.narrow ? ui.aiDrawer : settings.aiOpen}
      onclick={() => (ui.narrow ? (ui.aiDrawer = !ui.aiDrawer) : (settings.aiOpen = !settings.aiOpen))}
    ><Icon name="sparkle" /></button>
  </div>
</div>

{#if menu}<ContextMenu {...menu} onclose={() => (menu = null)} />{/if}

<style>
  .bar {
    display: flex;
    align-items: stretch;
    height: var(--header-h);
    border-bottom: 1px solid var(--border);
    background: var(--panel);
    padding-left: 4px;
    min-width: 0;
  }
  .bar > .icon-btn {
    align-self: center;
    margin-right: 2px;
  }
  .tabs {
    flex: 1;
    display: flex;
    align-items: stretch;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabs::-webkit-scrollbar {
    display: none;
  }
  .tab {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 0 1 200px;
    min-width: 120px;
    padding: 0 6px 0 12px;
    color: var(--muted);
    border-right: 1px solid var(--border);
    cursor: default;
    user-select: none;
    outline-offset: -2px;
  }
  .tab::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 2px;
    background: transparent;
  }
  .tab.env-prod::before {
    background: var(--env-prod);
  }
  .tab.env-staging::before {
    background: var(--env-staging);
  }
  .tab.env-dev::before {
    background: var(--env-dev);
    opacity: 0.6;
  }
  .tab:hover {
    color: var(--text-2);
    background: var(--hover);
  }
  .tab.active {
    background: var(--bg);
    color: var(--text);
  }
  .tab.active::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: -1px;
    height: 1px;
    background: var(--bg);
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: var(--r-sm);
    color: var(--faint);
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease);
  }
  .tab:hover .close,
  .tab.active .close {
    opacity: 1;
  }
  .close:hover {
    background: var(--active);
    color: var(--text);
  }
  .add {
    align-self: center;
    margin-left: 4px;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 8px 0 6px;
  }
  @media (hover: none) {
    .close {
      opacity: 1;
    }
  }
</style>
