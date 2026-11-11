<script lang="ts">
  import Logo from "../brand/Logo.svelte";
  import { onMount } from "svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import { updates } from "../../stores/updates.svelte";
  import { tour } from "../../stores/tour.svelte";
  import { formatBytes, relativeTime } from "../../util/time";
  import type { NightlyBuild } from "../../api/types";

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

  // ---- nightly builds: a separate app, offered as a download (never installed from here) ----
  /** This app is itself a nightly (`1.1.0-212`): it updates from the nightly channel. */
  const isNightly = $derived(version.includes("-"));
  let nightly = $state<{ phase: "idle" | "finding" | "done" | "error"; build: NightlyBuild | null; error: string; opened: boolean }>({
    phase: "idle",
    build: null,
    error: "",
    opened: false,
  });
  async function findNightly() {
    nightly = { phase: "finding", build: null, error: "", opened: false };
    try {
      nightly = { phase: "done", build: await api.nightlyBuild(), error: "", opened: false };
    } catch (e) {
      nightly = { phase: "error", build: null, error: String((e as { message?: string })?.message ?? e), opened: false };
    }
  }
  async function open(url: string, download: boolean) {
    try {
      await api.openUrl(url);
      if (download) nightly.opened = true;
    } catch (e) {
      nightly.error = String((e as { message?: string })?.message ?? e);
    }
  }
</script>

<div class="about">
  <div class="mark" aria-hidden="true"><Logo size={24} /></div>
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
  {#if version && !isNightly}
    <dt>{t("nightly.title")}</dt>
    <dd class="nightly">
      <p class="hint">{t("nightly.intro")}</p>
      {#if nightly.phase === "finding"}
        <span class="hint" role="status">{t("nightly.finding")}</span>
      {:else if nightly.phase === "done" && nightly.build}
        {@const b = nightly.build}
        <div class="upd" role="status">
          <span class="mono">{t("nightly.found", { version: b.version })}</span>
          {#if b.published_at}<span class="hint">{t("nightly.published", { when: relativeTime(Date.parse(b.published_at)) })}</span>{/if}
        </div>
        <div class="upd">
          {#if b.download}
            <button class="btn sm primary" onclick={() => open(b.download!.url, true)} title={b.download.name}
              >{t("nightly.download")} · {formatBytes(b.download.size)}</button
            >
          {:else}
            <span class="hint">{t("nightly.noInstaller")}</span>
          {/if}
          <button class="btn sm ghost" onclick={() => open(b.page_url, false)}>{t("nightly.notes")}</button>
        </div>
        {#if nightly.opened}<span class="hint" role="status">{t("nightly.opened")}</span>{/if}
        <p class="hint warn">{t("nightly.warning")}</p>
      {:else}
        <div class="upd">
          <button class="btn sm" onclick={findNightly}>{t("nightly.find")}</button>
          {#if nightly.phase === "done"}<span class="hint" role="status">{t("nightly.none")}</span>{/if}
        </div>
      {/if}
      {#if nightly.error}<span class="hint err" role="alert">{t("nightly.failed", { error: nightly.error })}</span>{/if}
    </dd>
  {/if}
  <dt>{t("about.tour")}</dt>
  <dd><button class="btn sm" onclick={() => tour.start()}>{t("tour.replay")}</button></dd>
</dl>

<style>
  .upd {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .nightly {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 560px;
  }
  .nightly p {
    margin: 0;
  }
  .warn {
    color: var(--warn, var(--muted));
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
    color: var(--text);
    flex: none;
  }
  strong {
    font-size: var(--fs-title);
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
