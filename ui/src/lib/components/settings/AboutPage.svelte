<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import { updates } from "../../stores/updates.svelte";
  import { relativeTime } from "../../util/time";

  let version = $state("");
  onMount(async () => {
    try {
      version = await api.appVersion();
    } catch {
      version = "?";
    }
  });

  async function checkNow() {
    const found = await updates.check(true);
    if (!found && updates.phase !== "error") upToDate = true;
  }
  let upToDate = $state(false);
</script>

<div class="about">
  <div class="mark" aria-hidden="true">✦</div>
  <div>
    <strong>{t("app.name")}</strong>
    <p class="intro">{t("about.body")}</p>
  </div>
</div>
<dl>
  <dt>{t("about.version")}</dt>
  <dd class="mono">{version}</dd>
  <dt>{t("about.backend")}</dt>
  <dd>{api.backend === "mock" ? t("about.backend.mock") : t("about.backend.tauri")}</dd>
  <dt>{t("update.title")}</dt>
  <dd class="upd">
    {#if updates.phase === "checking"}
      <span class="hint">{t("update.checking")}</span>
    {:else if updates.info && (updates.phase === "available" || updates.phase === "error")}
      <span>{t("update.available", { version: updates.info.version })}</span>
      <button class="btn sm primary" onclick={() => updates.install()}>{t("update.install")}</button>
    {:else if updates.phase === "downloading"}
      <span class="hint">{t("update.downloading", { version: updates.info?.version ?? "" })}</span>
    {:else}
      <button class="btn sm" onclick={checkNow}>{t("update.check")}</button>
      {#if upToDate}<span class="hint">{t("update.upToDate")}</span>{/if}
      {#if updates.lastChecked && !upToDate}<span class="hint">{t("update.lastChecked", { when: relativeTime(updates.lastChecked) })}</span>{/if}
    {/if}
    {#if updates.error}<span class="hint err">{updates.error}</span>{/if}
  </dd>
</dl>

<style>
  .upd {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .err {
    color: var(--danger);
  }
  .about {
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }
  .mark {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 10px;
    border: 1px solid var(--border-strong);
    color: var(--accent);
    flex: none;
  }
  strong {
    font-size: 15px;
    font-weight: 600;
  }
  .intro {
    margin: 2px 0 0 !important;
  }
  dl {
    display: grid;
    grid-template-columns: 120px 1fr;
    gap: 8px;
    margin: 24px 0 0;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
  }
</style>
