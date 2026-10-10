<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { Connection, ImportPreview, ImportSource } from "../../api/types";
  import { connections } from "../../stores/connections.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { groupPath } from "../../util/tree";

  let step = $state<"start" | "sources" | "preview">("start");
  let sources = $state<ImportSource[]>([]);
  let busy = $state(false);
  let preview = $state<ImportPreview | null>(null);
  /** Indices (into preview.connections) the user chose to import. */
  let selected = $state<Set<number>>(new Set());

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

  async function open(src: ImportSource) {
    busy = true;
    try {
      preview = await api.importPreview({ source_id: src.id });
      selected = new Set(preview.connections.map((_, i) => i).filter((i) => !preview!.duplicates.some((d) => d.index === i)));
      step = "preview";
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
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

<p class="intro">{t("imp.body")}</p>

{#if step === "start"}
  <button class="btn primary" onclick={scan} disabled={busy}><Icon name="search" size={14} />{busy ? t("imp.scanning") : t("imp.detect")}</button>
{:else if step === "sources"}
  {#if !sources.length}
    <p class="hint">{t("imp.none")}</p>
  {:else}
    <div class="card">
      {#each sources as s (s.id)}
        <div class="list-row">
          <span class="tool">{s.tool}</span>
          <span class="path mono" title={s.path ?? ""}>{s.path ?? ""}</span>
          {#if s.count_hint != null}<span class="pill">{t("imp.found", { count: s.count_hint })}</span>{/if}
          <button class="btn sm" onclick={() => open(s)} disabled={busy}>{t("imp.preview")}</button>
        </div>
      {/each}
    </div>
  {/if}
  <button class="btn ghost back" onclick={() => (step = "start")}>{t("imp.back")}</button>
{:else if preview}
  <h3>{preview.tool}</h3>
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
    <button class="btn ghost" onclick={() => (step = "sources")}>{t("imp.back")}</button>
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
  .back {
    margin-top: 12px;
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
