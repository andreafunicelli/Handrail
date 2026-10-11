<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";
  import { sudo } from "../../stores/sudo.svelte";

  function submit(ev: SubmitEvent) {
    ev.preventDefault();
    sudo.submit();
  }
</script>

{#if sudo.ask}
  {@const sa = sudo.ask}
  <Modal label={t("files.sudoTitle")} onclose={() => sudo.cancel()} width="380px">
    <form class="dialog" onsubmit={submit}>
      <h2>{t("files.sudoTitle")}</h2>
      <p class="hint">{t("files.sudoBody")}</p>
      <input class="input" type="password" bind:value={sa.password} aria-label={t("files.sudoPassword")} autocomplete="off" data-autofocus />
      {#if sa.error}<p class="err-line" role="alert">{sa.error}</p>{/if}
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => sudo.cancel()}>{t("common.cancel")}</button>
        <button class="btn primary" disabled={!sa.password || sa.busy}>{t("files.sudoGo")}</button>
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
  .hint {
    margin: 0;
    font-size: var(--fs-meta);
    color: var(--muted);
  }
  .err-line {
    margin: 0;
    font-size: var(--fs-meta);
    color: var(--danger);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
