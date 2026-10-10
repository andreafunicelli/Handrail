<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { sessions } from "../../stores/sessions.svelte";
  import { updates } from "../../stores/updates.svelte";

  const open = $derived(Object.keys(sessions.infos).length);
  const pct = $derived(updates.progress == null ? null : Math.round(updates.progress * 100));
</script>

{#if updates.bannerVisible && updates.info}
  <div class="bar" class:err={updates.phase === "error"} role="status">
    <Icon name={updates.phase === "error" ? "alert" : "download"} size={14} />
    <span class="msg">
      {#if updates.phase === "downloading"}
        {t("update.downloading", { version: updates.info.version })}{pct != null ? ` ${pct}%` : ""}
      {:else if updates.phase === "ready"}
        {t("update.restarting")}
      {:else if updates.phase === "error"}
        {t("update.failed", { error: updates.error ?? "" })}
      {:else}
        {t("update.available", { version: updates.info.version })}
        {#if open}<span class="hint">{t("update.sessionsWarning", { count: open })}</span>{/if}
      {/if}
    </span>
    {#if updates.phase === "downloading" && pct != null}
      <span class="meter" aria-hidden="true"><span style:width="{pct}%"></span></span>
    {/if}
    {#if updates.phase === "available" || updates.phase === "error"}
      <button class="btn sm primary" onclick={() => updates.install()}>{t(updates.phase === "error" ? "update.retry" : "update.install")}</button>
      <button class="icon-btn" aria-label={t("update.later")} title={t("update.later")} onclick={() => updates.dismiss()}><Icon name="x" size={13} /></button>
    {/if}
  </div>
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--accent-soft);
    color: var(--text);
    font-size: var(--fs-small);
  }
  .bar.err {
    background: var(--danger-soft);
  }
  .msg {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 8px;
    align-items: baseline;
    flex-wrap: wrap;
  }
  .meter {
    width: 120px;
    height: 4px;
    border-radius: 2px;
    background: var(--border);
    overflow: hidden;
  }
  .meter span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 120ms linear;
  }
</style>
