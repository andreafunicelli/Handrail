<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import type { HostKeyEvent } from "../../api/types";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";

  let { req }: { req: HostKeyEvent } = $props();
  let typed = $state("");
  const hostOnly = $derived(req.host.replace(/:\d+$/, ""));

  const answer = (accept: boolean) => sessions.answerHostKey(req.request_id, accept).catch(toasts.error);
</script>

{#if req.kind === "new"}
  <Modal label={t("hostkey.new.title")} onclose={() => answer(false)}>
    <div class="pad">
      <div class="icon calm"><Icon name="shield" size={18} /></div>
      <h2>{t("hostkey.new.title")}</h2>
      <p>{t("hostkey.new.body", { host: req.host })}</p>
      <div class="fp">
        <span class="label">{t("hostkey.fingerprint")}{req.key_type ? ` · ${req.key_type}` : ""}</span>
        <code class="mono">{req.fingerprint}</code>
      </div>
      <div class="actions">
        <button class="btn ghost" onclick={() => answer(false)}>{t("hostkey.cancel")}</button>
        <button class="btn primary" data-autofocus onclick={() => answer(true)}>{t("hostkey.trust")}</button>
      </div>
    </div>
  </Modal>
{:else}
  <Modal label={t("hostkey.changed.title")} tone="danger" width="560px" dismissable={false}>
    <div class="pad">
      <div class="icon danger"><Icon name="alert" size={20} /></div>
      <h2 class="danger-text">{t("hostkey.changed.title")}</h2>
      <p>{t("hostkey.changed.body", { host: req.host })}</p>
      {#if req.old_fingerprint}
        <div class="fp old">
          <span class="label">{t("hostkey.changed.old")}</span>
          <code class="mono">{req.old_fingerprint}</code>
        </div>
      {/if}
      <div class="fp new">
        <span class="label">{t("hostkey.changed.new")}{req.key_type ? ` · ${req.key_type}` : ""}</span>
        <code class="mono">{req.fingerprint}</code>
      </div>
      <label class="field confirm">
        <span class="label">{t("hostkey.changed.confirm")}: <code class="mono">{hostOnly}</code></span>
        <input class="input mono" bind:value={typed} autocomplete="off" spellcheck="false" />
      </label>
      <div class="actions">
        <button class="btn" disabled={typed.trim() !== hostOnly} onclick={() => answer(true)}>{t("hostkey.changed.replace")}</button>
        <button class="btn danger" data-autofocus onclick={() => answer(false)}>{t("hostkey.changed.disconnect")}</button>
      </div>
    </div>
  </Modal>
{/if}

<style>
  .pad {
    padding: 22px 22px 18px;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 8px;
    margin-bottom: 14px;
  }
  .icon.calm {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .icon.danger {
    background: var(--danger-soft);
    color: var(--danger);
  }
  h2 {
    margin: 0 0 6px;
    font-size: 15px;
    font-weight: 600;
  }
  .danger-text {
    color: var(--danger);
  }
  p {
    margin: 0 0 16px;
    color: var(--text-2);
  }
  .fp {
    display: grid;
    gap: 4px;
    padding: 10px 12px;
    border-radius: var(--r);
    background: var(--bg);
    border: 1px solid var(--border);
    margin-bottom: 8px;
  }
  .fp code {
    font-size: 12px;
    overflow-wrap: anywhere;
    color: var(--text);
  }
  .fp.old code {
    color: var(--muted);
    text-decoration: line-through;
    text-decoration-color: var(--faint);
  }
  .fp.new {
    border-color: var(--danger);
  }
  .confirm {
    margin-top: 16px;
  }
  .confirm code {
    color: var(--text);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 20px;
    flex-wrap: wrap;
  }
</style>
