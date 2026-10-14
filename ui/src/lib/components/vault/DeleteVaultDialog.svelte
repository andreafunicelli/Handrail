<script lang="ts">
  import HoldButton from "../common/HoldButton.svelte";
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";
  import { toasts } from "../../stores/toasts.svelte";
  import { vault } from "../../stores/vault.svelte";

  /** Confirm deleting the vault from this device: the button must be held, since nothing undoes
   *  it. */
  let { onclose }: { onclose: () => void } = $props();

  let busy = $state(false);
  let hold: HoldButton;

  async function go() {
    if (busy) return;
    busy = true;
    try {
      // Reloads the app on success.
      await vault.erase();
    } catch (err) {
      busy = false;
      hold.reset();
      toasts.error(err);
    }
  }
</script>

<Modal label={t("vaultdel.title")} onclose={() => !busy && onclose()} tone="danger" width="460px" dismissable={!busy}>
  <div class="pad">
    <h2>{t("vaultdel.title")}</h2>
    <p>{t("vaultdel.body")}</p>
    <ul>
      <li>{t("vaultdel.items")}</li>
      <li>{t("vaultdel.secrets")}</li>
      <li>{t("vaultdel.other")}</li>
    </ul>
    <p class="hint">{t("vaultdel.synced")}</p>
    <div class="actions">
      <button type="button" class="btn ghost" onclick={onclose} disabled={busy} data-autofocus>{t("common.cancel")}</button>
      <HoldButton bind:this={hold} onconfirm={go} disabled={busy} confirmLabel={t("vaultdel.busy")} duration={2000}>{t("vaultdel.hold")}</HoldButton>
    </div>
  </div>
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
