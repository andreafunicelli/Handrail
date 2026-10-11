<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import Modal from "../common/Modal.svelte";
  import ContextMenu, { type MenuEntry } from "../common/ContextMenu.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import { errorMessage } from "../../api/errors";
  import type { FileEntry } from "../../api/types";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { transfers } from "../../stores/transfers.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { joinPath, modeString, parentDir, validName } from "../../util/files";
  import { formatBytes, relativeTime } from "../../util/time";

  /** SFTP browser on the connection of SSH session `sessionId`. */
  let { sessionId }: { sessionId: string } = $props();

  let path = $state("");
  let pathInput = $state("");
  let entries = $state<FileEntry[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let selected = $state<string | null>(null);
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  /** Name dialog for "New folder" or "Rename". */
  let naming = $state<{ mode: "mkdir" | "rename"; entry?: FileEntry; value: string } | null>(null);

  const info = $derived(sessions.infos[sessionId]);
  const mine = $derived(transfers.list.filter((x) => x.sessionId === sessionId));

  async function go(to: string) {
    loading = true;
    error = null;
    try {
      entries = await api.sftpList(sessionId, to);
      path = to;
      pathInput = to;
      selected = null;
    } catch (e) {
      error = errorMessage(e);
      pathInput = path || to;
    } finally {
      loading = false;
    }
  }
  const refresh = () => go(path);

  onMount(async () => {
    try {
      await go(await api.sftpHome(sessionId));
    } catch (e) {
      error = errorMessage(e);
      loading = false;
    }
  });

  function open(e: FileEntry) {
    if (e.kind === "dir" || e.kind === "symlink") void go(e.path);
    else void download(e);
  }

  async function download(e: FileEntry) {
    try {
      const local = await transfers.download(sessionId, e.path);
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
        await transfers.upload(sessionId, p, dir);
      } catch (e) {
        toasts.error(e);
      }
    }
    if (picked.length && dir === path) await refresh();
  }

  async function remove(e: FileEntry) {
    const ok = await ui.ask(t(e.kind === "dir" ? "files.deleteFolder" : "files.deleteFile", { name: e.name }), {
      body: e.kind === "dir" ? t("files.deleteFolderHint") : undefined,
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.sftpRemove(sessionId, e.path, e.kind === "dir");
      await refresh();
    } catch (err) {
      toasts.error(err);
    }
  }

  async function submitName(ev: SubmitEvent) {
    ev.preventDefault();
    const n = naming;
    if (!n || !validName(n.value)) return;
    naming = null;
    try {
      if (n.mode === "mkdir") await api.sftpMkdir(sessionId, joinPath(path, n.value.trim()));
      else if (n.entry) await api.sftpRename(sessionId, n.entry.path, joinPath(parentDir(n.entry.path), n.value.trim()));
      await refresh();
    } catch (err) {
      toasts.error(err);
    }
  }

  function openMenu(ev: MouseEvent, e: FileEntry) {
    ev.preventDefault();
    selected = e.path;
    const isFile = e.kind === "file";
    menu = {
      x: ev.clientX,
      y: ev.clientY,
      items: [
        isFile ? { label: t("files.download"), icon: "download", action: () => download(e) } : { label: t("files.open"), icon: "folder", action: () => go(e.path) },
        { label: t("files.copyPath"), icon: "copy", action: () => void navigator.clipboard?.writeText(e.path).catch(() => {}) },
        "sep",
        { label: t("files.rename"), icon: "edit", action: () => (naming = { mode: "rename", entry: e, value: e.name }) },
        { label: t("common.delete"), icon: "trash", danger: true, action: () => remove(e) },
      ],
    };
  }

  function rowKey(ev: KeyboardEvent, e: FileEntry) {
    if (ev.key === "Enter") open(e);
    if (ev.key === "Delete") void remove(e);
    if (ev.key === "Backspace") void go(parentDir(path));
  }

  const crumbs = $derived.by(() => {
    const parts = path.split("/").filter(Boolean);
    return [{ name: "/", path: "/" }, ...parts.map((p, i) => ({ name: p, path: "/" + parts.slice(0, i + 1).join("/") }))];
  });
  const icon = (e: FileEntry) => (e.kind === "dir" ? "folder" : e.kind === "symlink" ? "link" : "file");
</script>

<div class="files">
  <div class="bar">
    <button class="icon-btn" title={t("files.up")} aria-label={t("files.up")} disabled={!path || path === "/"} onclick={() => go(parentDir(path))}><Icon name="arrowUp" size={14} /></button>
    <form class="pathform" onsubmit={(e) => (e.preventDefault(), go(pathInput.trim() || "/"))}>
      <input class="input mono path" bind:value={pathInput} aria-label={t("files.path")} spellcheck="false" />
    </form>
    <button class="icon-btn" title={t("files.refresh")} aria-label={t("files.refresh")} onclick={refresh}><Icon name="refresh" size={14} /></button>
    <button class="icon-btn" title={t("files.newFolder")} aria-label={t("files.newFolder")} disabled={!path} onclick={() => (naming = { mode: "mkdir", value: "" })}><Icon name="plus" size={14} /></button>
    <button class="btn sm" disabled={!path} onclick={upload}><Icon name="upload" size={13} />{t("files.upload")}</button>
  </div>
  <nav class="crumbs" aria-label={t("files.path")}>
    {#each crumbs as c, i (c.path)}
      {#if i > 1}<span class="sep">/</span>{/if}
      <button class="crumb" onclick={() => go(c.path)}>{c.name}</button>
    {/each}
    <span class="grow"></span>
    <span class="who">{info?.user}@{info?.host}</span>
  </nav>

  <div class="list" role="grid" aria-busy={loading}>
    {#if error}
      <div class="msg err">{error}</div>
    {:else if !loading && !entries.length}
      <div class="msg">{t("files.empty")}</div>
    {/if}
    {#each entries as e (e.path)}
      <div
        class="row"
        class:sel={selected === e.path}
        role="row"
        tabindex="0"
        onclick={() => (selected = e.path)}
        ondblclick={() => open(e)}
        onkeydown={(ev) => rowKey(ev, e)}
        oncontextmenu={(ev) => openMenu(ev, e)}
      >
        <span class="name" role="gridcell"><Icon name={icon(e)} size={14} /><span class="n">{e.name}</span></span>
        <span class="size" role="gridcell">{e.kind === "file" ? formatBytes(e.size) : ""}</span>
        <span class="mod" role="gridcell" title={e.modified ? new Date(e.modified * 1000).toLocaleString() : ""}>{e.modified ? relativeTime(e.modified * 1000) : ""}</span>
        <span class="perm mono" role="gridcell">{modeString(e.permissions)}</span>
      </div>
    {/each}
  </div>

  {#if mine.length}
    <div class="transfers">
      {#each mine as x (x.id)}
        <div class="tr" class:failed={x.state === "failed"}>
          <Icon name={x.direction === "up" ? "upload" : "download"} size={13} />
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
      <button class="btn sm ghost clear" onclick={() => transfers.clear(sessionId)}>{t("files.clearTransfers")}</button>
    </div>
  {/if}
</div>

{#if menu}<ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}

{#if naming}
  {@const n = naming}
  <Modal label={t(n.mode === "mkdir" ? "files.newFolder" : "files.rename")} onclose={() => (naming = null)} width="380px">
    <form class="dialog" onsubmit={submitName}>
      <h2>{t(n.mode === "mkdir" ? "files.newFolder" : "files.rename")}</h2>
      <input class="input mono" bind:value={n.value} aria-label={t("files.name")} spellcheck="false" data-autofocus />
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => (naming = null)}>{t("common.cancel")}</button>
        <button class="btn primary" disabled={!validName(n.value)}>{t(n.mode === "mkdir" ? "files.create" : "files.rename")}</button>
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
    min-width: 0;
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
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
