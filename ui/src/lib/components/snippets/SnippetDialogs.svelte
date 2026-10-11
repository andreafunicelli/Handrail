<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";
  import { snippets } from "../../stores/snippets.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { fillPlaceholders, placeholders } from "../../util/snippets";

  // ---- editor ----
  let tagsText = $state("");
  $effect(() => {
    tagsText = snippets.editing?.snippet.tags.join(", ") ?? "";
  });
  const editVars = $derived(snippets.editing ? placeholders(snippets.editing.snippet.command) : []);

  async function save(e: SubmitEvent) {
    e.preventDefault();
    const ed = snippets.editing;
    if (!ed) return;
    ed.snippet.tags = tagsText
      .split(",")
      .map((x) => x.trim())
      .filter(Boolean);
    try {
      await snippets.save(ed.snippet);
      snippets.editing = null;
    } catch (err) {
      toasts.error(err);
    }
  }

  // ---- placeholder values ----
  let values = $state<Record<string, string>>({});
  const fillVars = $derived(snippets.filling ? placeholders(snippets.filling.snippet.command) : []);
  $effect(() => {
    void snippets.filling;
    values = {};
  });
  const preview = $derived(snippets.filling ? fillPlaceholders(snippets.filling.snippet.command, values) : "");

  async function fill(e: SubmitEvent) {
    e.preventDefault();
    const f = snippets.filling;
    if (!f) return;
    snippets.filling = null;
    await snippets.use(f.snippet, f.run, $state.snapshot(values)).catch(toasts.error);
  }
</script>

{#if snippets.editing}
  {@const ed = snippets.editing}
  <Modal label={t(ed.isNew ? "snip.newTitle" : "snip.editTitle")} onclose={() => (snippets.editing = null)} width="520px">
    <form class="dialog" onsubmit={save}>
      <h2>{t(ed.isNew ? "snip.newTitle" : "snip.editTitle")}</h2>
      <label class="field">
        <span class="label">{t("snip.name")}</span>
        <input class="input" bind:value={ed.snippet.name} placeholder="Disk usage" data-autofocus />
      </label>
      <label class="field">
        <span class="label">{t("snip.command")}</span>
        <textarea class="textarea mono" rows="4" bind:value={ed.snippet.command} placeholder={"journalctl -u {{service}} -n 100"} spellcheck="false"></textarea>
        <span class="hint">{editVars.length ? t("snip.varsFound", { vars: editVars.join(", ") }) : t("snip.varsHint")}</span>
      </label>
      <label class="field">
        <span class="label">{t("snip.description")}</span>
        <input class="input" bind:value={ed.snippet.description} />
      </label>
      <label class="field">
        <span class="label">{t("snip.tags")}</span>
        <input class="input" bind:value={tagsText} placeholder="logs, nginx" />
      </label>
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => (snippets.editing = null)}>{t("common.cancel")}</button>
        <button class="btn primary" disabled={!ed.snippet.name.trim() || !ed.snippet.command.trim()}>{t("common.save")}</button>
      </div>
    </form>
  </Modal>
{/if}

{#if snippets.filling}
  {@const f = snippets.filling}
  <Modal label={f.snippet.name} onclose={() => (snippets.filling = null)} width="460px">
    <form class="dialog" onsubmit={fill}>
      <h2>{f.snippet.name}</h2>
      {#each fillVars as v, i (v)}
        <label class="field">
          <span class="label mono">{v}</span>
          <input class="input mono" bind:value={values[v]} data-autofocus={i === 0 ? true : undefined} spellcheck="false" />
        </label>
      {/each}
      <pre class="preview mono">{preview}</pre>
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => (snippets.filling = null)}>{t("common.cancel")}</button>
        <button class="btn primary">{t(f.run ? "snip.run" : "snip.insert")}</button>
      </div>
    </form>
  </Modal>
{/if}

<style>
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
  .preview {
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--r);
    background: var(--bg);
    border: 1px solid var(--border);
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-all;
  }
</style>
