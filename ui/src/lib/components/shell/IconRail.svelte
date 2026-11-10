<script lang="ts">
  import Logo from "../brand/Logo.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { ui } from "../../stores/ui.svelte";
  import { vault } from "../../stores/vault.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { transfers } from "../../stores/transfers.svelte";
  import TransferPanel from "../files/TransferPanel.svelte";

  let transfersBtn = $state<HTMLButtonElement>();
  const transfersLabel = $derived(transfers.running ? t("files.transfers.buttonRunning", { n: transfers.running }) : t("files.transfers.title"));
  function closeTransfers() {
    transfers.panelOpen = false;
    transfersBtn?.focus();
  }
</script>

<nav class="rail" aria-label="Main">
  <div class="mark" aria-hidden="true"><Logo size={18} /></div>
  <button class="item" class:on={ui.rail === "connections"} aria-pressed={ui.rail === "connections"} title={t("rail.connections")} aria-label={t("rail.connections")} onclick={() => (ui.rail = "connections")}>
    <Icon name="server" size={18} />
  </button>
  <button data-tour="snippets" class="item" class:on={ui.rail === "snippets"} aria-pressed={ui.rail === "snippets"} title={t("rail.snippets")} aria-label={t("rail.snippets")} onclick={() => (ui.rail = "snippets")}>
    <Icon name="snippet" size={18} />
  </button>
  <span class="grow"></span>
  <button
    bind:this={transfersBtn}
    data-transfers-button
    class="item"
    class:on={transfers.panelOpen}
    aria-expanded={transfers.panelOpen}
    aria-haspopup="dialog"
    title={transfersLabel}
    aria-label={transfersLabel}
    onclick={() => (transfers.panelOpen = !transfers.panelOpen)}
  >
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><path d="M7 4v14M3.5 14.5 7 18l3.5-3.5M17 20V6M13.5 9.5 17 6l3.5 3.5" /></svg>
    {#if transfers.running}<span class="badge" aria-hidden="true">{transfers.running > 99 ? "99+" : transfers.running}</span>{/if}
  </button>
  <button class="item" title={t("vault.lock")} aria-label={t("vault.lock")} onclick={() => vault.lock().catch(toasts.error)}>
    <Icon name="lock" size={17} />
  </button>
  <button data-tour="settings" class="item" class:on={!!ui.settingsPage} title={t("rail.settings")} aria-label={t("rail.settings")} onclick={() => ui.openSettings()}>
    <Icon name="settings" size={18} />
  </button>
</nav>
{#if transfers.panelOpen}<TransferPanel onclose={closeTransfers} />{/if}

<style>
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    width: var(--rail-w);
    flex: none;
    padding: 10px 0 10px;
    background: var(--bg);
    border-right: 1px solid var(--border);
  }
  .mark {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    margin-bottom: 10px;
    color: var(--text);
    font-size: var(--fs);
  }
  .item {
    position: relative;
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: var(--r-lg);
    color: var(--faint);
    transition: color var(--t-fast) var(--ease), background var(--t-fast) var(--ease);
  }
  .item:hover {
    color: var(--text);
    background: var(--hover);
  }
  .item.on {
    color: var(--text);
  }
  .item.on::before {
    content: "";
    position: absolute;
    left: -7px;
    top: 9px;
    bottom: 9px;
    width: 2px;
    border-radius: 2px;
    background: var(--accent);
  }
  .grow {
    flex: 1;
  }
  .badge {
    position: absolute;
    top: 2px;
    right: 1px;
    min-width: 15px;
    height: 15px;
    padding: 0 3px;
    border-radius: 8px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: var(--fs-tiny);
    font-weight: 600;
    line-height: 15px;
    text-align: center;
  }
</style>
