<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { ApprovalMode, ApprovalSettings } from "../../api/types";
  import { toasts } from "../../stores/toasts.svelte";
  import { literalText, normalizeDenyList, parseDenyInput } from "../../util/denylist";

  let s = $state<ApprovalSettings | null>(null);
  let pattern = $state("");
  let patternError = $state<string | null>(null);
  const modes: ApprovalMode[] = ["confirm_all", "confirm_writes", "auto"];
  const stepChoices = [25, 50, 100, 200];

  onMount(async () => {
    try {
      const loaded = await api.getApprovalSettings();
      const list = normalizeDenyList(loaded.deny_list);
      s = { ...loaded, deny_list: list };
      if (list !== loaded.deny_list) void persist();
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
    if (!s) return;
    const r = parseDenyInput(pattern);
    if (!r.ok) {
      patternError = r.error === "empty" ? null : t(`appr.denyError.${r.error}`);
      return;
    }
    patternError = null;
    pattern = "";
    if (s.deny_list.includes(r.pattern)) return;
    s.deny_list.push(r.pattern);
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

  <h3>{t("appr.steps")}</h3>
  <p class="hint sub">{t("appr.stepsHint")}</p>
  <div class="seg" role="group" aria-label={t("appr.steps")}>
    {#each stepChoices as n (n)}
      <button aria-pressed={(s.max_steps ?? 50) === n} onclick={() => s && ((s.max_steps = n), persist())}>{n}</button>
    {/each}
  </div>

  <h3>{t("appr.deny")}</h3>
  <p class="hint sub">{t("appr.denyHint")}</p>
  <p class="hint sub">{t("appr.denyBuiltIn")}</p>
  <div class="card">
    {#each s.deny_list as p (p)}
      {@const text = literalText(p)}
      <div class="list-row">
        <code class="mono pat">{text ?? p}</code>
        {#if text == null}<span class="pill" title={t("appr.denyRegexHint")}>{t("appr.denyRegex")}</span>{/if}
        <button class="icon-btn" aria-label={t("common.delete")} onclick={() => remove(p)}><Icon name="x" size={13} /></button>
      </div>
    {/each}
  </div>
  <form class="add" onsubmit={add}>
    <input
      class="input mono"
      bind:value={pattern}
      oninput={() => (patternError = null)}
      placeholder={t("appr.denyPlaceholder")}
      spellcheck="false"
      aria-invalid={patternError != null}
      aria-describedby={patternError ? "deny-error" : undefined}
    />
    <button class="btn" disabled={!pattern.trim()}>{t("appr.denyAdd")}</button>
  </form>
  {#if patternError}<p class="hint err" id="deny-error" role="alert">{patternError}</p>{/if}
{:else}
  <p class="hint">{t("common.loading")}</p>
{/if}

<style>
  .err {
    color: var(--danger);
    margin: 6px 0 0;
  }
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
