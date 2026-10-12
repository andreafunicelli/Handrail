<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";
  import { toasts } from "../../stores/toasts.svelte";
  import { vault } from "../../stores/vault.svelte";

  /** Confirm deleting the vault from this device; the word must be typed, nothing else undoes it. */
  let { onclose }: { onclose: () => void } = $props();

  const word = t("vaultdel.word");
  let typed = $state("");
  let busy = $state(false);
  const ok = $derived(typed.trim().toLowerCase() === word.toLowerCase());

  async function go(e: SubmitEvent) {
    e.preventDefault();
    if (!ok || busy) return;
    busy = true;
    try {
      // Reloads the app on success.
      await vault.erase();
    } catch (err) {
      busy = false;
      toasts.error(err);
    }
  }
</script>

<Modal label={t("vaultdel.title")} onclose={() => !busy && onclose()} tone="danger" width="460px" dismissable={!busy}>
  <form class="pad" onsubmit={go}>
    <h2>{t("vaultdel.title")}</h2>
    <p>{t("vaultdel.body")}</p>
    <ul>
      <li>{t("vaultdel.items")}</li>
      <li>{t("vaultdel.secrets")}</li>
      <li>{t("vaultdel.other")}</li>
    </ul>
    <p class="hint">{t("vaultdel.synced")}</p>
    <label class="field">
      <span class="label">{t("vaultdel.type", { word })}</span>
      <input class="input mono" bind:value={typed} placeholder={word} autocomplete="off" spellcheck="false" disabled={busy} data-autofocus />
    </label>
    <div class="actions">
      <button type="button" class="btn ghost" onclick={onclose} disabled={busy}>{t("common.cancel")}</button>
      <button type="submit" class="btn danger" disabled={!ok || busy}>{busy ? t("vaultdel.busy") : t("vaultdel.go")}</button>
    </div>
  </form>
</Modal>

<style>
  .pad {
    display: grid;
    gap: 12px;
    padding: 20px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  p,
  ul {
    margin: 0;
  }
  ul {
    padding-left: 18px;
    display: grid;
    gap: 4px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
