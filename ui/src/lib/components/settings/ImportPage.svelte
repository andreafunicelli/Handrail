<script lang="ts">
  import BusyButton from "../common/BusyButton.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { Connection, ImportPreview, ImportPreviewArgs, ImportSource } from "../../api/types";
  import { connections } from "../../stores/connections.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { groupPath } from "../../util/tree";
  import { IMPORT_TOOLS, needsMremotengPassword, toolLabel, type ImportTool } from "../../util/importers";

  let step = $state<"start" | "sources" | "file" | "preview">("start");
  let sources = $state<ImportSource[]>([]);
  let busy = $state(false);
  let preview = $state<ImportPreview | null>(null);
  /** Indices (into preview.connections) the user chose to import. */
  let selected = $state<Set<number>>(new Set());
  /** Where "Back" from the preview returns to. */
  let origin = $state<"sources" | "file">("sources");

  // "From a file" form.
  let fileTool = $state<ImportTool>("securecrt");
  let filePath = $state("");
  let importPasswords = $state(true);
  let mremotengPassword = $state("");

  /** An mRemoteNG preview that failed on its master password, waiting for the user to type it. */
  let pwRetry = $state<{ args: ImportPreviewArgs; wrong: boolean } | null>(null);
  let pwInput = $state("");
  let mrngField = $state<HTMLInputElement | null>(null);

  const dupIndex = $derived(new Set(preview?.duplicates.map((d) => d.index) ?? []));
  const grouped = $derived.by(() => {
    if (!preview) return [];
    const groups = new Map<string, { path: string; items: { c: Connection; i: number }[] }>();
    preview.connections.forEach((c, i) => {
      const path = groupPath(preview!.groups, c.group_id) || t("sidebar.ungrouped");
      (groups.get(path) ?? groups.set(path, { path, items: [] }).get(path)!).items.push({ c, i });
    });
    return [...groups.values()];
  });

  async function scan() {
    busy = true;
    try {
      sources = await api.importDetect();
      step = "sources";
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
  }

  function goto(next: typeof step) {
    pwRetry = null;
    step = next;
  }

  async function load(args: ImportPreviewArgs, tool: string, from: "sources" | "file") {
    busy = true;
    try {
      preview = await api.importPreview(args);
      selected = new Set(preview.connections.map((_, i) => i).filter((i) => !preview!.duplicates.some((d) => d.index === i)));
      origin = from;
      pwRetry = null;
      pwInput = "";
      step = "preview";
    } catch (e) {
      if (needsMremotengPassword(tool, e)) {
        pwRetry = { args, wrong: !!args.mremoteng_password };
        // The file form has its own password field; detected sources get the inline prompt.
        if (from === "file") mrngField?.focus();
      } else {
        toasts.error(e);
      }
    } finally {
      busy = false;
    }
  }

  function open(src: ImportSource) {
    void load({ source_id: src.id }, src.tool, "sources");
  }

  function openFile(e: SubmitEvent) {
    e.preventDefault();
    const path = filePath.trim();
    if (!path) return;
    const args: ImportPreviewArgs = { tool: fileTool, file_path: path, import_passwords: importPasswords };
    if (fileTool === "mremoteng" && mremotengPassword) args.mremoteng_password = mremotengPassword;
    void load(args, fileTool, "file");
  }

  function retryWithPassword(e: SubmitEvent) {
    e.preventDefault();
    if (!pwRetry || !pwInput) return;
    void load({ ...pwRetry.args, mremoteng_password: pwInput }, "mremoteng", "sources");
  }

  function toggle(i: number) {
    const s = new Set(selected);
    if (s.has(i)) s.delete(i);
    else s.add(i);
    selected = s;
  }

  async function apply() {
    if (!preview) return;
    busy = true;
    try {
      const skip = preview.connections.map((_, i) => i).filter((i) => !selected.has(i));
      const { imported } = await api.importApply(preview.preview_id, skip);
      await connections.load();
      toasts.success(t("imp.done", { count: imported }));
      preview = null;
      step = "start";
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
  }
</script>

{#snippet passwordPrompt()}
  {#if pwRetry}
    <form class="card pwprompt" onsubmit={retryWithPassword}>
      <p class="hint">{t(pwRetry.wrong ? "imp.mrngWrong" : "imp.mrngNeeded")}</p>
      <div class="row">
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input" type="password" bind:value={pwInput} placeholder={t("imp.mrngPassword")} aria-label={t("imp.mrngPassword")} autocomplete="off" autofocus />
        <button class="btn primary" disabled={busy || !pwInput}>{t("imp.retry")}</button>
      </div>
    </form>
  {/if}
{/snippet}

<p class="intro">{t("imp.body")}</p>

{#if step === "start"}
  <div class="actions start">
    <BusyButton class="primary" onclick={scan} {busy} busyLabel={t("imp.scanning")}><Icon name="search" size={14} />{t("imp.detect")}</BusyButton>
    <button class="btn" onclick={() => goto("file")} disabled={busy}><Icon name="folder" size={14} />{t("imp.fromFile")}</button>
  </div>
{:else if step === "sources"}
  {#if !sources.length}
    <p class="hint">{t("imp.none")}</p>
  {:else}
    <div class="card">
      {#each sources as s (s.id)}
        <div class="list-row">
          <span class="tool">{toolLabel(s.tool)}</span>
          <span class="path mono" title={s.path ?? ""}>{s.path ?? ""}</span>
          {#if s.count_hint != null}<span class="pill">{t("imp.found", { count: s.count_hint })}</span>{/if}
          <button class="btn sm" onclick={() => open(s)} disabled={busy}>{t("imp.preview")}</button>
        </div>
      {/each}
    </div>
  {/if}
  {@render passwordPrompt()}
  <div class="actions split">
    <button class="btn ghost" onclick={() => goto("start")}>{t("imp.back")}</button>
    <button class="btn ghost" onclick={() => goto("file")}>{t("imp.notListed")}</button>
  </div>
{:else if step === "file"}
  <form class="fileform" onsubmit={openFile}>
    <label class="field">
      <span class="label">{t("imp.tool")}</span>
      <select class="select" bind:value={fileTool} onchange={() => (pwRetry = null)}>
        {#each IMPORT_TOOLS as tool (tool)}<option value={tool}>{toolLabel(tool)}</option>{/each}
      </select>
    </label>
    <label class="field">
      <span class="label">{t("imp.path")}</span>
      <input class="input mono" bind:value={filePath} placeholder={t("imp.pathPlaceholder")} spellcheck="false" autocomplete="off" />
      <span class="hint">{t(`imp.pathHint.${fileTool}`)} · {t("imp.pathAbsolute")}</span>
    </label>
    {#if fileTool === "mremoteng"}
      <label class="field">
        <span class="label">{t("imp.mrngPassword")}</span>
        <input class="input" type="password" bind:value={mremotengPassword} bind:this={mrngField} oninput={() => (pwRetry = null)} autocomplete="off" aria-invalid={!!pwRetry} />
        {#if pwRetry}
          <span class="hint err" role="alert">{t(pwRetry.wrong ? "imp.mrngWrong" : "imp.mrngNeeded")}</span>
        {:else}
          <span class="hint">{t("imp.mrngOptional")}</span>
        {/if}
      </label>
    {/if}
    <label class="check"><input type="checkbox" bind:checked={importPasswords} /><span>{t("imp.passwords")}</span></label>
    {#if fileTool === "securecrt"}<p class="hint">{t("imp.securecrtNote")}</p>{/if}
    <div class="actions split">
      <button type="button" class="btn ghost" onclick={() => goto("start")}>{t("imp.back")}</button>
      <button class="btn primary" disabled={busy || !filePath.trim()}>{t("imp.preview")}</button>
    </div>
  </form>
{:else if preview}
  <h3>{toolLabel(preview.tool)}</h3>
  {#if preview.warnings.length}
    <ul class="warnings">
      {#each preview.warnings as w, i (i)}<li><Icon name="alert" size={12} />{w}</li>{/each}
    </ul>
  {/if}
  <div class="card tree">
    {#each grouped as g (g.path)}
      <div class="gpath"><Icon name="folder" size={12} />{g.path}</div>
      {#each g.items as { c, i } (i)}
        <label class="list-row item" class:dup={dupIndex.has(i)}>
          <input type="checkbox" checked={selected.has(i)} onchange={() => toggle(i)} />
          <span class="cname">{c.name}</span>
          <span class="path mono">{c.user}@{c.host}{c.port !== 22 ? `:${c.port}` : ""}</span>
          {#if dupIndex.has(i)}<span class="pill warn" title={t("imp.duplicateHint")}>{t("imp.duplicate")}</span>{/if}
        </label>
      {/each}
    {/each}
  </div>
  {#if preview.duplicates.length}<p class="hint dupnote">{t("imp.duplicateHint")}</p>{/if}
  <div class="actions">
    <button class="btn ghost" onclick={() => goto(origin)}>{t("imp.back")}</button>
    <button class="btn primary" onclick={apply} disabled={busy || !selected.size}>{t("imp.apply", { count: selected.size })}</button>
  </div>
{/if}

<style>
  .tool {
    font-weight: 550;
    min-width: 90px;
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
    color: var(--faint);
  }
  .actions.start {
    justify-content: flex-start;
    margin-top: 0;
  }
  .actions.split {
    justify-content: space-between;
  }
  .fileform {
    display: grid;
    gap: 12px;
    max-width: 520px;
  }
  .pwprompt {
    display: grid;
    gap: 8px;
    padding: 12px;
    margin-top: 12px;
  }
  .pwprompt .hint {
    margin: 0;
  }
  .hint.err {
    color: var(--danger);
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .row .input {
    flex: 1;
    min-width: 0;
  }
  .warnings {
    list-style: none;
    margin: 0 0 12px;
    padding: 0;
    display: grid;
    gap: 4px;
    font-size: var(--fs-meta);
    color: var(--warn);
  }
  .warnings li {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .tree {
    max-height: 320px;
    overflow: auto;
  }
  .gpath {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 9px 12px 4px;
    font-size: var(--fs-small);
    color: var(--muted);
  }
  .item {
    padding-left: 26px;
    cursor: pointer;
    border-top: 0 !important;
  }
  .item input {
    accent-color: var(--accent);
  }
  .item.dup .cname {
    color: var(--muted);
  }
  .cname {
    min-width: 110px;
  }
  .dupnote {
    margin: 8px 0 0;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
</style>
