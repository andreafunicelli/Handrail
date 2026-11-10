<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";

  /** A change to a database's structure: the exact SQL is shown before it runs. With
   *  `confirmName`, the name has to be typed first (destructive changes, production). */
  let {
    title,
    body,
    input,
    sql,
    confirmName = null,
    danger = false,
    run,
    onclose,
  }: {
    title: string;
    body?: string;
    /** A value to ask for (a new name), passed to `sql`. */
    input?: { label: string; value: string };
    /** The SQL for the value typed; null when it is not valid yet. */
    sql: (value: string) => string | null;
    confirmName?: string | null;
    danger?: boolean;
    /** Runs the SQL; resolves to the server's error, or null when it worked. */
    run: (sql: string) => Promise<string | null>;
    onclose: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let value = $state(input?.value ?? "");
  let typed = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  const text = $derived(sql(value));
  const ready = $derived(!!text && !busy && (!confirmName || typed === confirmName));

  async function submit(e: Event) {
    e.preventDefault();
    if (!ready || !text) return;
    busy = true;
    error = null;
    try {
      error = await run(text);
      if (!error) onclose();
    } finally {
      busy = false;
    }
  }
</script>

<Modal label={title} {onclose} tone={danger ? "danger" : "default"} width="520px">
  <form class="pad" onsubmit={submit}>
    <h2 class:danger>{title}</h2>
    {#if body}<p>{body}</p>{/if}
    {#if input}
      <label class="field">
        <span class="label">{input.label}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input mono" bind:value autofocus spellcheck="false" autocapitalize="off" autocomplete="off" />
      </label>
    {/if}
    <span class="label">{t("db.ddl.sql")}</span>
    <pre class="sql">{text ?? "…"}</pre>
    {#if confirmName}
      <label class="field">
        <span class="label">{t("db.ddl.typeToConfirm", { name: confirmName })}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input mono" bind:value={typed} placeholder={confirmName} autofocus={!input} spellcheck="false" autocomplete="off" />
      </label>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="btn ghost" onclick={onclose}>{t("common.cancel")}</button>
      <button type="submit" class="btn" class:danger class:primary={!danger} disabled={!ready}>{busy ? t("db.running") : t("db.ddl.run")}</button>
    </div>
  </form>
</Modal>

<style>
  .pad {
    padding: 20px;
    display: grid;
    gap: 8px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  h2.danger {
    color: var(--danger);
  }
  p {
    margin: 0;
    color: var(--text-2, var(--muted));
  }
  .field {
    display: grid;
    gap: 4px;
  }
  .label {
    font-size: 12px;
    color: var(--muted);
  }
  .sql {
    margin: 0;
    padding: 8px 10px;
    border-radius: 6px;
    background: var(--bg);
    border: 1px solid var(--border);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .error {
    color: var(--danger);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
</style>
