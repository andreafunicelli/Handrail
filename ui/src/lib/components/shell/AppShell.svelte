<script lang="ts">
  import { onMount } from "svelte";
  import IconRail from "./IconRail.svelte";
  import CommandPalette from "./CommandPalette.svelte";
  import ConnectionSidebar from "../connections/ConnectionSidebar.svelte";
  import ConnectionEditor from "../connections/ConnectionEditor.svelte";
  import TabStrip from "../terminal/TabStrip.svelte";
  import PaneTree from "../terminal/PaneTree.svelte";
  import StartPage from "../terminal/StartPage.svelte";
  import HostKeyDialog from "../terminal/HostKeyDialog.svelte";
  import AiSidebar from "../ai/AiSidebar.svelte";
  import Settings from "../settings/Settings.svelte";
  import Resizer from "../common/Resizer.svelte";
  import Modal from "../common/Modal.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { leaves } from "../../util/layout";
  import { agent } from "../../stores/agent.svelte";
  import { connections } from "../../stores/connections.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { vault } from "../../stores/vault.svelte";

  let wsName = $state("");

  // Ignore the synthetic click that follows the tap which opened a drawer (touch "ghost click").
  let openedAt = 0;
  $effect(() => {
    if (ui.sidebarDrawer || ui.aiDrawer) openedAt = performance.now();
  });
  const settled = () => performance.now() - openedAt > 350;

  onMount(() => {
    void Promise.all([connections.load(), sessions.init(), agent.init()]).catch(toasts.error);

    const mq = matchMedia("(max-width: 819px)");
    const sync = () => {
      ui.narrow = mq.matches;
      if (!mq.matches) ui.sidebarDrawer = ui.aiDrawer = false;
    };
    sync();
    mq.addEventListener("change", sync);

    // Inactivity auto-lock (the core enforces it too when it has the setting).
    let idle = Date.now();
    const bump = () => (idle = Date.now());
    const evs = ["pointerdown", "keydown", "wheel"] as const;
    evs.forEach((e) => addEventListener(e, bump, { passive: true, capture: true }));
    const iv = setInterval(() => {
      if (settings.autoLock > 0 && Date.now() - idle > settings.autoLock * 60_000) void vault.lock();
    }, 30_000);

    return () => {
      mq.removeEventListener("change", sync);
      evs.forEach((e) => removeEventListener(e, bump, { capture: true }));
      clearInterval(iv);
    };
  });

  function onkeydown(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    if (!mod || e.altKey) return;
    const k = e.key.toLowerCase();
    if (k === "k") {
      e.preventDefault();
      ui.paletteOpen = !ui.paletteOpen;
    } else if (k === "j") {
      e.preventDefault();
      if (ui.narrow) ui.aiDrawer = !ui.aiDrawer;
      else settings.aiOpen = !settings.aiOpen;
    } else if (k === ",") {
      e.preventDefault();
      ui.openSettings();
    } else if (k === "t" && e.shiftKey) {
      e.preventDefault();
      sessions.openLocal().catch(toasts.error);
    } else if (k === "w" && e.shiftKey && sessions.activeTabId) {
      e.preventDefault();
      sessions.closeTab(sessions.activeTabId);
    } else if (e.key === "Tab") {
      e.preventDefault();
      sessions.cycleTab(e.shiftKey ? -1 : 1);
    }
  }

  function saveWs(e: SubmitEvent) {
    e.preventDefault();
    const name = wsName.trim();
    if (!name) return;
    sessions.saveWorkspace(name);
    toasts.success(t("tabs.workspaceSaved", { name }));
    wsName = "";
    ui.saveWorkspaceOpen = false;
  }
</script>

<svelte:window {onkeydown} />

<div class="shell" class:narrow={ui.narrow}>
  {#if !ui.narrow}
    <IconRail />
    <div class="side" style:width="{settings.sidebarWidth}px">
      {@render sideContent()}
    </div>
    <Resizer value={settings.sidebarWidth} min={200} max={480} onchange={(v) => (settings.sidebarWidth = v)} />
  {/if}

  <main class="main">
    <TabStrip />
    <div class="panes">
      {#if sessions.activeTab}
        {#key sessions.activeTab.id}
          <PaneTree node={sessions.activeTab.layout} tabId={sessions.activeTab.id} multi={leaves(sessions.activeTab.layout).length > 1} />
        {/key}
      {:else}
        <StartPage />
      {/if}
    </div>
  </main>

  {#if !ui.narrow && settings.aiOpen}
    <Resizer value={settings.aiWidth} min={300} max={640} edge="left" onchange={(v) => (settings.aiWidth = v)} />
    <div class="ai-dock" style:width="{settings.aiWidth}px"><AiSidebar /></div>
  {/if}
</div>

{#snippet sideContent()}
  {#if ui.rail === "connections"}
    <ConnectionSidebar />
  {:else}
    <div class="snippets">
      <Icon name="snippet" size={18} />
      <strong>{t("sidebar.snippets.title")}</strong>
      <p>{t("sidebar.snippets.body")}</p>
    </div>
  {/if}
{/snippet}

{#if ui.narrow}
  {#if ui.sidebarDrawer}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="scrim" onclick={() => settled() && (ui.sidebarDrawer = false)}></div>
    <div class="drawer left" role="dialog" aria-label={t("rail.connections")} tabindex="-1" onkeydown={(e) => e.key === "Escape" && (ui.sidebarDrawer = false)}>
      <IconRail />
      <div class="side">{@render sideContent()}</div>
    </div>
  {/if}
  {#if ui.aiDrawer}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="scrim" onclick={() => settled() && (ui.aiDrawer = false)}></div>
    <div class="drawer right" role="dialog" aria-label={t("ai.title")} tabindex="-1"><AiSidebar /></div>
  {/if}
{/if}

{#if ui.editing}<ConnectionEditor connection={ui.editing.connection} isNew={ui.editing.isNew} />{/if}
{#if ui.settingsPage}<Settings />{/if}
{#if ui.paletteOpen}<CommandPalette />{/if}
{#if sessions.hostKeys[0]}{#key sessions.hostKeys[0].request_id}<HostKeyDialog req={sessions.hostKeys[0]} />{/key}{/if}

{#if ui.saveWorkspaceOpen}
  <Modal label={t("tabs.saveWorkspace")} onclose={() => (ui.saveWorkspaceOpen = false)} width="380px">
    <form class="dialog" onsubmit={saveWs}>
      <h2>{t("tabs.saveWorkspace")}</h2>
      <input class="input" bind:value={wsName} placeholder={t("tabs.workspaceName")} aria-label={t("tabs.workspaceName")} />
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => (ui.saveWorkspaceOpen = false)}>{t("common.cancel")}</button>
        <button class="btn primary" disabled={!wsName.trim()}>{t("common.save")}</button>
      </div>
    </form>
  </Modal>
{/if}

{#if ui.confirmReq}
  {@const req = ui.confirmReq}
  <Modal label={req.title} onclose={() => ui.answer(false)} width="380px">
    <div class="dialog">
      <h2>{req.title}</h2>
      {#if req.body}<p>{req.body}</p>{/if}
      <div class="actions">
        <button class="btn ghost" onclick={() => ui.answer(false)}>{t("common.cancel")}</button>
        <button class="btn {req.danger ? 'danger' : 'primary'}" data-autofocus onclick={() => ui.answer(true)}>{req.confirmLabel}</button>
      </div>
    </div>
  </Modal>
{/if}

<style>
  .shell {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .side {
    display: flex;
    flex: none;
    min-width: 0;
    height: 100%;
  }
  .side > :global(*) {
    flex: 1;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .panes {
    flex: 1;
    min-height: 0;
    position: relative;
  }
  .ai-dock {
    flex: none;
    min-width: 0;
    height: 100%;
  }
  .snippets {
    display: grid;
    align-content: start;
    justify-items: start;
    gap: 6px;
    padding: 24px 18px;
    background: var(--panel);
    color: var(--muted);
  }
  .snippets :global(svg) {
    color: var(--faint);
  }
  .snippets strong {
    color: var(--text);
    font-weight: 550;
  }
  .snippets p {
    margin: 0;
    font-size: var(--fs-meta);
  }
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: rgba(5, 6, 8, 0.5);
    animation: fade var(--t) var(--ease);
  }
  .drawer {
    position: fixed;
    top: 0;
    bottom: 0;
    z-index: 31;
    display: flex;
    width: min(360px, calc(100vw - 40px));
    background: var(--panel);
    box-shadow: var(--shadow-float);
    outline: none;
  }
  .drawer.left {
    left: 0;
    animation: inl var(--t) var(--ease);
  }
  .drawer.left .side {
    flex: 1;
  }
  .drawer.right {
    right: 0;
    width: min(420px, 100vw);
    animation: inr var(--t) var(--ease);
  }
  .drawer.right > :global(*) {
    flex: 1;
  }
  .dialog {
    padding: 20px;
  }
  .dialog h2 {
    margin: 0 0 12px;
    font-size: 15px;
    font-weight: 600;
  }
  .dialog p {
    margin: -4px 0 12px;
    color: var(--muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes inl {
    from {
      transform: translateX(-12px);
      opacity: 0;
    }
  }
  @keyframes inr {
    from {
      transform: translateX(12px);
      opacity: 0;
    }
  }
</style>
