<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { settings } from "../../stores/settings.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { vault } from "../../stores/vault.svelte";

  const options = [5, 15, 30, 60, 0];

  async function setAutoLock(min: number) {
    settings.autoLock = min;
  }
  async function lock() {
    ui.settingsPage = null;
    await vault.lock().catch(toasts.error);
  }
</script>

<button class="btn" onclick={lock}><Icon name="lock" size={14} />{t("vaultset.lockNow")}</button>

<h3>{t("vaultset.autoLock")}</h3>
<div class="seg" role="group" aria-label={t("vaultset.autoLock")}>
  {#each options as m (m)}
    <button aria-pressed={settings.autoLock === m} onclick={() => setAutoLock(m)}>{m ? t("vaultset.minutes", { n: m }) : t("vaultset.never")}</button>
  {/each}
</div>

<h3>{t("vaultset.changePassword")}</h3>
<p class="hint">{t("vaultset.changePasswordSoon")}</p>
<button class="btn" disabled>{t("vaultset.changePassword")} <span class="pill">{t("common.soon")}</span></button>
