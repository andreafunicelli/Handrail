<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t, fmtDateTime } from "../../i18n";
  import { errorMessage } from "../../api/errors";
  import type { FileEntry } from "../../api/types";
  import { looksBinary } from "../../util/fileBackends";
  import { modeString } from "../../util/files";
  import { PREVIEW_MAX, previewBlock } from "../../util/fileOps";
  import { compareTexts, type DiffResult } from "../../util/lineDiff";
  import { formatBytes } from "../../util/time";
  import type { CompareRequest } from "./compare.svelte";

  /** Before an overwrite: what is there and what would replace it, side by side. */
  let { req, onanswer }: { req: CompareRequest; onanswer: (replace: boolean) => void } = $props();

  const folder = $derived(req.src.entry?.kind === "dir" || req.dst.entry?.kind === "dir");
  const block = $derived(
    previewBlock(
      { entry: req.src.entry, readable: !!req.src.fs?.readText, local: !req.src.fs },
      { entry: req.dst.entry, readable: !!req.dst.fs?.readText, local: !req.dst.fs },
      looksBinary,
    ),
  );
  const BLOCK_TEXT = {
    local: "files.compare.noDiffLocal",
    folder: "files.compare.noDiffFolder",
    unknown: "files.compare.noDiffUnknown",
    tooLarge: "files.compare.noDiffLarge",
    binary: "files.compare.noDiffBinary",
    unreadable: "files.compare.noDiffUnreadable",
  } as const;

  let diff = $state<DiffResult | null>(null);
  let diffError = $state<string | null>(null);
  let loading = $state(false);

  async function loadDiff() {
    const [s, d] = [req.src, req.dst];
    if (block || !s.fs?.readText || !d.fs?.readText) return;
    loading = true;
    diffError = null;
    try {
      const [a, b] = await Promise.all([d.fs.readText(d.path), s.fs.readText(s.path)]);
      // The listing may be stale: never show more than the limit promised.
      if (a.text.length > PREVIEW_MAX * 4 || b.text.length > PREVIEW_MAX * 4) diffError = t("files.compare.noDiffLarge");
      else diff = compareTexts(a.text, b.text, { maxLines: 5000 });
    } catch (e) {
      diffError = t("files.compare.diffFailed", { error: errorMessage(e) });
    } finally {
      loading = false;
    }
  }

  const kindOf = (e: FileEntry | null) => (!e ? "—" : e.kind === "dir" ? t("files.compare.typeFolder") : e.link ? t("files.compare.typeLink") : e.kind === "file" ? t("files.compare.typeFile") : t("files.compare.typeOther"));
  const sizeOf = (e: FileEntry | null) => (!e ? "—" : e.kind === "file" ? `${formatBytes(e.size)} (${e.size.toLocaleString()} B)` : "—");
  const timeOf = (e: FileEntry | null) => (e?.modified ? fmtDateTime(e.modified * 1000) : "—");
  const permOf = (e: FileEntry | null) => (e?.permissions != null ? modeString(e.permissions) : "—");

  /** A hint when the file being replaced looks like the newer one. */
  const dstNewer = $derived(!!req.src.entry?.modified && !!req.dst.entry?.modified && req.dst.entry.modified > req.src.entry.modified);
  const title = $derived(t("files.compare.title", { name: req.name }));
</script>

<Modal label={title} onclose={() => onanswer(false)} width="760px">
  <div class="dialog">
    <h2>{title}</h2>
    <div class="warn" role="note">
      <Icon name="alert" size={15} />
      <span>{t(folder ? "files.compare.warnFolder" : "files.compare.warnFile")}</span>
    </div>

    <table class="meta">
      <caption class="sr-only">{t("files.compare.tableCaption")}</caption>
      <thead>
        <tr>
          <th scope="col"><span class="sr-only">{t("files.compare.property")}</span></th>
          <th scope="col">{t(req.mode === "upload" ? "files.compare.sourceLocal" : "files.compare.source")}</th>
          <th scope="col">{t("files.compare.destination")}</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <th scope="row">{t("files.compare.where")}</th>
          <td>{req.src.label}</td>
          <td>{req.dst.label}</td>
        </tr>
        <tr>
          <th scope="row">{t("files.compare.path")}</th>
          <td class="mono path">{req.src.path}</td>
          <td class="mono path">{req.dst.path}</td>
        </tr>
        <tr>
          <th scope="row">{t("files.compare.size")}</th>
          <td>{req.src.fs ? sizeOf(req.src.entry) : t("files.compare.unknownLocal")}</td>
          <td>{sizeOf(req.dst.entry)}</td>
        </tr>
        <tr>
          <th scope="row">{t("files.compare.modified")}</th>
          <td>{req.src.fs ? timeOf(req.src.entry) : t("files.compare.unknownLocal")}</td>
          <td class:newer={dstNewer}>{timeOf(req.dst.entry)}</td>
        </tr>
        <tr>
          <th scope="row">{t("files.compare.type")}</th>
          <td>{req.src.fs ? kindOf(req.src.entry) : t("files.compare.typeFile")}</td>
          <td>{kindOf(req.dst.entry)}</td>
        </tr>
        <tr>
          <th scope="row">{t("files.compare.permissions")}</th>
          <td class="mono">{req.src.fs ? permOf(req.src.entry) : "—"}</td>
          <td class="mono">{permOf(req.dst.entry)}</td>
        </tr>
      </tbody>
    </table>
    {#if dstNewer}<p class="hint newer-hint">{t("files.compare.destNewer")}</p>{/if}
    {#if !req.dst.entry}<p class="hint">{t("files.compare.destUnlisted")}</p>{/if}

    <section class="diff" aria-label={t("files.compare.contents")}>
      {#if block}
        <p class="hint">{t(BLOCK_TEXT[block], { max: formatBytes(PREVIEW_MAX) })}</p>
      {:else if diffError}
        <p class="hint">{diffError}</p>
      {:else if !diff}
        <button class="btn sm" onclick={loadDiff} disabled={loading}>
          <Icon name="file" size={13} />{loading ? t("common.loading") : t("files.compare.showDiff")}
        </button>
        <p class="hint">{t("files.compare.diffHint", { max: formatBytes(PREVIEW_MAX) })}</p>
      {:else if !diff.ok}
        <p class="hint">{t(diff.reason === "lines" ? "files.compare.diffTooLong" : "files.compare.diffTooDifferent")}</p>
      {:else if diff.identical}
        <p class="hint ok">{t("files.compare.identical")}</p>
      {:else if !diff.added && !diff.removed}
        <p class="hint">{t("files.compare.onlyEndings")}</p>
      {:else}
        <p class="stats" aria-live="polite">
          <span class="add">{t("files.compare.added", { n: diff.added })}</span>
          <span class="del">{t("files.compare.removed", { n: diff.removed })}</span>
          <span class="legend">{t("files.compare.legend")}</span>
        </p>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="lines mono" role="table" tabindex="0" aria-label={t("files.compare.contents")}>
          {#each diff.hunks as h, hi (hi)}
            {#if h.skipped}<div class="skip" role="row"><span role="cell">{t("files.compare.skipped", { n: h.skipped })}</span></div>{/if}
            {#each h.lines as l, li (li)}
              <div class="ln {l.op}" role="row">
                <span class="no" role="cell">{l.a ?? ""}</span>
                <span class="no" role="cell">{l.b ?? ""}</span>
                <span class="mark" role="cell" aria-label={l.op === "add" ? t("files.compare.lineAdded") : l.op === "del" ? t("files.compare.lineRemoved") : undefined}>{l.op === "add" ? "+" : l.op === "del" ? "−" : " "}</span>
                <span class="tx" role="cell">{l.text}</span>
              </div>
            {/each}
          {/each}
        </div>
      {/if}
    </section>

    <div class="actions">
      <button type="button" class="btn ghost" onclick={() => onanswer(false)} data-autofocus>{t("files.compare.skip")}</button>
      <button type="button" class="btn danger" onclick={() => onanswer(true)}>{t(folder ? "files.compare.merge" : "files.compare.replace")}</button>
    </div>
  </div>
</Modal>

<style>
  .dialog {
    display: grid;
    gap: var(--sp-3, 12px);
    padding: 20px;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: var(--fs-title);
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .warn {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--warn);
    border-radius: var(--r);
    background: var(--warn-soft);
    color: var(--text);
    font-size: var(--fs-meta);
  }
  .warn :global(svg) {
    flex: none;
    color: var(--warn);
    margin-top: 1px;
  }
  .meta {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: var(--fs-meta);
  }
  .meta th,
  .meta td {
    padding: 4px 8px;
    text-align: left;
    vertical-align: top;
    border-bottom: 1px solid var(--border);
    overflow-wrap: anywhere;
  }
  .meta thead th {
    color: var(--muted);
    font-weight: 600;
  }
  .meta thead th:first-child {
    width: 8.5em;
  }
  .meta tbody th {
    color: var(--faint);
    font-weight: 500;
  }
  .path {
    font-size: var(--fs-small);
  }
  .newer {
    color: var(--warn);
  }
  .hint {
    margin: 0;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  .newer-hint {
    color: var(--warn);
  }
  .hint.ok {
    color: var(--ok);
  }
  .diff {
    display: grid;
    gap: 8px;
    justify-items: start;
    min-width: 0;
  }
  .stats {
    display: flex;
    gap: 12px;
    margin: 0;
    font-size: var(--fs-meta);
  }
  .stats .add {
    color: var(--ok);
  }
  .stats .del {
    color: var(--danger);
  }
  .legend {
    color: var(--faint);
  }
  .lines {
    width: 100%;
    max-height: 300px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--r);
    font-size: var(--fs-small);
    background: var(--bg);
  }
  .ln {
    display: grid;
    grid-template-columns: 4ch 4ch 2ch minmax(0, 1fr);
    gap: 6px;
    padding: 0 8px;
    white-space: pre;
  }
  .ln.add {
    background: var(--diff-add);
  }
  .ln.del {
    background: var(--diff-del);
  }
  .no {
    color: var(--faint);
    text-align: right;
    user-select: none;
  }
  .mark {
    color: var(--muted);
    user-select: none;
  }
  .tx {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .skip {
    padding: 2px 8px;
    color: var(--faint);
    background: var(--hover);
    font-style: italic;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
