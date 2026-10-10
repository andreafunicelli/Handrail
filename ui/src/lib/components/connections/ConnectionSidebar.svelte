<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import ContextMenu, { type MenuEntry } from "../common/ContextMenu.svelte";
  import { t } from "../../i18n";
  import { connections, blankConnection } from "../../stores/connections.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { fuzzyFilter } from "../../util/fuzzy";
  import { parseQuickConnect } from "../../util/quickconnect";
  import { groupPath, type FolderNode } from "../../util/tree";
  import type { Connection, SessionState } from "../../api/types";

  let query = $state("");
  let quick = $state("");
  let quickError = $state(false);
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  let newMenu = $state<{ x: number; y: number } | null>(null);
  let renaming = $state<string | null>(null);
  let renameValue = $state("");
  let creatingFolder = $state<{ parent: string | null } | null>(null);
  let folderName = $state("");
  let dropTarget = $state<string | null>(null);

  const results = $derived(
    query.trim()
      ? fuzzyFilter(connections.list, query, (c) => [c.name, c.host, `${c.user}@${c.host}`, ...c.tags, groupPath(connections.groups, c.group_id)])
      : null,
  );

  /** Most relevant live state per connection (for the status dot). */
  const liveState = $derived.by(() => {
    const rank: SessionState[] = ["agent_running", "busy", "full_screen", "idle", "connecting", "reconnecting", "disconnected"];
    const m = new Map<string, SessionState>();
    for (const info of Object.values(sessions.infos)) {
      if (!info.connection_id) continue;
      const cur = m.get(info.connection_id);
      if (!cur || rank.indexOf(info.state) < rank.indexOf(cur)) m.set(info.connection_id, info.state);
    }
    return m;
  });

  const activeConnection = $derived(sessions.activeSession?.connection_id ?? null);

  async function connect(c: Connection, split?: "row") {
    ui.sidebarDrawer = false;
    try {
      await sessions.open(c.id, split);
    } catch (e) {
      toasts.error(e);
    }
  }

  async function submitQuick(e: SubmitEvent) {
    e.preventDefault();
    const target = parseQuickConnect(quick);
    if (!target) {
      quickError = true;
      return;
    }
    try {
      const c = await connections.quick(target);
      quick = "";
      await connect(c);
    } catch (err) {
      toasts.error(err);
    }
  }

  function connMenu(e: MouseEvent, c: Connection) {
    e.preventDefault();
    const pinned = connections.pinned.includes(c.id);
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: t("ctx.connect"), icon: "play", shortcut: "↵", action: () => connect(c) },
        { label: t("ctx.connectSplit"), icon: "splitRight", disabled: !sessions.activeTab, action: () => connect(c, "row") },
        "sep",
        { label: t("ctx.edit"), icon: "edit", action: () => ui.edit(c) },
        { label: t("ctx.duplicate"), icon: "copy", action: () => connections.duplicate(c.id, t("ctx.copySuffix")).catch(toasts.error) },
        { label: pinned ? t("ctx.unpin") : t("ctx.pin"), icon: "pin", action: () => connections.togglePin(c.id) },
        "sep",
        {
          label: t("ctx.delete"),
          icon: "trash",
          danger: true,
          action: async () => {
            if (!(await ui.ask(t("ctx.deleteConfirm", { name: c.name }), { confirmLabel: t("common.delete"), danger: true }))) return;
            await connections.remove(c.id).catch(toasts.error);
            toasts.info(t("ctx.deleted", { name: c.name }));
          },
        },
      ],
    };
  }

  function folderMenu(e: MouseEvent, f: FolderNode) {
    e.preventDefault();
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: t("ctx.newConnectionHere"), icon: "plus", action: () => ui.edit(blankConnection(f.group.id), true) },
        { label: t("sidebar.newFolder"), icon: "folderPlus", action: () => startFolder(f.group.id) },
        { label: t("ctx.rename"), icon: "edit", action: () => ((renaming = f.group.id), (renameValue = f.group.name)) },
        "sep",
        {
          label: t("ctx.delete"),
          icon: "trash",
          danger: true,
          action: async () => {
            if (!(await ui.ask(t("ctx.deleteConfirm", { name: f.group.name }), { confirmLabel: t("common.delete"), danger: true }))) return;
            await connections.removeGroup(f.group.id).catch(toasts.error);
          },
        },
      ],
    };
  }

  function startFolder(parent: string | null) {
    creatingFolder = { parent };
    folderName = "";
    if (parent && connections.collapsed[parent]) connections.toggleCollapsed(parent);
  }

  async function commitFolder() {
    const name = folderName.trim();
    const parent = creatingFolder?.parent ?? null;
    creatingFolder = null;
    if (name) await connections.newGroup(name, parent).catch(toasts.error);
  }

  async function commitRename(id: string) {
    const g = connections.groups.find((x) => x.id === id);
    const name = renameValue.trim();
    renaming = null;
    if (g && name && name !== g.name) await connections.saveGroup({ ...g, name }).catch(toasts.error);
  }

  // ---- drag & drop ----
  const DT_CONN = "application/x-aht-connection";
  const DT_GROUP = "application/x-aht-group";

  function dragStart(e: DragEvent, kind: "c" | "g", id: string) {
    e.dataTransfer?.setData(kind === "c" ? DT_CONN : DT_GROUP, id);
    e.dataTransfer?.setData("text/plain", id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }
  function over(e: DragEvent, key: string) {
    const types = e.dataTransfer?.types ?? [];
    if (!types.includes(DT_CONN) && !types.includes(DT_GROUP)) return;
    e.preventDefault();
    e.stopPropagation();
    dropTarget = key;
  }
  async function dropOnFolder(e: DragEvent, groupId: string | null) {
    e.preventDefault();
    e.stopPropagation();
    dropTarget = null;
    const c = e.dataTransfer?.getData(DT_CONN);
    const g = e.dataTransfer?.getData(DT_GROUP);
    if (c) await connections.moveConnection(c, groupId).catch(toasts.error);
    else if (g && g !== groupId) await connections.moveGroup(g, groupId).catch(toasts.error);
  }
  async function dropOnConnection(e: DragEvent, target: Connection) {
    e.preventDefault();
    e.stopPropagation();
    dropTarget = null;
    const c = e.dataTransfer?.getData(DT_CONN);
    if (c && c !== target.id) await connections.moveConnection(c, target.group_id, target.id).catch(toasts.error);
    const g = e.dataTransfer?.getData(DT_GROUP);
    if (g) await connections.moveGroup(g, target.group_id).catch(toasts.error);
  }

  function rowKey(e: KeyboardEvent, c: Connection) {
    if (e.key === "Enter") connect(c);
    if (e.key === "F2" || (e.key === "e" && (e.metaKey || e.ctrlKey))) ui.edit(c);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const rows = [...document.querySelectorAll<HTMLElement>(".sidebar [data-row]")];
      const i = rows.indexOf(e.currentTarget as HTMLElement);
      rows[i + (e.key === "ArrowDown" ? 1 : -1)]?.focus();
    }
    if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      connMenu(new MouseEvent("contextmenu", { clientX: r.left + 24, clientY: r.bottom }), c);
    }
  }
</script>

{#snippet row(c: Connection, depth: number, showPath = false)}
  {@const st = liveState.get(c.id)}
  <div
    class="row"
    class:active={activeConnection === c.id}
    class:drop={dropTarget === "c:" + c.id}
    data-row
    role="treeitem"
    aria-selected={activeConnection === c.id}
    tabindex="0"
    draggable={!showPath}
    style:--depth={depth}
    title="{c.user}@{c.host}:{c.port}"
    ondblclick={() => connect(c)}
    onkeydown={(e) => rowKey(e, c)}
    oncontextmenu={(e) => connMenu(e, c)}
    ondragstart={(e) => dragStart(e, "c", c.id)}
    ondragover={(e) => over(e, "c:" + c.id)}
    ondragleave={() => dropTarget === "c:" + c.id && (dropTarget = null)}
    ondrop={(e) => dropOnConnection(e, c)}
  >
    <span class="env env-{c.env}" aria-label={t(`env.${c.env}`)}></span>
    <span class="name">{c.name}</span>
    {#if showPath}
      <span class="meta">{groupPath(connections.groups, c.group_id) || c.host}</span>
    {:else}
      <span class="meta">{c.host}</span>
    {/if}
    {#if st}<span class="dot {st}" title={t(`state.${st}`)}></span>{/if}
  </div>
{/snippet}

{#snippet folder(f: FolderNode, depth: number)}
  {@const open = !connections.collapsed[f.group.id]}
  <div role="group">
    <div
      class="folder"
      class:drop={dropTarget === "g:" + f.group.id}
      role="treeitem"
      aria-expanded={open}
      aria-selected="false"
      tabindex="0"
      draggable={renaming !== f.group.id}
      style:--depth={depth}
      onclick={() => connections.toggleCollapsed(f.group.id)}
      onkeydown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          connections.toggleCollapsed(f.group.id);
        }
        if (e.key === "F2") ((renaming = f.group.id), (renameValue = f.group.name));
      }}
      oncontextmenu={(e) => folderMenu(e, f)}
      ondragstart={(e) => dragStart(e, "g", f.group.id)}
      ondragover={(e) => over(e, "g:" + f.group.id)}
      ondragleave={() => dropTarget === "g:" + f.group.id && (dropTarget = null)}
      ondrop={(e) => dropOnFolder(e, f.group.id)}
    >
      <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
      {#if renaming === f.group.id}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="rename"
          bind:value={renameValue}
          autofocus
          onclick={(e) => e.stopPropagation()}
          onkeydown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") commitRename(f.group.id);
            if (e.key === "Escape") renaming = null;
          }}
          onblur={() => commitRename(f.group.id)}
        />
      {:else}
        <span class="fname">{f.group.name}</span>
        <span class="count">{f.count}</span>
      {/if}
    </div>
    {#if open}
      {#if creatingFolder?.parent === f.group.id}{@render folderInput(depth + 1)}{/if}
      {#each f.folders as sub (sub.group.id)}{@render folder(sub, depth + 1)}{/each}
      {#each f.connections as c (c.id)}{@render row(c, depth + 1)}{/each}
    {/if}
  </div>
{/snippet}

{#snippet folderInput(depth: number)}
  <div class="folder" style:--depth={depth}>
    <Icon name="folder" size={12} />
    <!-- svelte-ignore a11y_autofocus -->
    <input
      class="rename"
      placeholder={t("sidebar.folderName")}
      bind:value={folderName}
      autofocus
      onkeydown={(e) => {
        if (e.key === "Enter") commitFolder();
        if (e.key === "Escape") creatingFolder = null;
      }}
      onblur={commitFolder}
    />
  </div>
{/snippet}

<aside class="sidebar" aria-label={t("rail.connections")}>
  <div class="head">
    <div class="search">
      <Icon name="search" size={14} />
      <input
        type="search"
        placeholder={t("sidebar.search")}
        bind:value={query}
        aria-label={t("sidebar.search")}
        onkeydown={(e) => {
          if (e.key === "Escape") query = "";
          if (e.key === "ArrowDown") {
            e.preventDefault();
            document.querySelector<HTMLElement>(".sidebar [data-row]")?.focus();
          }
        }}
      />
    </div>
    <button class="icon-btn" aria-label={t("sidebar.new")} aria-haspopup="menu" onclick={(e) => {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      newMenu = { x: r.left, y: r.bottom + 4 };
    }}><Icon name="plus" /></button>
  </div>

  <div class="kinds" role="tablist">
    <button role="tab" aria-selected="true" class="kind on">SSH</button>
    <button role="tab" aria-selected="false" class="kind" disabled>MySQL <span class="pill">{t("sidebar.soon")}</span></button>
    <button role="tab" aria-selected="false" class="kind" disabled>SFTP <span class="pill">{t("sidebar.soon")}</span></button>
  </div>

  <div
    class="tree"
    role="tree"
    tabindex="-1"
    aria-label={t("rail.connections")}
    class:drop={dropTarget === "root"}
    ondragover={(e) => over(e, "root")}
    ondragleave={(e) => e.target === e.currentTarget && (dropTarget = null)}
    ondrop={(e) => dropOnFolder(e, null)}
  >
    {#if results}
      {#each results as c (c.id)}{@render row(c, 0, true)}{/each}
      {#if !results.length}<p class="empty-line">{t("sidebar.noResults", { q: query })}</p>{/if}
    {:else if connections.loaded && !connections.list.length && !connections.groups.length}
      <div class="empty">
        <Icon name="server" size={20} />
        <strong>{t("sidebar.empty.title")}</strong>
        <p>{t("sidebar.empty.body")}</p>
        <div class="empty-actions">
          <button class="btn sm primary" onclick={() => ui.edit(blankConnection(), true)}>{t("sidebar.newConnection")}</button>
          <button class="btn sm" onclick={() => ui.openSettings("import")}>{t("sidebar.empty.import")}</button>
        </div>
      </div>
    {:else}
      {#if creatingFolder?.parent === null}{@render folderInput(0)}{/if}
      {#each connections.tree.folders as f (f.group.id)}{@render folder(f, 0)}{/each}
      {#if connections.tree.root.length}
        {#if connections.tree.folders.length}<div class="section">{t("sidebar.ungrouped")}</div>{/if}
        {#each connections.tree.root as c (c.id)}{@render row(c, 0)}{/each}
      {/if}
    {/if}
  </div>

  <form class="quick" onsubmit={submitQuick}>
    <Icon name="bolt" size={13} />
    <input
      class="mono"
      placeholder={t("sidebar.quick")}
      bind:value={quick}
      oninput={() => (quickError = false)}
      aria-label={t("sidebar.quickHint")}
      aria-invalid={quickError}
      spellcheck="false"
      autocomplete="off"
    />
    {#if quickError}<span class="qerr">{t("sidebar.quickInvalid")}</span>{:else if quick}<span class="kbd">↵</span>{/if}
  </form>
</aside>

{#if menu}<ContextMenu {...menu} onclose={() => (menu = null)} />{/if}
{#if newMenu}
  <ContextMenu
    {...newMenu}
    items={[
      { label: t("sidebar.newConnection"), icon: "server", action: () => ui.edit(blankConnection(), true) },
      { label: t("sidebar.newFolder"), icon: "folderPlus", action: () => startFolder(null) },
      "sep",
      { label: t("sidebar.empty.import"), icon: "import", action: () => ui.openSettings("import") },
    ]}
    onclose={() => (newMenu = null)}
  />
{/if}

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    background: var(--panel);
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
    transition: border-color var(--t-fast) var(--ease);
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
  .search input::placeholder {
    color: var(--faint);
  }
  .kinds {
    display: flex;
    gap: 2px;
    padding: 0 10px 8px;
    border-bottom: 1px solid var(--border);
  }
  .kind {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 8px;
    border-radius: var(--r-sm);
    font-size: var(--fs-meta);
    color: var(--muted);
  }
  .kind.on {
    background: var(--active);
    color: var(--text);
  }
  .kind:disabled {
    color: var(--faint);
  }
  .kind .pill {
    height: 15px;
    font-size: 9.5px;
    padding: 0 5px;
  }
  .tree {
    flex: 1;
    overflow: auto;
    padding: 6px 6px 12px;
  }
  .tree.drop {
    background: var(--accent-soft);
  }
  .folder,
  .row {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 8px 0 calc(8px + var(--depth, 0) * 14px);
    border-radius: var(--r);
    cursor: default;
    user-select: none;
    outline-offset: -2px;
  }
  .folder {
    color: var(--muted);
    font-size: var(--fs-meta);
    font-weight: 550;
  }
  .folder :global(svg) {
    flex: none;
    color: var(--faint);
  }
  .folder:hover,
  .row:hover {
    background: var(--hover);
  }
  .folder.drop,
  .row.drop {
    box-shadow: inset 0 0 0 1px var(--accent-line);
    background: var(--accent-soft);
  }
  .fname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
  }
  .count {
    font-size: var(--fs-small);
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .rename {
    flex: 1;
    min-width: 0;
    height: 22px;
    padding: 0 6px;
    border-radius: var(--r-sm);
    border: 1px solid var(--accent-line);
    background: var(--bg);
    outline: none;
    font-weight: 400;
    color: var(--text);
  }
  .row {
    padding-left: calc(22px + var(--depth, 0) * 14px);
  }
  .row.active {
    background: var(--active);
  }
  .row.active .name {
    color: var(--text);
  }
  .env {
    width: 3px;
    height: 12px;
    border-radius: 2px;
    flex: none;
    margin-left: -10px;
    background: transparent;
  }
  .env-prod {
    background: var(--env-prod);
  }
  .env-staging {
    background: var(--env-staging);
  }
  .env-dev {
    background: var(--env-dev);
    opacity: 0.7;
  }
  .name {
    flex: none;
    max-width: 60%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
  }
  .meta {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-small);
    color: var(--faint);
  }
  .section {
    padding: 12px 8px 4px;
    font-size: var(--fs-small);
    color: var(--faint);
  }
  .empty-line {
    padding: 10px 8px;
    margin: 0;
    color: var(--faint);
    font-size: var(--fs-meta);
  }
  .empty {
    display: grid;
    justify-items: start;
    gap: 6px;
    padding: 18px 10px;
    color: var(--muted);
  }
  .empty strong {
    color: var(--text);
    font-weight: 550;
  }
  .empty p {
    margin: 0 0 6px;
    font-size: var(--fs-meta);
  }
  .empty-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .quick {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 38px;
    padding: 0 12px;
    border-top: 1px solid var(--border);
    color: var(--faint);
  }
  .quick:focus-within {
    color: var(--accent);
  }
  .quick input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
    font-size: 12px;
    color: var(--text);
  }
  .quick input::placeholder {
    color: var(--faint);
  }
  .qerr {
    font-size: var(--fs-small);
    color: var(--danger);
    white-space: nowrap;
  }
</style>
