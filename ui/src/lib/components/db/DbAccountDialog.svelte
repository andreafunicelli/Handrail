<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import DbGrid from "./DbGrid.svelte";
  import { api } from "../../api";
  import type { DbAccountChange, DbResultSet } from "../../api/types";
  import { t } from "../../i18n";
  import { toasts } from "../../stores/toasts.svelte";
  import { errorMessage } from "../../api/errors";

  /** Create an account, change its password, drop it, or grant and revoke privileges. The
   *  backend builds the SQL; the preview shows it with the password masked. */
  let {
    id,
    dialect,
    mode,
    user: initialUser = "",
    host: initialHost = null,
    namespaces,
    prod,
    ondone,
    onclose,
  }: {
    id: string;
    dialect: "mysql" | "postgres";
    mode: "create" | "password" | "drop" | "grants";
    user?: string;
    host?: string | null;
    namespaces: string[];
    prod: boolean;
    ondone: () => void;
    onclose: () => void;
  } = $props();

  const PRIVS = {
    mysql: ["ALL PRIVILEGES", "SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "DROP", "ALTER", "INDEX", "REFERENCES", "TRIGGER", "EXECUTE", "CREATE VIEW", "SHOW VIEW"],
    postgres: ["ALL PRIVILEGES", "SELECT", "INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER"],
  };

  // svelte-ignore state_referenced_locally
  let user = $state(initialUser);
  // svelte-ignore state_referenced_locally
  let host = $state(initialHost ?? (dialect === "mysql" ? "%" : ""));
  let password = $state("");
  let again = $state("");
  let typed = $state("");
  let privileges = $state<string[]>([]);
  // svelte-ignore state_referenced_locally
  let namespace = $state<string>(dialect === "postgres" ? (namespaces.includes("public") ? "public" : (namespaces[0] ?? "")) : "");
  let grantOp = $state<"grant" | "revoke">("grant");
  let preview = $state<string[] | null>(null);
  let previewError = $state<string | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let grants = $state<DbResultSet | null>(null);

  const label = $derived(dialect === "mysql" ? `'${user}'@'${host || "%"}'` : user);
  const change = $derived.by((): DbAccountChange | null => {
    const h = dialect === "mysql" ? host : null;
    if (!user.trim()) return null;
    if (mode === "create" || mode === "password") {
      if (!password || password !== again) return null;
      return { op: mode, user: user.trim(), host: h, password };
    }
    if (mode === "drop") return { op: "drop", user: user.trim(), host: h };
    if (!privileges.length) return null;
    return { op: grantOp, user: user.trim(), host: h, privileges, namespace: namespace || null };
  });
  /** Destructive or production: the account's name has to be typed. */
  const needsName = $derived(mode === "drop" || (mode === "grants" && grantOp === "revoke") || prod);
  const ready = $derived(!!change && !!preview && !busy && (!needsName || typed === user.trim()));

  // The preview follows the form (debounced; the password is masked by the backend).
  $effect(() => {
    const c = change;
    preview = null;
    previewError = null;
    if (!c) return;
    const snapshot = $state.snapshot(c) as DbAccountChange;
    const timer = setTimeout(async () => {
      try {
        preview = await api.dbAccountPreview(id, snapshot);
      } catch (e) {
        previewError = errorMessage(e);
      }
    }, 150);
    return () => clearTimeout(timer);
  });

  async function loadGrants() {
    try {
      grants = await api.dbGrants(id, user.trim(), dialect === "mysql" ? host : null);
    } catch (e) {
      error = errorMessage(e);
    }
  }
  // svelte-ignore state_referenced_locally
  if (mode === "grants") void loadGrants();

  async function submit(e: Event) {
    e.preventDefault();
    if (!ready || !change) return;
    busy = true;
    error = null;
    try {
      await api.dbAccount(id, $state.snapshot(change) as DbAccountChange);
      password = "";
      again = "";
      toasts.success(t("db.acc.done"));
      ondone();
      if (mode === "grants") {
        typed = "";
        await loadGrants();
      } else onclose();
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  function toggle(p: string) {
    privileges = privileges.includes(p) ? privileges.filter((x) => x !== p) : [...privileges, p];
  }
</script>

<Modal label={t(`db.acc.${mode}`)} {onclose} tone={mode === "drop" ? "danger" : "default"} width="560px">
  <form class="pad" onsubmit={submit} autocomplete="off">
    <h2 class:danger={mode === "drop"}>{t(`db.acc.${mode}`)}{mode === "create" ? "" : ` · ${label}`}</h2>

    {#if mode === "create"}
      <div class="row">
        <label class="field"><span class="label">{t("db.acc.user")}</span><input class="input mono" bind:value={user} spellcheck="false" /></label>
        {#if dialect === "mysql"}
          <label class="field"><span class="label">{t("db.acc.host")}</span><input class="input mono" bind:value={host} spellcheck="false" placeholder="%" /></label>
        {/if}
      </div>
    {/if}

    {#if mode === "create" || mode === "password"}
      <div class="row">
        <label class="field"><span class="label">{t("db.acc.password")}</span><input class="input" type="password" bind:value={password} autocomplete="new-password" /></label>
        <label class="field"><span class="label">{t("db.acc.again")}</span><input class="input" type="password" bind:value={again} autocomplete="new-password" /></label>
      </div>
      {#if again && password !== again}<p class="warn">{t("db.acc.mismatch")}</p>{/if}
    {/if}

    {#if mode === "drop"}
      <p>{t(dialect === "mysql" ? "db.acc.dropBodyMysql" : "db.acc.dropBodyPg")}</p>
    {/if}

    {#if mode === "grants"}
      <span class="label">{t("db.acc.current")}</span>
      <div class="grants">{#if grants}<DbGrid result={grants} />{:else}<p class="hint">{t("db.loading")}</p>{/if}</div>
      <div class="row">
        <div class="seg" role="group">
          <button type="button" aria-pressed={grantOp === "grant"} onclick={() => (grantOp = "grant")}>{t("db.acc.grant")}</button>
          <button type="button" aria-pressed={grantOp === "revoke"} onclick={() => (grantOp = "revoke")}>{t("db.acc.revoke")}</button>
        </div>
        <label class="field">
          <span class="label">{t(dialect === "mysql" ? "db.acc.onDatabase" : "db.acc.onSchema")}</span>
          <select class="select" bind:value={namespace}>
            {#if dialect === "mysql"}<option value="">{t("db.acc.everything")}</option>{/if}
            {#each namespaces as ns (ns)}<option value={ns}>{ns}</option>{/each}
          </select>
        </label>
      </div>
      <div class="privs">
        {#each PRIVS[dialect] as p (p)}
          <label class="priv"><input type="checkbox" checked={privileges.includes(p)} onchange={() => toggle(p)} />{p}</label>
        {/each}
      </div>
    {/if}

    <span class="label">{t("db.ddl.sql")}</span>
    <pre class="sql">{preview ? preview.join("\n") : (previewError ?? "…")}</pre>

    {#if needsName && change}
      <label class="field">
        <span class="label">{t("db.ddl.typeToConfirm", { name: user.trim() })}</span>
        <input class="input mono" bind:value={typed} placeholder={user.trim()} spellcheck="false" />
      </label>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="btn ghost" onclick={onclose}>{t("common.close")}</button>
      <button type="submit" class="btn" class:danger={mode === "drop" || grantOp === "revoke"} class:primary={mode !== "drop" && grantOp !== "revoke"} disabled={!ready}>
        {busy ? t("db.running") : t("db.ddl.run")}
      </button>
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
    color: var(--muted);
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: flex-end;
  }
  .field {
    display: grid;
    gap: 4px;
    flex: 1;
  }
  .label {
    font-size: 12px;
    color: var(--muted);
  }
  .grants {
    height: 120px;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg {
    display: flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg button {
    border: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    padding: 5px 10px;
    cursor: pointer;
  }
  .seg button[aria-pressed="true"] {
    background: var(--accent-soft);
    color: var(--text);
  }
  .privs {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 4px 10px;
    font-size: 12px;
  }
  .priv {
    display: flex;
    gap: 5px;
    align-items: center;
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
  .warn {
    color: var(--warn);
    font-size: 12px;
  }
  .error {
    color: var(--danger);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
  }
  .hint {
    padding: 8px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
</style>
