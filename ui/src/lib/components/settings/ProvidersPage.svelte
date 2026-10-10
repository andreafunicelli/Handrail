<script lang="ts" module>
  import type { ProviderKind, ProviderPreset } from "../../api/types";
  export const PRESETS: Record<ProviderPreset, { name: string; kind: ProviderKind; base_url: string; model: string; keyless?: boolean }> = {
    anthropic: { name: "Anthropic", kind: "anthropic", base_url: "https://api.anthropic.com", model: "claude-sonnet-4-5" },
    open_ai: { name: "OpenAI", kind: "open_ai", base_url: "https://api.openai.com/v1", model: "gpt-5" },
    deep_seek: { name: "DeepSeek", kind: "open_ai_compat", base_url: "https://api.deepseek.com/v1", model: "deepseek-chat" },
    open_code_go: { name: "OpenCode Go", kind: "open_ai_compat", base_url: "https://opencode.ai/zen/go/v1", model: "" },
    open_code_zen: { name: "OpenCode Zen", kind: "open_ai_compat", base_url: "https://opencode.ai/zen/v1", model: "" },
    open_router: { name: "OpenRouter", kind: "open_ai_compat", base_url: "https://openrouter.ai/api/v1", model: "" },
    ollama: { name: "Ollama", kind: "open_ai_compat", base_url: "http://localhost:11434/v1", model: "", keyless: true },
    lm_studio: { name: "LM Studio", kind: "open_ai_compat", base_url: "http://localhost:1234/v1", model: "", keyless: true },
    custom: { name: "Custom", kind: "open_ai_compat", base_url: "", model: "" },
  };
</script>

<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { Provider } from "../../api/types";
  import { agent } from "../../stores/agent.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { uuid } from "../../util/id";

  let editing = $state<Provider | null>(null);
  let isNew = $state(false);
  let apiKey = $state("");
  let models = $state<string[]>([]);
  let loadingModels = $state(false);
  let testing = $state(false);
  let testResult = $state<{ ok: boolean; message: string | null } | null>(null);

  function add(preset: ProviderPreset = "anthropic") {
    const p = PRESETS[preset];
    editing = { id: uuid(), name: p.name, kind: p.kind, preset, base_url: p.base_url, api_key: null, default_model: p.model };
    isNew = true;
    reset();
  }
  function edit(p: Provider) {
    editing = structuredClone($state.snapshot(p) as Provider);
    isNew = false;
    reset();
    void loadModels();
  }
  function reset() {
    apiKey = "";
    models = [];
    testResult = null;
  }
  function setPreset(preset: ProviderPreset) {
    if (!editing) return;
    const p = PRESETS[preset];
    editing = { ...editing, preset, kind: p.kind, base_url: p.base_url, name: isNew ? p.name : editing.name, default_model: editing.default_model || p.model };
  }

  async function save() {
    if (!editing) return;
    try {
      await api.saveProvider($state.snapshot(editing) as Provider, apiKey || undefined);
      apiKey = "";
      await agent.reload();
      toasts.success(t("prov.saved"));
      editing = null;
    } catch (e) {
      toasts.error(e);
    }
  }
  async function remove(p: Provider) {
    if (!(await ui.ask(t("ctx.deleteConfirm", { name: p.name }), { confirmLabel: t("prov.delete"), danger: true }))) return;
    try {
      await api.deleteProvider(p.id);
      await agent.reload();
      if (editing?.id === p.id) editing = null;
    } catch (e) {
      toasts.error(e);
    }
  }
  async function loadModels() {
    if (!editing || isNew) return;
    loadingModels = true;
    try {
      models = await api.listModels(editing.id);
    } catch (e) {
      toasts.error(e);
    } finally {
      loadingModels = false;
    }
  }
  async function test() {
    if (!editing) return;
    testing = true;
    try {
      if (isNew || apiKey) {
        await api.saveProvider($state.snapshot(editing) as Provider, apiKey || undefined);
        apiKey = "";
        isNew = false;
        await agent.reload();
        editing = structuredClone($state.snapshot(agent.providers.find((p) => p.id === editing!.id) ?? editing));
      }
      testResult = await api.testProvider(editing!.id);
      if (testResult.ok) void loadModels();
    } catch (e) {
      testResult = { ok: false, message: String(e) };
    } finally {
      testing = false;
    }
  }
</script>

<p class="intro">{t("prov.body")}</p>

<div class="card">
  {#each agent.providers as p (p.id)}
    <div class="list-row">
      <span class="pname">{p.name}</span>
      <span class="pmodel mono">{p.default_model}</span>
      <span class="grow"></span>
      <span class="pill {p.api_key ? 'ok' : ''}">{p.api_key ? t("prov.keyStored") : t("prov.noKey")}</span>
      <button class="btn sm ghost" onclick={() => edit(p)}><Icon name="edit" size={13} />{t("ctx.edit")}</button>
      <button class="icon-btn" aria-label={t("prov.delete")} onclick={() => remove(p)}><Icon name="trash" size={14} /></button>
    </div>
  {:else}
    <div class="list-row muted">{t("prov.empty")}</div>
  {/each}
</div>
{#if !editing}
  <button class="btn add" onclick={() => add()}><Icon name="plus" size={14} />{t("prov.add")}</button>
{/if}

{#if editing}
  <h3>{isNew ? t("prov.add") : editing.name}</h3>
  <div class="presets" role="group" aria-label={t("prov.preset")}>
    {#each Object.entries(PRESETS) as [id, p] (id)}
      <button class="preset" aria-pressed={editing.preset === id} onclick={() => setPreset(id as ProviderPreset)}>{p.name}</button>
    {/each}
  </div>
  <div class="form-grid">
    <label class="field"><span class="label">{t("prov.name")}</span><input class="input" bind:value={editing.name} /></label>
    <label class="field"><span class="label">{t("prov.baseUrl")}</span><input class="input mono" bind:value={editing.base_url} spellcheck="false" /></label>
    <label class="field">
      <span class="label">{t("prov.apiKey")}</span>
      <input
        class="input mono"
        type="password"
        autocomplete="off"
        bind:value={apiKey}
        placeholder={editing.api_key ? t("prov.apiKeySaved") : PRESETS[editing.preset].keyless ? t("prov.apiKeyNone") : "sk-…"}
      />
    </label>
    <label class="field">
      <span class="label">{t("prov.model")}</span>
      <div class="model-row">
        {#if models.length}
          <select class="select mono" bind:value={editing.default_model}>
            {#if !models.includes(editing.default_model)}<option value={editing.default_model}>{editing.default_model || "—"}</option>{/if}
            {#each models as m (m)}<option value={m}>{m}</option>{/each}
          </select>
        {:else}
          <input class="input mono" bind:value={editing.default_model} spellcheck="false" />
        {/if}
        <button class="icon-btn" title={t("prov.loadModels")} aria-label={t("prov.loadModels")} disabled={isNew || loadingModels} onclick={loadModels}><Icon name="refresh" size={14} /></button>
      </div>
    </label>
  </div>
  <div class="actions">
    <button class="btn" onclick={test} disabled={testing}>{testing ? t("prov.testing") : t("prov.test")}</button>
    {#if testResult}
      <span class="result" class:ok={testResult.ok}><Icon name={testResult.ok ? "check" : "alert"} size={13} />{testResult.message}</span>
    {/if}
    <span class="grow"></span>
    <button class="btn ghost" onclick={() => (editing = null)}>{t("common.cancel")}</button>
    <button class="btn primary" onclick={save} disabled={!editing.name.trim() || !editing.base_url.trim()}>{t("prov.save")}</button>
  </div>
{/if}

<style>
  .pname {
    font-weight: 550;
  }
  .pmodel {
    font-size: 11.5px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .muted {
    color: var(--faint);
  }
  .add {
    margin-top: 12px;
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 16px;
  }
  .preset {
    height: 26px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid var(--border-strong);
    font-size: var(--fs-meta);
    color: var(--text-2);
  }
  .preset[aria-pressed="true"] {
    border-color: var(--accent-line);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .model-row {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 16px;
    flex-wrap: wrap;
  }
  .result {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-meta);
    color: var(--danger);
  }
  .result.ok {
    color: var(--ok);
  }
</style>
