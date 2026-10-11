<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { SecretInfo } from "../../api/types";
  import { credentials } from "../../stores/credentials.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";

  /** Keys and passwords connections log in with; one saved secret can serve many of them. */
  const shown = $derived(credentials.list.filter((s) => s.kind === "private_key" || s.kind === "password" || s.kind === "passphrase").sort((a, b) => a.label.localeCompare(b.label)));
  const unused = $derived(shown.filter((s) => !credentials.usedBy(s.id).length));

  let renaming = $state<{ id: string; value: string } | null>(null);
  let replacing = $state<{ secret: SecretInfo; value: string } | null>(null);
  let busy = $state(false);

  onMount(() => void credentials.load().catch(toasts.error));

  async function rename(ev: SubmitEvent) {
    ev.preventDefault();
    const r = renaming;
    if (!r?.value.trim()) return;
    try {
      await api.updateSecret(r.id, r.value.trim(), null);
      renaming = null;
      await credentials.load();
    } catch (e) {
      toasts.error(e);
    }
  }

  async function replace(ev: SubmitEvent) {
    ev.preventDefault();
    const r = replacing;
    if (!r?.value || busy) return;
    busy = true;
    try {
      await api.updateSecret(r.secret.id, null, r.value);
      replacing = null;
      const n = credentials.usedBy(r.secret.id).length;
      toasts.success(t("cred.replaced", { name: r.secret.label, n }));
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
  }

  async function remove(s: SecretInfo) {
    if (!(await ui.ask(t("cred.deleteTitle", { name: s.label }), { body: t("cred.deleteBody"), confirmLabel: t("common.delete"), danger: true }))) return;
    try {
      await api.deleteSecret(s.id);
      await credentials.load();
    } catch (e) {
      toasts.error(e);
    }
  }

  async function removeUnused() {
    const list = unused;
    if (!(await ui.ask(t("cred.cleanTitle", { n: list.length }), { body: t("cred.deleteBody"), confirmLabel: t("common.delete"), danger: true }))) return;
    for (const s of list) await api.deleteSecret(s.id).catch(toasts.error);
    await credentials.load();
  }

  async function pickFile(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    if (f && replacing) replacing.value = await f.text();
  }
  let fileInput = $state<HTMLInputElement>();

  const KIND_ICON = { private_key: "key", password: "lock", passphrase: "lock" } as const;
</script>

<p class="intro">{t("cred.body")}</p>

<div class="card">
  {#each shown as s (s.id)}
    {@const users = credentials.usedBy(s.id)}
    <div class="list-row">
      <Icon name={KIND_ICON[s.kind as keyof typeof KIND_ICON]} size={14} />
      {#if renaming?.id === s.id}
        <form class="rename" onsubmit={rename}>
          <input class="input sm" bind:value={renaming.value} aria-label={t("cred.name")} data-autofocus />
          <button class="btn sm primary" disabled={!renaming.value.trim()}>{t("cred.save")}</button>
          <button type="button" class="btn sm ghost" onclick={() => (renaming = null)}>{t("common.cancel")}</button>
        </form>
      {:else}
        <span class="what">
          <span class="name">{s.label}<span class="kind">{t(`cred.kind.${s.kind}` as "cred.kind.password")}</span></span>
          <span class="users" class:none={!users.length}>{users.length ? t("cred.usedBy", { names: users.join(", ") }) : t("cred.unused")}</span>
        </span>
        <button class="btn sm ghost" onclick={() => (renaming = { id: s.id, value: s.label })}>{t("cred.rename")}</button>
        <button class="btn sm ghost" onclick={() => (replacing = { secret: s, value: "" })}>{t("cred.replace")}</button>
        <button class="icon-btn" aria-label={t("common.delete")} title={users.length ? t("cred.inUse") : t("common.delete")} disabled={!!users.length} onclick={() => remove(s)}><Icon name="trash" size={14} /></button>
      {/if}
    </div>
  {:else}
    <div class="list-row muted">{t("cred.empty")}</div>
  {/each}
</div>
{#if unused.length > 1}
  <button class="btn sm ghost clean" onclick={removeUnused}><Icon name="trash" size={13} />{t("cred.clean", { n: unused.length })}</button>
{/if}

{#if replacing}
  {@const r = replacing}
  {@const users = credentials.usedBy(r.secret.id)}
  <Modal label={t("cred.replaceTitle", { name: r.secret.label })} onclose={() => (replacing = null)} width="460px">
    <form class="dialog" onsubmit={replace}>
      <h2>{t("cred.replaceTitle", { name: r.secret.label })}</h2>
      <p class="hint">{users.length ? t("cred.replaceBody", { names: users.join(", ") }) : t("cred.replaceUnused")}</p>
      {#if r.secret.kind === "private_key"}
        <textarea class="textarea mono" rows="6" bind:value={r.value} placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" spellcheck="false" data-autofocus></textarea>
        <div>
          <button type="button" class="btn sm" onclick={() => fileInput?.click()}><Icon name="key" size={13} />{t("editor.keyPick")}</button>
          <input type="file" class="visually-hidden" bind:this={fileInput} onchange={pickFile} tabindex="-1" />
        </div>
      {:else}
        <input class="input" type="password" autocomplete="new-password" bind:value={r.value} aria-label={t("cred.newValue")} placeholder={t("cred.newValue")} data-autofocus />
      {/if}
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => (replacing = null)}>{t("common.cancel")}</button>
        <button class="btn primary" disabled={!r.value || busy}>{t("cred.replaceGo")}</button>
      </div>
    </form>
  </Modal>
{/if}

<style>
  .what {
    flex: 1;
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .name {
    display: flex;
    align-items: baseline;
    gap: 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind {
    color: var(--faint);
    font-size: var(--fs-small);
  }
  .users {
    color: var(--muted);
    font-size: var(--fs-small);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .users.none {
    color: var(--faint);
  }
  .rename {
    flex: 1;
    display: flex;
    gap: 6px;
  }
  .rename .input {
    flex: 1;
    height: 28px;
  }
  .clean {
    margin-top: 8px;
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
  .hint {
    margin: 0;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
