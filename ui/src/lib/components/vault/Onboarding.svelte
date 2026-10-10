<script lang="ts">
  import VaultFrame from "./VaultFrame.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t, type MessageKey } from "../../i18n";
  import { passwordStrength, vault } from "../../stores/vault.svelte";
  import { toasts } from "../../stores/toasts.svelte";

  let password = $state("");
  let confirm = $state("");
  let busy = $state(false);
  let saved = $state(false);
  let copied = $state(false);

  const strength = $derived(passwordStrength(password));
  const mismatch = $derived(confirm.length > 0 && confirm !== password);
  const canSubmit = $derived(password.length >= 10 && password === confirm && strength >= 2 && !busy);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    busy = true;
    try {
      await vault.create(password);
      password = confirm = "";
    } catch (err) {
      toasts.error(err);
    } finally {
      busy = false;
    }
  }

  async function copy() {
    if (!vault.secretKey) return;
    try {
      await navigator.clipboard.writeText(vault.secretKey);
      copied = true;
      setTimeout(() => (copied = false), 1600);
    } catch (err) {
      toasts.error(err);
    }
  }

  function download() {
    if (!vault.secretKey) return;
    const body = `${t("vault.kit.fileHeader")}\n${new Date().toISOString().slice(0, 10)}\n\n${t("vault.kit.secretKey")}: ${vault.secretKey}\n\n${t("vault.kit.fileBody")}\n`;
    const url = URL.createObjectURL(new Blob([body], { type: "text/plain" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = "AIHubTerm-Emergency-Kit.txt";
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
</script>

<VaultFrame>
  {#if vault.phase === "kit" && vault.secretKey}
    <h1>{t("vault.kit.title")}</h1>
    <p class="lede">{t("vault.kit.body")}</p>

    <div class="kit">
      <div class="label">{t("vault.kit.secretKey")}</div>
      <div class="key mono" aria-label={t("vault.kit.secretKey")}>
        {#each vault.secretKey.split("-") as part, i (i)}<span>{part}</span>{/each}
      </div>
      <div class="kit-actions">
        <button class="btn sm" onclick={copy}><Icon name={copied ? "check" : "copy"} size={13} />{copied ? t("vault.kit.copied") : t("vault.kit.copy")}</button>
        <button class="btn sm" onclick={download}><Icon name="download" size={13} />{t("vault.kit.download")}</button>
      </div>
    </div>

    <label class="check confirm">
      <input type="checkbox" bind:checked={saved} />
      <span>{t("vault.kit.confirm")}</span>
    </label>
    <button class="btn primary lg wide" disabled={!saved} onclick={() => vault.finishKit()}>{t("vault.kit.continue")}</button>
  {:else}
    <h1>{t("vault.create.title")}</h1>
    <p class="lede">{t("vault.create.body")}</p>
    <form onsubmit={submit} class="form">
      <label class="field">
        <span class="label">{t("vault.password")}</span>
        <input class="input lg" type="password" autocomplete="new-password" bind:value={password} data-autofocus />
      </label>
      <div class="meter" aria-live="polite">
        <div class="bars" data-s={password ? strength : -1}>
          {#each [1, 2, 3, 4] as n (n)}<span class:on={password.length > 0 && strength >= n}></span>{/each}
        </div>
        <span class="hint">
          {#if password}{t(`vault.strength.${strength}` as MessageKey)}{:else}{t("vault.minLength")}{/if}
        </span>
      </div>
      <label class="field">
        <span class="label">{t("vault.passwordConfirm")}</span>
        <input class="input lg" type="password" autocomplete="new-password" bind:value={confirm} aria-invalid={mismatch} />
        {#if mismatch}<span class="err">{t("vault.mismatch")}</span>{/if}
      </label>
      <button class="btn primary lg wide" type="submit" disabled={!canSubmit}>
        {busy ? t("vault.creating") : t("vault.create.submit")}
      </button>
    </form>
    <button type="button" class="link join" onclick={() => (vault.phase = "join")}>{t("vault.create.join")}</button>
  {/if}
</VaultFrame>

<style>
  .link.join {
    display: block;
    margin: 18px auto 0;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  .link.join:hover {
    color: var(--text);
  }
  .form {
    display: grid;
    gap: 14px;
  }
  .input.lg {
    height: 36px;
    font-size: 14px;
  }
  .wide {
    width: 100%;
    margin-top: 6px;
  }
  .meter {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: -6px;
  }
  .bars {
    display: flex;
    gap: 3px;
  }
  .bars span {
    width: 28px;
    height: 3px;
    border-radius: 2px;
    background: var(--border-strong);
    transition: background var(--t) var(--ease);
  }
  .bars[data-s="1"] span.on {
    background: var(--danger);
  }
  .bars[data-s="2"] span.on {
    background: var(--warn);
  }
  .bars[data-s="3"] span.on,
  .bars[data-s="4"] span.on {
    background: var(--ok);
  }
  .err {
    font-size: var(--fs-meta);
    color: var(--danger);
  }
  .kit {
    padding: 16px;
    border-radius: var(--r-lg);
    border: 1px dashed var(--border-strong);
    background: var(--panel);
    margin-bottom: 18px;
  }
  .key {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
    margin: 8px 0 14px;
    font-size: 16px;
    letter-spacing: 0.06em;
    color: var(--text);
    user-select: all;
  }
  .key span:first-child {
    color: var(--accent-text);
  }
  .kit-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .confirm {
    margin-bottom: 14px;
    color: var(--text-2);
  }
</style>
