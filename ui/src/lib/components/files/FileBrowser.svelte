<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import Modal from "../common/Modal.svelte";
  import ContextMenu, { type MenuEntry } from "../common/ContextMenu.svelte";
  import CopyDialog from "./CopyDialog.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import { errorMessage } from "../../api/errors";
  import type { FileEntry } from "../../api/types";
  import { toasts } from "../../stores/toasts.svelte";
  import { copyText } from "../../util/clipboard";
  import { transfers } from "../../stores/transfers.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { changedOnServer, editable, isFolder, type FileBackend } from "../../util/fileBackends";
  import { editors } from "../../stores/editors.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { sudo } from "../../stores/sudo.svelte";
  import { joinPath, modeString, parentDir, validName } from "../../util/files";
  import { nameMatcher, searchTree, type SearchProgress } from "../../util/fileSearch";
  import { formatBytes, relativeTime } from "../../util/time";

  /** Browser for one remote file system; `label` names it (user@host, account name). */

  /** The menu closes on click, so the result is said in a toast. */
  async function copyPath(path: string) {
    if (await copyText(path)) toasts.success(t("files.pathCopied"));
    else toasts.error(t("copy.failed"));
  }
  let { fs: base, label }: { fs: FileBackend; label: string } = $props();

  /** The same file system as root, while the root toggle is on. */
  let rootFs = $state<FileBackend | null>(null);
  const fs = $derived(rootFs ?? base);
  // A reconnect hands over a new session: its root server has to be started again.
  $effect(() => {
    void base.key;
    untrack(() => {
      void rootFs?.leaveRoot?.().catch(() => {});
      rootFs = null;
    });
  });

  let path = $state("");
  let pathInput = $state("");
  let entries = $state<FileEntry[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  /** Selected rows by path; Ctrl/Cmd-click adds or removes one, Shift-click a range. */
  let sel = $state<string[]>([]);
  /** Where a Shift-click range starts. */
  let anchor: string | null = null;
  const selSet = $derived(new Set(sel));
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  /** Name dialog for "New folder", "New file" (in `dir`) or "Rename". */
  let naming = $state<{ mode: "mkdir" | "touch" | "rename"; dir?: string; entry?: FileEntry; value: string } | null>(null);
  const TITLES = { mkdir: "files.newFolder", touch: "files.newFile", rename: "files.rename" } as const;

  const mine = $derived(transfers.of(fs.key));
  /** Entries being copied to another server. */
  let copying = $state<FileEntry[] | null>(null);
  const writable = $derived(fs.writable(path));

  /** The folder last asked for (switching to root retries it, e.g. after "Permission denied"). */
  let wanted = "";

  async function go(to: string) {
    wanted = to;
    loading = true;
    error = null;
    try {
      entries = await fs.list(to);
      path = to;
      pathInput = to;
      sel = [];
      anchor = null;
      // A new folder is a new place to look: the search starts over.
      query = "";
      // Where this browser is: the default destination of copies from other servers.
      transfers.located(fs, to);
    } catch (e) {
      error = errorMessage(e);
      pathInput = path || to;
    } finally {
      loading = false;
    }
  }
  const refresh = () => go(path);

  // ---- search ----
  let searching = $state(false);
  let query = $state("");
  /** Also look in subfolders (walks them; see `searchTree`). */
  let deep = $state(false);
  let found = $state<SearchProgress | null>(null);
  let searchInput = $state<HTMLInputElement>();
  const matcher = $derived(nameMatcher(query));
  const deepOn = $derived(searching && deep && !!query.trim());
  /** The rows on screen: the folder filtered by the search, or what the walk found so far. */
  const shown = $derived(deepOn ? (found?.matches ?? []) : searching ? entries.filter((e) => matcher(e.name)) : entries);
  /** The selected rows among those on screen: actions never touch rows the search hides. */
  const picked = $derived(shown.filter((e) => selSet.has(e.path)));

  function openSearch() {
    searching = true;
    queueMicrotask(() => searchInput?.focus());
  }
  function closeSearch() {
    searching = false;
    query = "";
  }

  // Walk the subfolders, a moment after typing stops; a new query, folder or close cancels it.
  $effect(() => {
    if (!deepOn) {
      found = null;
      return;
    }
    const [q, root, f] = [query, path, fs];
    const ctl = new AbortController();
    found = { folders: 0, matches: [], truncated: false, done: false };
    const timer = setTimeout(() => {
      void searchTree(f, root, q, (p) => (found = p), ctl.signal).catch(() => {});
    }, 300);
    return () => {
      clearTimeout(timer);
      ctl.abort();
    };
  });

  /** Where a match is, relative to the folder searched. */
  function whereIs(e: FileEntry) {
    const dir = parentDir(e.path);
    if (dir === path) return "";
    const rel = path !== "/" && dir.startsWith(path + "/") ? dir.slice(path.length + 1) : dir;
    return rel + "/";
  }

  // ---- selection ----
  function clickRow(ev: MouseEvent, e: FileEntry) {
    const add = ev.ctrlKey || ev.metaKey;
    if (ev.shiftKey && anchor) {
      const list = shown.map((x) => x.path);
      const [a, b] = [list.indexOf(anchor), list.indexOf(e.path)];
      if (a >= 0 && b >= 0) {
        const range = list.slice(Math.min(a, b), Math.max(a, b) + 1);
        sel = add ? [...new Set([...sel, ...range])] : range;
        return;
      }
    }
    if (add) sel = selSet.has(e.path) ? sel.filter((x) => x !== e.path) : [...sel, e.path];
    else sel = [e.path];
    anchor = e.path;
  }

  function listKey(ev: KeyboardEvent) {
    const mod = ev.ctrlKey || ev.metaKey;
    // Text fields (path, search) keep their own Select All.
    const typing = ev.target instanceof HTMLElement && (ev.target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(ev.target.tagName));
    if (mod && ev.key.toLowerCase() === "a" && !typing) {
      ev.preventDefault();
      sel = shown.map((e) => e.path);
    } else if (mod && ev.key.toLowerCase() === "f") {
      ev.preventDefault();
      openSearch();
    } else if (ev.key === "Escape" && sel.length) {
      ev.stopPropagation();
      sel = [];
    }
  }

  async function toggleRoot() {
    if (rootFs) {
      void rootFs.leaveRoot?.().catch(() => {});
      rootFs = null;
      return refresh();
    }
    await becomeRoot();
  }

  /** Start the root SFTP server; asks for the sudo password when sudo needs one. */
  async function becomeRoot() {
    try {
      const r = await sudo.elevate(base);
      if (!r) return;
      rootFs = r;
      await go(wanted || path || (await r.home()));
    } catch (e) {
      toasts.error(e);
    }
  }

  // Closing the browser leaves root mode too.
  onDestroy(() => {
    void rootFs?.leaveRoot?.().catch(() => {});
    transfers.gone(fs);
  });

  // A copy from another browser landed here: show it.
  let seen = untrack(() => transfers.touched[base.key] ?? 0);
  $effect(() => {
    const n = transfers.touched[base.key] ?? 0;
    if (n !== seen) {
      seen = n;
      untrack(() => path && void refresh());
    }
  });

  onMount(async () => {
    try {
      await go(await fs.home());
    } catch (e) {
      error = errorMessage(e);
      loading = false;
    }
  });

  function open(e: FileEntry) {
    if (e.kind === "dir") void go(e.path);
    else if (editable(fs, e)) void edit(e);
    else void download(e);
  }

  /** Open a text file in the editor (as root while the browser is). */
  async function edit(e: FileEntry) {
    try {
      sessions.openEditor(await editors.open(fs, e.path));
    } catch (err) {
      toasts.error(err);
    }
  }

  async function download(e: FileEntry) {
    try {
      const local = await transfers.download(fs, e.path);
      toasts.success(t("files.downloaded", { path: local }));
    } catch (err) {
      toasts.error(err);
    }
  }

  async function upload() {
    const picked = await api.pickFiles().catch((e) => (toasts.error(e), [] as string[]));
    const dir = path;
    for (const p of picked) {
      try {
        await transfers.upload(fs, p, dir);
      } catch (e) {
        toasts.error(e);
      }
    }
    if (picked.length && dir === path) await refresh();
  }

  async function remove(list: FileEntry[]) {
    if (!list.length) return;
    const one = list.length === 1 ? list[0] : null;
    const folders = list.some(isFolder);
    const names = list.slice(0, 5).map((e) => e.name).join(", ") + (list.length > 5 ? ", …" : "");
    const ok = await ui.ask(one ? t(isFolder(one) ? "files.deleteFolder" : "files.deleteFile", { name: one.name }) : t("files.deleteMany", { n: list.length }), {
      body: [one ? "" : names, folders ? t("files.deleteFolderHint") : ""].filter(Boolean).join(". ") || undefined,
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!ok) return;
    const gone = new Set<string>();
    // Deepest first: a selected folder is removed after the selected things inside it.
    const depth = (e: FileEntry) => e.path.split("/").length;
    for (const e of [...list].sort((a, b) => depth(b) - depth(a))) {
      try {
        await fs.remove(e);
        gone.add(e.path);
      } catch (err) {
        toasts.error(list.length > 1 ? `${e.name}: ${errorMessage(err)}` : err);
      }
    }
    sel = sel.filter((p) => !gone.has(p));
    if (found) found = { ...found, matches: found.matches.filter((m) => !gone.has(m.path)) };
    await refresh();
  }

  /** Download the files among `list` (folders cannot be downloaded). */
  async function downloadMany(list: FileEntry[]) {
    const files = list.filter((e) => e.kind === "file");
    if (files.length < list.length) toasts.info(t("files.downloadSkipsFolders"));
    if (files.length === 1) return download(files[0]);
    let ok = 0;
    for (const f of files) {
      try {
        await transfers.download(fs, f.path);
        ok++;
      } catch (err) {
        toasts.error(`${f.name}: ${errorMessage(err)}`);
      }
    }
    if (ok) toasts.success(t("files.downloadedMany", { n: ok }));
  }

  async function submitName(ev: SubmitEvent) {
    ev.preventDefault();
    const n = naming;
    if (!n || !validName(n.value)) return;
    naming = null;
    const name = n.value.trim();
    try {
      if (n.mode === "mkdir") await fs.mkdir(joinPath(n.dir ?? path, name));
      else if (n.mode === "touch") return await createFile(n.dir ?? path, name);
      else if (n.entry) await fs.rename(n.entry, joinPath(parentDir(n.entry.path), name));
      await refresh();
    } catch (err) {
      toasts.error(err);
    }
  }

  /** Create an empty file (never over an existing one) and open it in the editor. */
  async function createFile(dir: string, name: string) {
    const target = joinPath(dir, name);
    const fsNow = fs;
    try {
      await fsNow.writeText!(target, "", null, false);
    } catch (err) {
      toasts.error(changedOnServer(err) ? t("files.exists", { name }) : err);
      return;
    }
    await refresh();
    sel = [target];
    try {
      sessions.openEditor(await editors.open(fsNow, target, true));
    } catch (err) {
      toasts.error(err);
    }
  }

  /** "New file" and "New folder", in `dir`. */
  const newItems = (dir: string, ok: boolean): MenuEntry[] => [
    ...(fs.writeText ? [{ label: t("files.newFile"), icon: "filePlus" as const, disabled: !ok, action: () => (naming = { mode: "touch", dir, value: "" }) }] : []),
    { label: t("files.newFolder"), icon: "folderPlus", disabled: !ok, action: () => (naming = { mode: "mkdir", dir, value: "" }) },
  ];

  /** Right-click on the empty part of the list: the current folder. */
  function folderMenu(ev: MouseEvent) {
    if (ev.defaultPrevented) return;
    ev.preventDefault();
    sel = [];
    menu = {
      x: ev.clientX,
      y: ev.clientY,
      items: [
        ...newItems(path, writable),
        { label: t("files.upload"), icon: "upload", disabled: !writable, action: upload },
        "sep",
        { label: t("files.refresh"), icon: "refresh", action: refresh },
        { label: t("files.copyPath"), icon: "copy", disabled: !path, action: () => void copyPath(path) },
      ],
    };
  }

  function openMenu(ev: MouseEvent, e: FileEntry) {
    ev.preventDefault();
    // Right-click inside the selection acts on all of it; elsewhere it selects that row.
    if (!selSet.has(e.path)) {
      sel = [e.path];
      anchor = e.path;
    }
    const many = shown.filter((x) => selSet.has(x.path));
    if (many.length > 1) {
      const files = many.filter((x) => x.kind === "file").length;
      menu = {
        x: ev.clientX,
        y: ev.clientY,
        items: [
          { label: t("files.downloadN", { n: files }), icon: "download", disabled: !files, action: () => downloadMany(many) },
          { label: t("files.copyN", { n: many.length }), icon: "server", disabled: !copyAll(many), action: () => (copying = many) },
          { label: t("files.copyPaths"), icon: "copy", action: () => void copyPath(many.map((x) => x.path).join("\n")) },
          "sep",
          { label: t("files.deleteN", { n: many.length }), icon: "trash", danger: true, disabled: !many.every((x) => fs.writable(parentDir(x.path))), action: () => remove(many) },
        ],
      };
      return;
    }
    const isFile = e.kind !== "dir";
    menu = {
      x: ev.clientX,
      y: ev.clientY,
      items: [
        ...(isFile && fs.readText ? [{ label: t("files.edit"), icon: "edit" as const, action: () => edit(e) }] : []),
        isFile ? { label: t("files.download"), icon: "download", action: () => download(e) } : { label: t("files.open"), icon: "folder", action: () => go(e.path) },
        { label: t("files.copyPath"), icon: "copy", action: () => void copyPath(e.path) },
        // Folders only go server to server; files also between a server and S3.
        ...(fs.kind === "sftp" || isFile ? [{ label: t("files.copyToServer"), icon: "server" as const, action: () => (copying = [e]) }] : []),
        "sep",
        { label: t("files.rename"), icon: "edit", disabled: !fs.writable(parentDir(e.path)) || !fs.canRename(e), action: () => (naming = { mode: "rename", entry: e, value: e.name }) },
        { label: t("common.delete"), icon: "trash", danger: true, disabled: !fs.writable(parentDir(e.path)), action: () => remove([e]) },
        "sep",
        // On a folder: new items go inside it.
        ...newItems(e.kind === "dir" ? e.path : path, e.kind === "dir" ? fs.writable(e.path) : writable),
      ],
    };
  }

  function rowKey(ev: KeyboardEvent, e: FileEntry) {
    if (ev.key === "Enter") open(e);
    if (ev.key === " ") {
      ev.preventDefault();
      clickRow(new MouseEvent("click", { ctrlKey: true }), e);
    }
    if (ev.key === "Delete") {
      // The selection if this row is in it, else just this row.
      const list = selSet.has(e.path) ? picked : [e];
      if (list.every((x) => fs.writable(parentDir(x.path)))) void remove(list);
    }
    if (ev.key === "Backspace") void go(parentDir(path));
  }

  const crumbs = $derived.by(() => {
    const parts = path.split("/").filter(Boolean);
    return [{ name: "/", path: "/" }, ...parts.map((p, i) => ({ name: p, path: "/" + parts.slice(0, i + 1).join("/") }))];
  });
  /** Whether all of `list` can be copied elsewhere (folders only go server to server). */
  const copyAll = (list: FileEntry[]) => list.length > 0 && list.every((e) => fs.kind === "sftp" || e.kind !== "dir");
  const copyable = $derived(copyAll(picked) ? picked : null);
  const icon = (e: FileEntry) => (e.kind === "dir" ? "folder" : e.link ? "link" : "file");
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="files" onkeydown={listKey}>
  <div class="bar">
    <button class="icon-btn" title={t("files.up")} aria-label={t("files.up")} disabled={!path || path === "/"} onclick={() => go(parentDir(path))}><Icon name="arrowUp" size={14} /></button>
    <form class="pathform" onsubmit={(e) => (e.preventDefault(), go(pathInput.trim() || "/"))}>
      <input class="input mono path" bind:value={pathInput} aria-label={t("files.path")} spellcheck="false" />
    </form>
    <button class="icon-btn" title={t("files.searchTip")} aria-label={t("files.search")} aria-pressed={searching} class:on={searching} onclick={() => (searching ? closeSearch() : openSearch())}><Icon name="search" size={14} /></button>
    <button class="icon-btn" title={t("files.refresh")} aria-label={t("files.refresh")} onclick={refresh}><Icon name="refresh" size={14} /></button>
    {#if fs.writeText}
      <button class="icon-btn" title={t("files.newFile")} aria-label={t("files.newFile")} disabled={!writable} onclick={() => (naming = { mode: "touch", value: "" })}><Icon name="filePlus" size={14} /></button>
    {/if}
    <button class="icon-btn" title={t("files.newFolder")} aria-label={t("files.newFolder")} disabled={!writable} onclick={() => (naming = { mode: "mkdir", value: "" })}><Icon name="folderPlus" size={14} /></button>
    <button class="btn sm" disabled={!writable} aria-label={t("files.upload")} onclick={upload}><Icon name="upload" size={13} /><span class="lbl">{t("files.upload")}</span></button>
    <button class="btn sm" disabled={!copyable} aria-label={t("files.copyButton")} title={copyable ? t("files.copyToServer") : t("files.copyPick")} onclick={() => (copying = copyable)}><Icon name="send" size={13} /><span class="lbl">{t("files.copyButton")}</span></button>
    {#if base.asRoot}
      <button class="btn sm root" class:on={!!rootFs} aria-pressed={!!rootFs} aria-label={t("files.root")} title={t(rootFs ? "files.rootOff" : "files.rootOn")} onclick={toggleRoot}>
        <Icon name="shield" size={13} /><span class="lbl">{t("files.root")}</span>
      </button>
    {/if}
  </div>
  {#if searching}
    <div class="search">
      <Icon name="search" size={13} />
      <input
        class="input sm"
        bind:this={searchInput}
        bind:value={query}
        placeholder={t("files.searchPlaceholder")}
        aria-label={t("files.search")}
        spellcheck="false"
        onkeydown={(e) => e.key === "Escape" && (e.stopPropagation(), closeSearch())}
      />
      <label class="deep" title={t("files.searchDeepTip")}><input type="checkbox" bind:checked={deep} />{t("files.searchDeep")}</label>
      <span class="status" aria-live="polite">
        {#if deepOn && found}
          {found.done ? (found.truncated ? t("files.searchCapped", { n: found.matches.length, folders: found.folders }) : t("files.searchFound", { n: found.matches.length, folders: found.folders })) : t("files.searching", { folders: found.folders })}
        {:else if query.trim()}
          {t("files.searchShown", { n: shown.length, total: entries.length })}
        {/if}
      </span>
      <button class="icon-btn" aria-label={t("common.close")} onclick={closeSearch}><Icon name="x" size={13} /></button>
    </div>
  {/if}
  <nav class="crumbs" aria-label={t("files.path")}>
    {#each crumbs as c, i (c.path)}
      {#if i > 1}<span class="sep">/</span>{/if}
      <button class="crumb" onclick={() => go(c.path)}>{c.name}</button>
    {/each}
    <span class="grow"></span>
    <span class="who" class:asroot={!!rootFs}>{rootFs ? t("files.asRoot", { label }) : label}</span>
  </nav>

  <div class="list" role="grid" tabindex="-1" aria-busy={loading} oncontextmenu={folderMenu}>
    {#if error}
      <div class="msg err">{error}</div>
    {:else if !loading && !entries.length && !deepOn}
      <div class="msg">{t("files.empty")}</div>
    {:else if searching && query.trim() && !shown.length && (!deepOn || found?.done)}
      <div class="msg">{t("files.searchNone")}</div>
    {/if}
    {#each shown as e (e.path)}
      <div
        class="row"
        class:sel={selSet.has(e.path)}
        role="row"
        tabindex="0"
        aria-selected={selSet.has(e.path)}
        onclick={(ev) => clickRow(ev, e)}
        ondblclick={() => open(e)}
        onkeydown={(ev) => rowKey(ev, e)}
        oncontextmenu={(ev) => openMenu(ev, e)}
      >
        <span class="name" role="gridcell"><Icon name={icon(e)} size={14} /><span class="n">{#if deepOn && whereIs(e)}<span class="in">{whereIs(e)}</span>{/if}{e.name}</span>{#if e.link}<span class="lnk" title={t("files.link")} aria-label={t("files.link")}>↗</span>{/if}</span>
        <span class="size" role="gridcell">{e.kind === "file" ? formatBytes(e.size) : ""}</span>
        <span class="mod" role="gridcell" title={e.modified ? new Date(e.modified * 1000).toLocaleString() : ""}>{e.modified ? relativeTime(e.modified * 1000) : ""}</span>
        <span class="perm mono" role="gridcell">{modeString(e.permissions)}</span>
      </div>
    {/each}
  </div>

  {#if picked.length > 1}
    <div class="selbar">
      <span class="count">{t("files.selected", { n: picked.length })}</span>
      <button class="btn sm ghost" disabled={!picked.some((e) => e.kind === "file")} onclick={() => downloadMany(picked)} aria-label={t("files.download")} title={t("files.download")}><Icon name="download" size={13} /><span class="lbl">{t("files.download")}</span></button>
      <button class="btn sm ghost danger" disabled={!picked.every((e) => fs.writable(parentDir(e.path)))} onclick={() => remove(picked)} aria-label={t("common.delete")} title={t("common.delete")}><Icon name="trash" size={13} /><span class="lbl">{t("common.delete")}</span></button>
      <span class="grow"></span>
      <button class="icon-btn" onclick={() => (sel = [])} aria-label={t("files.clearSelection")} title={t("files.clearSelection")}><Icon name="x" size={13} /></button>
    </div>
  {/if}

  {#if mine.length}
    <div class="transfers">
      {#each mine as x (x.id)}
        <div class="tr" class:failed={x.state === "failed"}>
          <Icon name={x.direction === "up" ? "upload" : x.direction === "copy" ? "copy" : "download"} size={13} />
          <span class="tn" title={x.result ?? ""}>{x.name}</span>
          {#if x.state === "running"}
            <progress max={x.total || 1} value={x.done}></progress>
            <span class="pct">{x.total ? Math.floor((x.done / x.total) * 100) : 0}%</span>
          {:else if x.state === "done"}
            <span class="ok">{t("files.done")}</span>
          {:else}
            <span class="bad" title={x.result ?? ""}>{t("files.failed")}</span>
          {/if}
        </div>
      {/each}
      <button class="btn sm ghost clear" onclick={() => transfers.clear(fs.key)}>{t("files.clearTransfers")}</button>
    </div>
  {/if}
</div>

{#if menu}<ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}
{#if copying}<CopyDialog {fs} entries={copying} onclose={() => (copying = null)} />{/if}

{#if naming}
  {@const n = naming}
  <Modal label={t(TITLES[n.mode])} onclose={() => (naming = null)} width="380px">
    <form class="dialog" onsubmit={submitName}>
      <h2>{t(TITLES[n.mode])}</h2>
      {#if n.mode !== "rename" && n.dir && n.dir !== path}<p class="where mono">{n.dir}</p>{/if}
      <input class="input mono" bind:value={n.value} aria-label={t("files.name")} spellcheck="false" data-autofocus />
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => (naming = null)}>{t("common.cancel")}</button>
        <button class="btn primary" disabled={!validName(n.value)}>{t(n.mode === "rename" ? "files.rename" : "files.create")}</button>
      </div>
    </form>
  </Modal>
{/if}

<style>
  .files {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-width: 0;
    background: var(--bg);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .pathform {
    flex: 1;
    min-width: 90px;
  }
  /* Narrow panes: the labelled buttons keep only their icons, so the path stays usable. */
  @container (max-width: 560px) {
    .bar .lbl {
      display: none;
    }
  }
  .path {
    width: 100%;
    height: 28px;
    font-size: 12px;
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 10px;
    min-width: 0;
    overflow: hidden;
    font-size: var(--fs-small);
    color: var(--faint);
    border-bottom: 1px solid var(--border);
  }
  .crumb {
    border: 0;
    background: none;
    color: var(--muted);
    padding: 1px 3px;
    border-radius: 4px;
    white-space: nowrap;
  }
  .crumb:hover {
    background: var(--hover);
    color: var(--text);
  }
  .grow {
    flex: 1;
  }
  .who {
    white-space: nowrap;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 4px;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto auto;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-radius: var(--r);
    cursor: default;
    font-size: var(--fs-small);
    user-select: none;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.sel,
  .row:focus-visible {
    background: var(--accent-soft);
    outline: none;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    color: var(--text);
  }
  .name :global(svg) {
    flex: none;
    color: var(--faint);
  }
  .n {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size,
  .mod,
  .perm {
    color: var(--faint);
    white-space: nowrap;
    text-align: right;
  }
  .perm {
    font-size: 11px;
  }
  /* Narrow panes: the name matters most, then size, then age. */
  @container (max-width: 560px) {
    .perm {
      display: none;
    }
    .row {
      grid-template-columns: minmax(0, 1fr) auto auto;
    }
  }
  @container (max-width: 400px) {
    .mod {
      display: none;
    }
    .row {
      grid-template-columns: minmax(0, 1fr) auto;
    }
  }
  .msg {
    padding: 14px 10px;
    color: var(--faint);
    font-size: var(--fs-meta);
  }
  .msg.err {
    color: var(--danger);
  }
  .transfers {
    display: grid;
    gap: 4px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-small);
  }
  .tr {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
  }
  .tn {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  progress {
    width: 110px;
    height: 6px;
  }
  .pct {
    width: 3.5ch;
    text-align: right;
    color: var(--faint);
  }
  .ok {
    color: var(--ok, var(--accent-text));
  }
  .bad,
  .tr.failed {
    color: var(--danger);
  }
  .clear {
    justify-self: end;
  }
  .dialog {
    display: grid;
    gap: 12px;
    padding: 20px;
  }
  .dialog h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .where {
    margin: -6px 0 0;
    color: var(--faint);
    font-size: var(--fs-small);
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .lnk {
    flex: none;
    color: var(--faint);
    font-size: var(--fs-small);
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    border-bottom: 1px solid var(--border);
    color: var(--faint);
  }
  .search .input {
    flex: 1;
    min-width: 80px;
    height: 26px;
  }
  .deep {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--muted);
    font-size: var(--fs-small);
    white-space: nowrap;
  }
  .status {
    color: var(--faint);
    font-size: var(--fs-small);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  @container (max-width: 560px) {
    .status {
      display: none;
    }
  }
  .icon-btn.on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .in {
    color: var(--faint);
  }
  .selbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-small);
  }
  .count {
    color: var(--muted);
    margin-right: 4px;
    white-space: nowrap;
  }
  @container (max-width: 560px) {
    .selbar .lbl {
      display: none;
    }
  }
  .selbar .danger {
    color: var(--danger);
  }
  .root.on {
    background: var(--warn-soft);
    border-color: var(--warn);
    color: var(--warn);
  }
  .who.asroot {
    color: var(--warn);
  }
</style>
