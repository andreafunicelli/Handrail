<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { Identity, SecretKind, SshAuth } from "../../api/types";
  import { segmented } from "../../actions/segmented";
  import { credentials } from "../../stores/credentials.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { uuid } from "../../util/id";

  /** Add or edit an identity: a user name with a password, a key or the SSH agent. A typed
   *  password or key is saved as a new vault secret; a saved one can be picked instead. */
  let { identity, onclose }: { identity: Identity | null; onclose: () => void } = $props();

  // The dialog edits a copy: it is opened afresh for each identity.
  // svelte-ignore state_referenced_locally
  const initial = identity;
  const start = initial?.auth;
  let label = $state(initial?.label ?? "");
  let user = $state(initial?.user ?? "");
  let authType = $state<"password" | "key" | "agent">(start && start.type !== "prompt" ? start.type : "password");
  let passwordId = $state(start?.type === "password" ? start.secret : (credentials.passwords[0]?.id ?? ""));
  let keyId = $state(start?.type === "key" ? start.key : (credentials.keys[0]?.id ?? ""));
  let password = $state("");
  let keyText = $state("");
  let passphrase = $state("");
  let busy = $state(false);
  let fileInput = $state<HTMLInputElement>();
  const keptPassphrase = start?.type === "key" ? start.passphrase : null;

  // A new identity starts with a new password/key when nothing is saved yet.
  if (!initial) {
    if (!credentials.passwords.length) passwordId = "";
    if (!credentials.keys.length) keyId = "";
  }

  const others = (id: string) => credentials.usedBy(id).filter((n) => n !== identity?.label);
  const authOk = $derived(authType === "agent" || (authType === "password" ? !!(password || passwordId) : !!(keyText.trim() || keyId)));
  const valid = $derived(label.trim() !== "" && user.trim() !== "" && authOk);
  const users = $derived(identity ? credentials.identityUsers(identity.id) : []);

  async function secret(kind: SecretKind, value: string, name: string) {
    return api.saveSecret({ id: uuid(), kind, label: name, value });
  }

  async function save(ev: SubmitEvent) {
    ev.preventDefault();
    if (!valid || busy) return;
    busy = true;
    try {
      const name = label.trim();
      let auth: SshAuth;
      if (authType === "password") {
        auth = { type: "password", secret: password ? await secret("password", password, `${name} password`) : passwordId };
      } else if (authType === "key") {
        if (keyText.trim()) {
          const key = await secret("private_key", keyText, `${name} key`);
          auth = { type: "key", key, passphrase: passphrase ? await secret("passphrase", passphrase, `${name} passphrase`) : null };
        } else {
          // A saved key keeps the passphrase stored with it, unless a new one is typed.
          const kept = start?.type === "key" && start.key === keyId ? keptPassphrase : credentials.passphraseFor(keyId);
          auth = { type: "key", key: keyId, passphrase: passphrase ? await secret("passphrase", passphrase, `${credentials.label(keyId) || name} passphrase`) : kept };
        }
      } else {
        auth = { type: "agent" };
      }
      password = keyText = passphrase = "";
      const saved = await credentials.saveIdentity({ id: identity?.id ?? uuid(), label: name, user: user.trim(), auth });
      // New secrets show up in the keys and passwords list.
      await credentials.load();
      toasts.success(t("ident.saved", { name: saved.label }));
      onclose();
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
  }

  async function pickKey(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    if (f) keyText = await f.text();
  }
</script>

<Modal label={identity ? t("ident.editTitle", { name: identity.label }) : t("ident.addTitle")} {onclose} width="480px">
  <form class="dialog" onsubmit={save}>
    <h2>{identity ? t("ident.editTitle", { name: identity.label }) : t("ident.addTitle")}</h2>
    {#if users.length}<p class="hint">{t("ident.editUsers", { names: users.join(", ") })}</p>{/if}
    <label class="field">
      <span class="label">{t("cred.name")}</span>
      <input class="input" bind:value={label} placeholder={t("ident.namePlaceholder")} data-autofocus />
    </label>
    <label class="field">
      <span class="label">{t("editor.user")}</span>
      <input class="input mono" bind:value={user} placeholder="deploy" spellcheck="false" autocapitalize="off" />
    </label>
    <fieldset class="field">
      <legend class="label">{t("editor.auth")}</legend>
      <div class="seg" role="group" use:segmented>
        {#each ["password", "key", "agent"] as const as a (a)}
          <button type="button" aria-pressed={authType === a} onclick={() => (authType = a)}>{t(`editor.auth.${a}`)}</button>
        {/each}
      </div>
      {#if authType === "password"}
        {#if credentials.passwords.length}
          <select class="select" bind:value={passwordId} aria-label={t("editor.savedPassword")}>
            <option value="">{t("editor.newPassword")}</option>
            {#each credentials.passwords as p (p.id)}<option value={p.id}>{p.label}{others(p.id).length ? ` — ${t("editor.usedBy", { names: others(p.id).join(", ") })}` : ""}</option>{/each}
          </select>
        {/if}
        {#if !passwordId}
          <input class="input" type="password" autocomplete="new-password" bind:value={password} placeholder={t("editor.auth.password")} aria-label={t("editor.auth.password")} />
        {/if}
      {:else if authType === "key"}
        {#if credentials.keys.length}
          <select class="select" bind:value={keyId} aria-label={t("editor.savedKey")}>
            <option value="">{t("editor.newKey")}</option>
            {#each credentials.keys as k (k.id)}<option value={k.id}>{k.label}{others(k.id).length ? ` — ${t("editor.usedBy", { names: others(k.id).join(", ") })}` : ""}</option>{/each}
          </select>
        {/if}
        {#if !keyId}
          <textarea class="textarea mono" rows="4" bind:value={keyText} placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" spellcheck="false" aria-label={t("editor.auth.key")}></textarea>
          <div>
            <button type="button" class="btn sm" onclick={() => fileInput?.click()}><Icon name="key" size={13} />{t("editor.keyPick")}</button>
            <input type="file" class="visually-hidden" bind:this={fileInput} onchange={pickKey} tabindex="-1" />
          </div>
        {/if}
        <input class="input" type="password" bind:value={passphrase} placeholder={keyId ? t("editor.passphraseKeep") : t("editor.passphrase")} autocomplete="off" aria-label={t("editor.passphrase")} />
      {:else}
        <p class="hint">{t("editor.agentNote")}</p>
      {/if}
    </fieldset>
    <div class="actions">
      <button type="button" class="btn ghost" onclick={onclose}>{t("common.cancel")}</button>
      <button class="btn primary" disabled={!valid || busy}>{identity ? t("cred.save") : t("ident.add")}</button>
    </div>
  </form>
</Modal>

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
  .hint {
    margin: 0;
  }
  fieldset {
    border: 0;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 8px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
