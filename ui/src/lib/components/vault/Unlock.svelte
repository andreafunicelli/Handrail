<script lang="ts">
  import VaultFrame from "./VaultFrame.svelte";
  import DeleteVaultDialog from "./DeleteVaultDialog.svelte";
  import { t } from "../../i18n";
  import { vault } from "../../stores/vault.svelte";
  import { errorMessage, isSecretKeyRequired } from "../../api/errors";

  let password = $state("");
  let secretKey = $state("");
  let newDevice = $state(vault.joined || (vault.status.needs_secret_key ?? false));
  let busy = $state(false);
  let error = $state<string | null>(null);
  let forgot = $state(false);
  let deleting = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!password || busy) return;
    busy = true;
    error = null;
    try {
      await vault.unlock(password, newDevice ? secretKey.trim() : undefined);
      password = "";
    } catch (err) {
      if (isSecretKeyRequired(err)) {
        newDevice = true;
        error = t("vault.unlock.secretKeyHint");
      } else error = errorMessage(err);
    } finally {
      busy = false;
    }
  }
</script>

<VaultFrame>
  <h1>{t("vault.unlock.title")}</h1>
  <p class="lede">{t("vault.unlock.body")}</p>
  <form class="form" onsubmit={submit}>
    <label class="field">
      <span class="label">{t("vault.password")}</span>
      <input class="input lg" type="password" autocomplete="current-password" bind:value={password} aria-invalid={!!error} data-autofocus />
    </label>
    {#if newDevice}
      <label class="field">
        <span class="label">{t("vault.unlock.secretKey")}</span>
        <input class="input lg mono" placeholder="A3-XXXXX-XXXXX-…" bind:value={secretKey} autocomplete="off" spellcheck="false" />
        <span class="hint">{t("vault.unlock.secretKeyHint")}</span>
      </label>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <button class="btn primary lg" type="submit" disabled={!password || busy || (newDevice && !secretKey.trim())}>
      {busy ? t("vault.unlocking") : t("vault.unlock.submit")}
    </button>
    {#if !newDevice}
      <button type="button" class="link" onclick={() => (newDevice = true)}>{t("vault.unlock.newDevice")}</button>
    {/if}
    {#if forgot}
      <div class="forgot">
        <p class="hint">{t("vault.unlock.forgotBody")}</p>
        <button type="button" class="btn sm danger" onclick={() => (deleting = true)}>{t("vaultdel.go")}</button>
      </div>
    {:else}
      <button type="button" class="link" onclick={() => (forgot = true)}>{t("vault.unlock.forgot")}</button>
    {/if}
  </form>
</VaultFrame>

{#if deleting}<DeleteVaultDialog onclose={() => (deleting = false)} />{/if}

<style>
  .form {
    display: grid;
    gap: 14px;
  }
  .input.lg {
    height: 36px;
    font-size: 14px;
  }
  .err {
    margin: -4px 0 0;
    color: var(--danger);
    font-size: var(--fs-meta);
  }
  .link {
    justify-self: start;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  .link:hover {
    color: var(--text);
  }
  .forgot {
    display: grid;
    gap: 8px;
    justify-items: start;
  }
  .forgot .hint {
    margin: 0;
  }
</style>
