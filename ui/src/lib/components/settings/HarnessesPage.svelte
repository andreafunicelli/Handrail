<script lang="ts" module>
  import type { HarnessKind } from "../../api/types";
  export const HARNESS_NAMES: Record<HarnessKind, string> = { claude_code: "Claude Code", codex: "Codex", open_code: "OpenCode" };
</script>

<script lang="ts">
  import { segmented } from "../../actions/segmented";
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { HarnessExtensions, HarnessProfile } from "../../api/types";
  import { agent } from "../../stores/agent.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { uuid } from "../../util/id";
  import { joinArgs, splitArgs } from "../../util/args";

  /** `aht_harness::launch::API_BILLING_FLAG`: consumed by the launcher, never passed to the CLI. */
  const API_BILLING_FLAG = "--aht-api-billing";

  let detecting = $state(false);
  let editing = $state<HarnessProfile | null>(null);
  let ext = $state<HarnessExtensions | null>(null);
  /** The editor's extra arguments, as typed (without the billing marker). */
  let extraText = $state("");
  let apiBilling = $state(false);
  const extraParsed = $derived(splitArgs(extraText));

  function startEditing(p: HarnessProfile) {
    editing = p;
    extraText = joinArgs(p.extra_args.filter((a) => a !== API_BILLING_FLAG));
    apiBilling = p.extra_args.includes(API_BILLING_FLAG);
    void loadExt();
  }

  onMount(() => void detect());

  async function detect() {
    detecting = true;
    try {
      await agent.detect();
    } catch (e) {
      toasts.error(e);
    } finally {
      detecting = false;
    }
  }

  async function signIn(kind: HarnessKind) {
    try {
      const info = await api.harnessLogin(kind, 100, 30);
      sessions.place(info);
      ui.settingsPage = null;
    } catch (e) {
      toasts.error(e);
    }
  }

  function newProfile() {
    startEditing({ id: uuid(), name: "", harness: "claude_code", display: "native", server_mode: true, skills: [], plugins: [], extra_args: [], model: null });
  }
  function edit(p: HarnessProfile) {
    startEditing(structuredClone($state.snapshot(p) as HarnessProfile));
  }
  async function loadExt() {
    if (!editing) return;
    ext = null;
    try {
      ext = await api.listHarnessExtensions(editing.harness);
    } catch (e) {
      toasts.error(e);
    }
  }
  function setHarness(k: HarnessKind) {
    if (!editing) return;
    editing.harness = k;
    editing.display = k === "claude_code" ? "native" : k !== "open_code" && editing.display === "open_chamber" ? "native" : editing.display;
    editing.skills = [];
    editing.plugins = [];
    void loadExt();
  }
  function toggle(list: "skills" | "plugins", path: string) {
    if (!editing) return;
    const cur = editing[list];
    editing[list] = cur.includes(path) ? cur.filter((x) => x !== path) : [...cur, path];
  }
  async function save() {
    if (!editing) return;
    const p = $state.snapshot(editing) as HarnessProfile;
    if (!p.name.trim()) p.name = HARNESS_NAMES[p.harness];
    if (!extraParsed) return;
    // OpenCode never scrubs API keys, so the marker only means something for the other two.
    const billing = (apiBilling || extraParsed.includes(API_BILLING_FLAG)) && p.harness !== "open_code";
    p.extra_args = [...extraParsed.filter((a) => a !== API_BILLING_FLAG), ...(billing ? [API_BILLING_FLAG] : [])];
    try {
      await api.saveHarnessProfile(p);
      await agent.reload();
      toasts.success(t("harn.saved"));
      editing = null;
    } catch (e) {
      toasts.error(e);
    }
  }
  async function remove() {
    if (!editing) return;
    try {
      await api.deleteHarnessProfile(editing.id);
      await agent.reload();
      editing = null;
    } catch (e) {
      toasts.error(e);
    }
  }
</script>

<p class="intro">{t("harn.body")}</p>

<div class="head-row">
  <h3>{t("harn.detected")}</h3>
  <button class="btn sm ghost" onclick={detect} disabled={detecting}><Icon name="refresh" size={13} />{t("harn.redetect")}</button>
</div>
<div class="card">
  {#each Object.keys(HARNESS_NAMES) as HarnessKind[] as kind (kind)}
    {@const d = agent.detected.find((x) => x.kind === kind)}
    <div class="list-row">
      <span class="hname">{HARNESS_NAMES[kind]}</span>
      {#if d?.path}
        <span class="ver mono">{d.version ?? ""}</span>
        <span class="path mono" title={d.path}>{d.path}</span>
        <span class="pill {d.logged_in ? 'ok' : 'warn'}">{d.logged_in ? t("harn.loggedIn") : t("harn.loggedOut")}</span>
        <button class="btn sm" onclick={() => signIn(kind)}>{t("harn.signIn")}</button>
      {:else}
        <span class="path">{detecting ? t("common.loading") : t("harn.notFound")}</span>
      {/if}
    </div>
  {/each}
</div>

<div class="head-row">
  <h3>{t("harn.profiles")}</h3>
  {#if !editing}<button class="btn sm" onclick={newProfile}><Icon name="plus" size={13} />{t("harn.newProfile")}</button>{/if}
</div>
{#if !editing}
  <div class="card">
    {#each agent.profiles as p (p.id)}
      <div class="list-row">
        <span class="hname">{p.name}</span>
        <span class="path">{HARNESS_NAMES[p.harness]} · {p.display === "native" ? t("harn.display.native") : p.display === "open_chamber" ? t("harn.display.openChamber") : t("harn.display.sidebar")}</span>
        <span class="grow"></span>
        {#if p.server_mode}<span class="pill accent">{t("harn.serverMode")}</span>{/if}
        <button class="btn sm ghost" onclick={() => edit(p)}><Icon name="edit" size={13} />{t("ctx.edit")}</button>
      </div>
    {:else}
      <div class="list-row path">{t("harn.newProfile")}</div>
    {/each}
  </div>
{:else}
  <div class="editor">
    <div class="form-grid">
      <label class="field"><span class="label">{t("harn.profileName")}</span><input class="input" bind:value={editing.name} placeholder={HARNESS_NAMES[editing.harness]} /></label>
      <label class="field">
        <span class="label">{t("harn.harness")}</span>
        <select class="select" value={editing.harness} onchange={(e) => setHarness((e.currentTarget as HTMLSelectElement).value as HarnessKind)}>
          {#each Object.entries(HARNESS_NAMES) as [k, n] (k)}<option value={k}>{n}</option>{/each}
        </select>
      </label>
    </div>
    <div class="field">
      <span class="label">{t("harn.display")}</span>
      <div class="seg" role="group" use:segmented>
        <button aria-pressed={editing.display === "native"} onclick={() => (editing!.display = "native")}>{t("harn.display.native")}</button>
        <button aria-pressed={editing.display === "sidebar"} onclick={() => (editing!.display = "sidebar")}>{t("harn.display.sidebar")}</button>
        {#if editing.harness === "open_code"}
          <button aria-pressed={editing.display === "open_chamber"} onclick={() => (editing!.display = "open_chamber")}>{t("harn.display.openChamber")}</button>
        {/if}
      </div>
      {#if editing.display === "open_chamber"}
        <p class="note"><Icon name="info" size={13} />{t("harn.openChamberNote")}</p>
      {/if}
      {#if editing.harness === "claude_code"}
        <p class="note"><Icon name="info" size={13} />{t("harn.claudeNote")}</p>
      {/if}
    </div>
    <label class="check toggle">
      <input type="checkbox" bind:checked={editing.server_mode} />
      <span><strong>{t("harn.serverMode")}</strong><br /><span class="hint">{t("harn.serverModeHint")}</span></span>
    </label>
    <label class="field">
      <span class="label">{t("harn.extraArgs")}</span>
      <input class="input mono" bind:value={extraText} placeholder={t("harn.extraArgsPlaceholder")} spellcheck="false" autocomplete="off" aria-invalid={!extraParsed} />
      <span class="hint" class:err={!extraParsed}>{extraParsed ? t("harn.extraArgsHint") : t("harn.extraArgsUnterminated")}</span>
    </label>
    {#if editing.harness !== "open_code"}
      <label class="check toggle">
        <input type="checkbox" bind:checked={apiBilling} />
        <span>
          <strong>{t("harn.apiBilling")}</strong><br />
          <span class="hint">{t(editing.harness === "claude_code" ? "harn.apiBillingHint.claude_code" : "harn.apiBillingHint.codex")}</span>
        </span>
      </label>
    {/if}
    <label class="field"><span class="label">{t("harn.model")}</span><input class="input mono" value={editing.model ?? ""} oninput={(e) => (editing!.model = (e.currentTarget as HTMLInputElement).value || null)} placeholder={t("harn.modelPlaceholder")} /></label>
    <div class="form-grid">
      {#each ["skills", "plugins"] as const as list (list)}
        <fieldset class="field">
          <legend class="label">{t(list === "skills" ? "harn.skills" : "harn.plugins")}</legend>
          {#if !ext}
            <span class="hint">{t("common.loading")}</span>
          {:else if !ext[list].length}
            <span class="hint">{t("harn.noExtensions")}</span>
          {:else}
            {#each ext[list] as x (x.path)}
              <label class="check"><input type="checkbox" checked={editing[list].includes(x.path) || editing[list].includes(x.name)} onchange={() => toggle(list, x.path)} /><span>{x.name}</span></label>
            {/each}
          {/if}
        </fieldset>
      {/each}
    </div>
    <div class="actions">
      {#if agent.profiles.some((p) => p.id === editing!.id)}<button class="btn ghost danger-text" onclick={remove}>{t("harn.delete")}</button>{/if}
      <span class="grow"></span>
      <button class="btn ghost" onclick={() => (editing = null)}>{t("common.cancel")}</button>
      <button class="btn primary" onclick={save} disabled={!extraParsed}>{t("harn.save")}</button>
    </div>
  </div>
{/if}

<style>
  .head-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .head-row h3 {
    margin-bottom: 10px;
  }
  .hname {
    font-weight: 550;
    min-width: 96px;
  }
  .ver {
    font-size: 11.5px;
    color: var(--text-2);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
    color: var(--faint);
  }
  .grow {
    flex: 1;
  }
  .hint.err {
    color: var(--danger);
  }
  .editor {
    display: grid;
    gap: 16px;
  }
  fieldset {
    border: 0;
    margin: 0;
    padding: 0;
    gap: 7px;
  }
  legend {
    padding: 0;
    margin-bottom: 6px;
  }
  .seg {
    justify-self: start;
  }
  .note {
    display: flex;
    gap: 8px;
    margin: 6px 0 0;
    padding: 9px 11px;
    border-radius: var(--r);
    background: var(--accent-soft);
    color: var(--text-2);
    font-size: var(--fs-meta);
    max-width: 70ch;
  }
  .note :global(svg) {
    flex: none;
    margin-top: 2px;
    color: var(--accent-text);
  }
  .toggle {
    align-items: flex-start;
  }
  .toggle input {
    margin-top: 3px;
  }
  .toggle strong {
    font-weight: 550;
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  .danger-text {
    color: var(--danger);
  }
</style>
