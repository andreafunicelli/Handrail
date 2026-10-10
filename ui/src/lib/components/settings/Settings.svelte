<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import Icon, { type IconName } from "../icons/Icon.svelte";
  import ProvidersPage from "./ProvidersPage.svelte";
  import HarnessesPage from "./HarnessesPage.svelte";
  import ApprovalsPage from "./ApprovalsPage.svelte";
  import ImportPage from "./ImportPage.svelte";
  import SyncPage from "./SyncPage.svelte";
  import AppearancePage from "./AppearancePage.svelte";
  import VaultPage from "./VaultPage.svelte";
  import AboutPage from "./AboutPage.svelte";
  import { t } from "../../i18n";
  import { ui, type SettingsPage } from "../../stores/ui.svelte";

  const nav: { id: SettingsPage; icon: IconName }[] = [
    { id: "providers", icon: "sparkle" },
    { id: "harnesses", icon: "terminal" },
    { id: "approvals", icon: "shield" },
    { id: "import", icon: "import" },
    { id: "sync", icon: "cloud" },
    { id: "appearance", icon: "palette" },
    { id: "vault", icon: "lock" },
    { id: "about", icon: "info" },
  ];
  const page = $derived(ui.settingsPage ?? "providers");
</script>

<Modal label={t("settings.title")} onclose={() => (ui.settingsPage = null)} width="920px">
  <div class="settings">
    <nav aria-label={t("settings.title")}>
      <div class="title">{t("settings.title")}</div>
      {#each nav as n (n.id)}
        <button class:on={page === n.id} aria-current={page === n.id ? "page" : undefined} onclick={() => (ui.settingsPage = n.id)}>
          <Icon name={n.icon} size={14} />{t(`settings.${n.id}`)}
        </button>
      {/each}
    </nav>
    <section class="page">
      <header>
        <h2>{t(`settings.${page}`)}</h2>
        <button class="icon-btn" aria-label={t("settings.close")} onclick={() => (ui.settingsPage = null)}><Icon name="x" /></button>
      </header>
      <div class="content">
        {#if page === "providers"}<ProvidersPage />
        {:else if page === "harnesses"}<HarnessesPage />
        {:else if page === "approvals"}<ApprovalsPage />
        {:else if page === "import"}<ImportPage />
        {:else if page === "sync"}<SyncPage />
        {:else if page === "appearance"}<AppearancePage />
        {:else if page === "vault"}<VaultPage />
        {:else}<AboutPage />{/if}
      </div>
    </section>
  </div>
</Modal>

<style>
  .settings {
    display: grid;
    grid-template-columns: 200px 1fr;
    height: min(640px, calc(100vh - 32px));
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 14px 8px;
    border-right: 1px solid var(--border);
    background: var(--bg);
    overflow: auto;
  }
  .title {
    padding: 2px 10px 12px;
    font-weight: 600;
  }
  nav button {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--r);
    color: var(--muted);
    text-align: left;
  }
  nav button:hover {
    background: var(--hover);
    color: var(--text);
  }
  nav button.on {
    background: var(--active);
    color: var(--text);
  }
  nav button.on :global(svg) {
    color: var(--accent-text);
  }
  .page {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 52px;
    padding: 0 12px 0 28px;
    flex: none;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .content {
    flex: 1;
    overflow: auto;
    padding: 0 28px 28px;
  }
  .content :global(.intro) {
    margin: 0 0 20px;
    color: var(--muted);
    max-width: 62ch;
  }
  .content :global(h3) {
    margin: 26px 0 10px;
    font-size: var(--fs);
    font-weight: 600;
  }
  .content :global(.card) {
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--bg);
  }
  .content :global(.list-row) {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-top: 1px solid var(--border);
    min-width: 0;
  }
  .content :global(.list-row:first-child) {
    border-top: 0;
  }
  .content :global(.form-grid) {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  @media (max-width: 700px) {
    .settings {
      grid-template-columns: 1fr;
      grid-template-rows: auto 1fr;
      height: calc(100vh - 32px);
    }
    nav {
      flex-direction: row;
      overflow-x: auto;
      border-right: 0;
      border-bottom: 1px solid var(--border);
      padding: 8px;
    }
    .title {
      display: none;
    }
    nav button {
      flex: none;
    }
    header {
      padding-left: 16px;
    }
    .content {
      padding: 0 16px 20px;
    }
    .content :global(.form-grid) {
      grid-template-columns: 1fr;
    }
  }
</style>
