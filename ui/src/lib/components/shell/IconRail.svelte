<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { ui } from "../../stores/ui.svelte";
  import { vault } from "../../stores/vault.svelte";
  import { toasts } from "../../stores/toasts.svelte";
</script>

<nav class="rail" aria-label="Main">
  <div class="mark" aria-hidden="true">✦</div>
  <button class="item" class:on={ui.rail === "connections"} aria-pressed={ui.rail === "connections"} title={t("rail.connections")} aria-label={t("rail.connections")} onclick={() => (ui.rail = "connections")}>
    <Icon name="server" size={18} />
  </button>
  <button class="item" class:on={ui.rail === "snippets"} aria-pressed={ui.rail === "snippets"} title={t("rail.snippets")} aria-label={t("rail.snippets")} onclick={() => (ui.rail = "snippets")}>
    <Icon name="snippet" size={18} />
  </button>
  <span class="grow"></span>
  <button class="item" title={t("vault.lock")} aria-label={t("vault.lock")} onclick={() => vault.lock().catch(toasts.error)}>
    <Icon name="lock" size={17} />
  </button>
  <button class="item" class:on={!!ui.settingsPage} title={t("rail.settings")} aria-label={t("rail.settings")} onclick={() => ui.openSettings()}>
    <Icon name="settings" size={18} />
  </button>
</nav>

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
    color: var(--accent);
    font-size: 13px;
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
</style>
