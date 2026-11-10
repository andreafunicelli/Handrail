<script lang="ts">
  import ConnIcon from "../common/ConnIcon.svelte";
  import { segmented } from "../../actions/segmented";
  import Icon from "../icons/Icon.svelte";
  import ContextMenu, { type MenuEntry } from "../common/ContextMenu.svelte";
  import { t } from "../../i18n";
  import { connections, blankConnection, blankDbConnection, isDbConnection } from "../../stores/connections.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { fuzzyFilter } from "../../util/fuzzy";
  import { parseQuickConnect } from "../../util/quickconnect";
  import { buildTree, groupPath, type FolderNode } from "../../util/tree";
  import type { Connection, S3Account, SessionState } from "../../api/types";
  import { blankS3Account, s3 } from "../../stores/s3.svelte";
  import { s3Where } from "../../util/s3";
  import { filterConnections, parseQuery, removeToken, type QueryContext } from "../../util/connQuery";
  import { load, save } from "../../util/storage";
  import type { OpenPane } from "../../stores/sessions.svelte";

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
  /** Which list the sidebar shows. */
  let kind = $state<"ssh" | "db" | "s3">("ssh");
  /** Databases are listed under their own tab; SSH and RDP servers under Servers. */
  const inTab = (c: Connection) => (kind === "db" ? isDbConnection(c) : !isDbConnection(c));
  const blankForTab = (groupId: string | null = null) => (kind === "db" ? blankDbConnection(groupId) : blankConnection(groupId));
  /** The folder tree of this tab's connections. Folders are shared: one holding only the other
   *  tab's connections is left out, an empty one shows in both. */
  const tabTree = $derived.by(() => {
    const t = buildTree(connections.groups, connections.list.filter(inTab), connections.order);
    const total = new Map<string, number>();
    const walk = (fs: FolderNode[]) => fs.forEach((f) => (total.set(f.group.id, f.count), walk(f.folders)));
    walk(connections.tree.folders);
    const prune = (fs: FolderNode[]): FolderNode[] =>
      fs.filter((f) => f.count > 0 || !total.get(f.group.id)).map((f) => ({ ...f, folders: prune(f.folders) }));
    return { folders: prune(t.folders), root: t.root };
  });
  // A connection just saved of the other kind (a new one, an imported one, or one edited into
  // the other kind): show the tab that lists it.
  let known: Map<string, boolean> | null = null;
  $effect(() => {
    const list = connections.list;
    if (known && kind !== "s3") {
      const moved = list.find((c) => known!.get(c.id) !== isDbConnection(c));
      if (moved && !inTab(moved)) kind = isDbConnection(moved) ? "db" : "ssh";
    }
    if (connections.loaded) known = new Map(list.map((c) => [c.id, isDbConnection(c)]));
  });
  function newDatabase(groupId: string | null = null) {
    kind = "db";
    ui.edit(blankDbConnection(groupId), true);
  }

  const s3Results = $derived(query.trim() ? fuzzyFilter(s3.list, query, (a) => [a.name, a.endpoint, a.bucket ?? "", a.region]) : s3.sorted);

  function s3Menu(e: MouseEvent, a: S3Account) {
    e.preventDefault();
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: t("s3.open"), icon: "folder", action: () => sessions.openS3(a.id) },
        { label: t("common.edit"), icon: "edit", action: () => s3.edit(a) },
        "sep",
        { label: t("common.delete"), icon: "trash", danger: true, action: () => removeS3(a) },
      ],
    };
  }

  async function removeS3(a: S3Account) {
    const ok = await ui.ask(t("s3.deleteTitle", { name: a.name }), { body: t("s3.deleteBody"), confirmLabel: t("common.delete"), danger: true });
    if (ok) await s3.remove(a.id).catch(toasts.error);
  }

  /** The search: filter words (`env:prod type:ssh tag:web host:10.200 group:x is:active`)
   *  plus free text, matched fuzzily. */
  const parsed = $derived(parseQuery(query));
  const queryCtx = $derived<QueryContext>({
    groupPath: (c) => groupPath(connections.groups, c.group_id),
    active: sessions.activeConnections,
    pinned: new Set(connections.pinned),
  });
  /** A type filter looks in both tabs (`type:mysql` from Servers). */
  const pool = $derived(parsed.tokens.some((t) => t.key === "type") ? connections.list : connections.list.filter(inTab));
  /** Free text: a flat list, best match first. */
  const results = $derived(parsed.text ? filterConnections(pool, parsed, queryCtx) : null);
  /** Filters only: the folder tree, holding just what matches. */
  const filteredTree = $derived.by(() => {
    if (parsed.text || !parsed.tokens.length) return null;
    const keep = filterConnections(pool, parsed, queryCtx);
    const tree = buildTree(connections.groups, keep, connections.order);
    const prune = (fs: FolderNode[]): FolderNode[] => fs.filter((f) => f.count > 0).map((f) => ({ ...f, folders: prune(f.folders) }));
    return { folders: prune(tree.folders), root: tree.root, total: keep.length };
  });
  /** Filter menu entries: a word added to the search. */
  const QUICK_FILTERS: { label: () => string; token: string }[] = [
    { label: () => t("filter.prod"), token: "env:prod" },
    { label: () => t("filter.staging"), token: "env:staging" },
    { label: () => t("filter.dev"), token: "env:dev" },
    { label: () => "SSH", token: "type:ssh" },
    { label: () => "RDP", token: "type:rdp" },
    { label: () => t("filter.databases"), token: "type:db" },
    { label: () => t("filter.active"), token: "is:active" },
    { label: () => t("filter.favorites"), token: "is:fav" },
  ];
  function addFilter(token: string) {
    if (!query.split(/\s+/).includes(token)) query = `${query.trim()} ${token}`.trim();
  }
  function filterMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = {
      x: r.left,
      y: r.bottom + 4,
      items: [
        ...QUICK_FILTERS.map((f) => ({ label: f.label(), icon: "search" as const, action: () => addFilter(f.token) })),
        "sep" as const,
        { label: t("filter.syntax"), icon: "info" as const, action: () => toasts.info(t("filter.syntaxHelp")) },
      ],
    };
  }

  // ---- active sessions, favorites, recent ----
  type Sections = { active: boolean; fav: boolean; recent: boolean };
  let sections = $state<Sections>({ active: true, fav: true, recent: true, ...load<Partial<Sections>>("sidebarSections", {}) });
  function toggleSection(k: keyof Sections) {
    sections[k] = !sections[k];
    save("sidebarSections", $state.snapshot(sections));
  }
  const byId = $derived(new Map(connections.list.map((c) => [c.id, c])));
  const favorites = $derived(connections.pinned.map((id) => byId.get(id)).filter((c): c is Connection => !!c && inTab(c)));
  const recents = $derived(
    connections.recent
      .filter((id) => !connections.pinned.includes(id))
      .map((id) => byId.get(id))
      .filter((c): c is Connection => !!c && inTab(c))
      .slice(0, 5),
  );
  /** Open panes with their connection, numbered when one connection is open more than once. */
  const activePanes = $derived.by(() => {
    const seen = new Map<string, number>();
    const out: { pane: OpenPane; c: Connection; n: number }[] = [];
    for (const pane of sessions.openPanes) {
      const c = byId.get(pane.connectionId);
      if (!c) continue;
      const n = (seen.get(c.id) ?? 0) + 1;
      seen.set(c.id, n);
      out.push({ pane, c, n });
    }
    return out;
  });
  const openCount = $derived(new Map(activePanes.map((a) => [a.c.id, activePanes.filter((x) => x.c.id === a.c.id).length])));

  /** Connect, or focus the pane already open on it (the most recent when several are). */
  function openOrFocus(c: Connection) {
    const panes = sessions.openPanes.filter((p) => p.connectionId === c.id);
    const last = panes[panes.length - 1];
    if (last) {
      ui.sidebarDrawer = false;
      sessions.focus(last.tabId, last.leafId);
    } else void connect(c);
  }

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
        ...(c.kind === "ssh"
          ? [{ label: t("ctx.newDatabaseVia"), icon: "database" as const, action: () => ((kind = "db"), ui.edit(blankDbConnection(c.group_id, c), true)) }]
          : []),
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
        { label: t(kind === "db" ? "ctx.newDatabaseHere" : "ctx.newConnectionHere"), icon: "plus", action: () => ui.edit(blankForTab(f.group.id), true) },
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
    // The name field lives in the folder tree, which a search or the empty state replaces.
    query = "";
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
    if (e.key === "Enter") openOrFocus(c);
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
    ondblclick={() => openOrFocus(c)}
    onclick={() => ui.narrow && openOrFocus(c)}
    onkeydown={(e) => rowKey(e, c)}
    oncontextmenu={(e) => connMenu(e, c)}
    ondragstart={(e) => dragStart(e, "c", c.id)}
    ondragover={(e) => over(e, "c:" + c.id)}
    ondragleave={() => dropTarget === "c:" + c.id && (dropTarget = null)}
    ondrop={(e) => dropOnConnection(e, c)}
  >
    <span class="env env-{c.env}" aria-label={t(`env.${c.env}`)}></span>
    <ConnIcon icon={c.icon} />
    <span class="name">{c.name}</span>
    {#if c.kind === "rdp"}<span class="kindic" title={t("editor.kind.rdp")}><Icon name="monitor" size={12} /></span>{/if}
    {#if c.kind === "mysql" || c.kind === "postgres"}<span class="kindic" title={c.kind === "mysql" ? "MySQL" : "PostgreSQL"}><Icon name="database" size={12} /></span>{/if}
    {#if showPath}
      <span class="meta">{groupPath(connections.groups, c.group_id) || c.host}</span>
    {:else}
      <span class="meta">{c.host}</span>
    {/if}
    {#if st}<span class="dot {st}" title={t(`state.${st}`)}></span>{/if}
    <span class="acts">
      <button
        class="act"
        tabindex="-1"
        title={sessions.activeConnections.has(c.id) ? t("sidebar.focusOpen") : t("ctx.connect")}
        aria-label={sessions.activeConnections.has(c.id) ? t("sidebar.focusOpen") : t("ctx.connect")}
        onclick={(e) => (e.stopPropagation(), openOrFocus(c))}
        ondblclick={(e) => e.stopPropagation()}><Icon name={sessions.activeConnections.has(c.id) ? "jump" : "play"} size={11} /></button>
      <button
        class="act"
        tabindex="-1"
        title={t("sidebar.more")}
        aria-label={t("sidebar.more")}
        onclick={(e) => (e.stopPropagation(), connMenu(e, c))}
        ondblclick={(e) => e.stopPropagation()}><Icon name="more" size={11} /></button>
    </span>
  </div>
{/snippet}

{#snippet sectionHead(k: keyof Sections, label: string, count: number)}
  <button class="shead" aria-expanded={sections[k]} onclick={() => toggleSection(k)}>
    <Icon name={sections[k] ? "chevronDown" : "chevronRight"} size={11} />
    <span>{label}</span>
    <span class="count">{count}</span>
  </button>
{/snippet}

{#snippet activeRow(a: { pane: OpenPane; c: Connection; n: number })}
  {@const st = a.pane.state}
  <div
    class="row"
    class:active={sessions.activeTabId === a.pane.tabId && sessions.activeTab?.activeLeaf === a.pane.leafId}
    data-row
    role="treeitem"
    aria-selected={sessions.activeTabId === a.pane.tabId && sessions.activeTab?.activeLeaf === a.pane.leafId}
    tabindex="0"
    style:--depth={0}
    title={t("sidebar.focusOpen")}
    onclick={() => ((ui.sidebarDrawer = false), sessions.focus(a.pane.tabId, a.pane.leafId))}
    onkeydown={(e) => {
      if (e.key === "Enter") sessions.focus(a.pane.tabId, a.pane.leafId);
      else rowKey(e, a.c);
    }}
    oncontextmenu={(e) => connMenu(e, a.c)}
  >
    <span class="env env-{a.c.env}" aria-label={t(`env.${a.c.env}`)}></span>
    <ConnIcon icon={a.c.icon} />
    <span class="name">{a.c.name}{(openCount.get(a.c.id) ?? 0) > 1 ? ` #${a.n}` : ""}</span>
    {#if a.c.env === "prod"}<span class="env-prod-badge">PROD</span>{/if}
    {#if st === "agent_running"}<span class="agentic" title={t("state.agent_running")}><Icon name="sparkle" size={11} /></span>{/if}
    <span class="meta">{a.c.kind === "rdp" ? "RDP" : a.c.kind === "mysql" ? "MySQL" : a.c.kind === "postgres" ? "PostgreSQL" : "SSH"}</span>
    <span class="dot {a.pane.pending ? 'connecting' : (st ?? 'idle')}" title={a.pane.pending ? t("state.connecting") : st ? t(`state.${st}`) : t("sidebar.open")}></span>
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
        placeholder={t("sidebar.searchFilters")}
        title={t("sidebar.searchHint")}
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
    <button class="icon-btn" class:on={parsed.tokens.length > 0} aria-label={t("filter.menu")} title={t("filter.menu")} aria-haspopup="menu" onclick={filterMenu}><Icon name="menu" /></button>
    <button class="icon-btn" aria-label={t("sidebar.new")} aria-haspopup="menu" onclick={(e) => {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      newMenu = { x: r.left, y: r.bottom + 4 };
    }}><Icon name="plus" /></button>
  </div>
  {#if parsed.tokens.length}
    <div class="chips" aria-label={t("filter.active")}>
      {#each parsed.tokens as tk, i (i)}
        <button class="chip" class:not={tk.not} title={t("filter.remove")} onclick={() => (query = removeToken(query, tk.raw))}>
          <span>{tk.not ? "−" : ""}{tk.key}:{tk.value}</span><Icon name="x" size={10} />
        </button>
      {/each}
      <span class="sr-only" aria-live="polite">{t("filter.matches", { n: results?.length ?? filteredTree?.total ?? 0 })}</span>
    </div>
  {/if}

  <div class="kinds" role="tablist" data-tour="kinds" use:segmented>
    <button role="tab" aria-selected={kind === "ssh"} class="kind" class:on={kind === "ssh"} onclick={() => (kind = "ssh")}>{t("sidebar.servers")}</button>
    <button role="tab" aria-selected={kind === "db"} class="kind" class:on={kind === "db"} onclick={() => (kind = "db")}>{t("sidebar.databases")}</button>
    <button role="tab" aria-selected={kind === "s3"} class="kind" class:on={kind === "s3"} onclick={() => (kind = "s3")}>S3</button>
  </div>

  {#if kind === "s3"}
    <div class="tree" role="tree" aria-label={t("s3.accounts")}>
      {#each s3Results as a (a.id)}
        <div
          class="row"
          data-row
          role="treeitem"
          aria-selected="false"
          tabindex="0"
          style:--depth={0}
          title={s3Where(a)}
          ondblclick={() => sessions.openS3(a.id)}
          onclick={() => ui.narrow && sessions.openS3(a.id)}
          onkeydown={(e) => e.key === "Enter" && sessions.openS3(a.id)}
          oncontextmenu={(e) => s3Menu(e, a)}
        >
          <Icon name="bucket" size={13} />
          <span class="name">{a.name}</span>
          <span class="meta">{s3Where(a)}</span>
        </div>
      {:else}
        {#if query.trim()}
          <p class="empty-line">{t("sidebar.noResults", { q: query })}</p>
        {:else}
          <div class="empty">
            <Icon name="bucket" size={20} />
            <strong>{t("s3.empty.title")}</strong>
            <p>{t("s3.empty.body")}</p>
            <div class="empty-actions">
              <button class="btn sm primary" onclick={() => s3.edit(blankS3Account(), true)}>{t("s3.new")}</button>
            </div>
          </div>
        {/if}
      {/each}
    </div>
  {:else}
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
    {:else if connections.loaded && !tabTree.root.length && !tabTree.folders.length && !creatingFolder}
      {#if kind === "db"}
        <div class="empty">
          <Icon name="database" size={20} />
          <strong>{t("sidebar.emptyDb.title")}</strong>
          <p>{t("sidebar.emptyDb.body")}</p>
          <div class="empty-actions">
            <button class="btn sm primary" onclick={() => newDatabase()}>{t("sidebar.newDatabase")}</button>
            <button class="btn sm" onclick={() => sessions.openBackup()}>{t("backup.open")}</button>
          </div>
        </div>
      {:else}
        <div class="empty">
          <Icon name="server" size={20} />
          <strong>{t("sidebar.empty.title")}</strong>
          <p>{t("sidebar.empty.body")}</p>
          <div class="empty-actions">
            <button class="btn sm primary" onclick={() => ui.edit(blankConnection(), true)}>{t("sidebar.newConnection")}</button>
            <button class="btn sm" onclick={() => ui.openSettings("import")}>{t("sidebar.empty.import")}</button>
          </div>
        </div>
      {/if}
    {:else if filteredTree}
      {#each filteredTree.folders as f (f.group.id)}{@render folder(f, 0)}{/each}
      {#each filteredTree.root as c (c.id)}{@render row(c, 0)}{/each}
      {#if !filteredTree.total}<p class="empty-line">{t("sidebar.noResults", { q: query })}</p>{/if}
    {:else}
      {#if activePanes.length}
        {@render sectionHead("active", t("sidebar.activeSessions"), activePanes.length)}
        {#if sections.active}{#each activePanes as a (a.pane.leafId)}{@render activeRow(a)}{/each}{/if}
      {/if}
      {#if favorites.length}
        {@render sectionHead("fav", t("sidebar.favorites"), favorites.length)}
        {#if sections.fav}{#each favorites as c (c.id)}{@render row(c, 0)}{/each}{/if}
      {/if}
      {#if recents.length}
        {@render sectionHead("recent", t("sidebar.recent"), recents.length)}
        {#if sections.recent}{#each recents as c (c.id)}{@render row(c, 0)}{/each}{/if}
      {/if}
      {#if activePanes.length || favorites.length || recents.length}<div class="section">{t("sidebar.all")}</div>{/if}
      {#if creatingFolder?.parent === null}{@render folderInput(0)}{/if}
      {#each tabTree.folders as f (f.group.id)}{@render folder(f, 0)}{/each}
      {#if tabTree.root.length}
        {#if tabTree.folders.length}<div class="section">{t("sidebar.ungrouped")}</div>{/if}
        {#each tabTree.root as c (c.id)}{@render row(c, 0)}{/each}
      {/if}
    {/if}
  </div>
  {/if}

  {#if kind === "ssh"}
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
  {/if}
</aside>

{#if menu}<ContextMenu {...menu} onclose={() => (menu = null)} />{/if}
{#if newMenu}
  <ContextMenu
    {...newMenu}
    items={[
      { label: t("sidebar.newConnection"), icon: "server", action: () => ((kind = "ssh"), ui.edit(blankConnection(), true)) },
      { label: t("sidebar.newDatabase"), icon: "database", action: () => newDatabase() },
      { label: t("backup.open"), icon: "file", action: () => sessions.openBackup() },
      { label: t("sidebar.newFolder"), icon: "folderPlus", action: () => startFolder(null) },
      { label: t("s3.new"), icon: "bucket", action: () => s3.edit(blankS3Account(), true) },
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
    min-width: 0;
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
    color: var(--text);
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
  /* Folders read as headings of the tree: bright, semibold. (Issue #30: they were dimmer than
     the connections inside them, and the chevrons nearly invisible.) */
  .folder {
    color: var(--text-2);
    font-size: var(--fs-meta);
    font-weight: 600;
  }
  .folder > :global(svg) {
    flex: none;
    color: var(--muted);
    transition: color var(--t-fast) var(--ease);
  }
  .folder:hover > :global(svg) {
    color: var(--text);
  }
  .folder:hover,
  .row:hover {
    background: var(--active);
  }
  /* Connect / more, shown on hover or keyboard focus (always on touch layouts). */
  .acts {
    display: none;
    gap: 2px;
    margin-left: 2px;
  }
  .row:hover .acts,
  .row:focus-visible .acts,
  .row:focus-within .acts {
    display: inline-flex;
  }
  .row:hover .meta,
  .row:focus-visible .meta {
    display: none;
  }
  .act {
    display: inline-grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: var(--r-sm);
    color: var(--muted);
  }
  .act:hover {
    background: var(--hover);
    color: var(--text);
  }
  .agentic {
    display: inline-flex;
    color: var(--accent-text);
  }
  .shead {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 8px 4px;
    color: var(--muted);
    font-size: var(--fs-small);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .shead:hover {
    color: var(--text-2);
  }
  .shead .count {
    margin-left: auto;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 0 10px 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 6px;
    border-radius: 10px;
    background: var(--accent-soft);
    border: 1px solid var(--accent-line);
    color: var(--accent-text);
    font-family: var(--font-mono);
    font-size: var(--fs-tiny);
  }
  .chip.not {
    background: var(--danger-soft);
    border-color: var(--danger);
    color: var(--danger);
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
    color: var(--meta);
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
  /* The open connection: accent tint plus a bar, distinct from hover in both themes. */
  .row.active {
    background: var(--accent-soft);
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .row.active .name {
    color: var(--text);
    font-weight: 550;
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
    color: var(--text);
  }
  .meta {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-small);
    color: var(--meta);
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
    font-size: var(--fs-meta);
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
  .kindic {
    display: inline-flex;
    flex: none;
    color: var(--faint);
  }
</style>
