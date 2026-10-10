<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { ApprovalMode, ApprovalSettings } from "../../api/types";
  import { toasts } from "../../stores/toasts.svelte";

  let s = $state<ApprovalSettings | null>(null);
  let pattern = $state("");
  const modes: ApprovalMode[] = ["confirm_all", "confirm_writes", "auto"];

  onMount(async () => {
    try {
      s = await api.getApprovalSettings();
    } catch (e) {
      toasts.error(e);
    }
  });

  async function persist() {
    if (!s) return;
    try {
      await api.saveApprovalSettings($state.snapshot(s) as ApprovalSettings);
      toasts.success(t("appr.saved"));
    } catch (e) {
      toasts.error(e);
    }
  }
  function add(e: SubmitEvent) {
    e.preventDefault();
    const p = pattern.trim();
    if (!s || !p || s.deny_list.includes(p)) return;
    s.deny_list.push(p);
    pattern = "";
    void persist();
  }
  function remove(p: string) {
    if (!s) return;
    s.deny_list = s.deny_list.filter((x) => x !== p);
    void persist();
  }
</script>

<p class="intro">{t("appr.body")}</p>

{#if s}
  <h3>{t("appr.default")}</h3>
  <div class="modes" role="radiogroup" aria-label={t("appr.default")}>
    {#each modes as m (m)}
      <label class="mode" class:on={s.default_mode === m}>
        <input type="radio" name="mode" value={m} bind:group={s.default_mode} onchange={persist} />
        <span class="mname">{t(`ai.mode.${m}`)}</span>
        <span class="hint">{t(`appr.mode.${m}.desc`)}</span>
      </label>
    {/each}
  </div>

  <h3>{t("appr.deny")}</h3>
  <p class="hint sub">{t("appr.denyHint")}</p>
  <div class="card">
    {#each s.deny_list as p (p)}
      <div class="list-row">
        <code class="mono pat">{p}</code>
        <button class="icon-btn" aria-label={t("common.delete")} onclick={() => remove(p)}><Icon name="x" size={13} /></button>
      </div>
    {/each}
  </div>
  <form class="add" onsubmit={add}>
    <input class="input mono" bind:value={pattern} placeholder={t("appr.denyPlaceholder")} spellcheck="false" />
    <button class="btn" disabled={!pattern.trim()}>{t("appr.denyAdd")}</button>
  </form>
{:else}
  <p class="hint">{t("common.loading")}</p>
{/if}

<style>
  .modes {
    display: grid;
    gap: 6px;
  }
  .mode {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: 10px;
    padding: 10px 12px;
    border-radius: var(--r-lg);
    border: 1px solid var(--border);
    cursor: pointer;
  }
  .mode.on {
    border-color: var(--accent-line);
    background: var(--accent-soft);
  }
  .mode input {
    grid-row: span 2;
    accent-color: var(--accent);
    margin: 3px 0 0;
  }
  .mname {
    font-weight: 550;
  }
  .sub {
    margin: -4px 0 10px;
  }
  .pat {
    flex: 1;
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  .add {
    display: flex;
    gap: 8px;
    margin-top: 10px;
  }
</style>
