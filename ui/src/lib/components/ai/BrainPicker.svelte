<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import { agent, type BrainOption } from "../../stores/agent.svelte";
  import { ui } from "../../stores/ui.svelte";

  let { sessionId }: { sessionId: string | null } = $props();
  let open = $state(false);
  let root: HTMLDivElement;
  let models = $state<Record<string, string[]>>({});

  const current = $derived(agent.brainFor(sessionId));
  const builtIn = $derived(agent.options.filter((o) => !o.harness));
  const harness = $derived(agent.options.filter((o) => o.harness));

  function pick(o: BrainOption) {
    agent.setBrain(sessionId, o.key);
    open = false;
  }

  async function loadModels(providerId: string) {
    if (models[providerId]) return;
    try {
      models[providerId] = await api.listModels(providerId);
    } catch {
      models[providerId] = [];
    }
  }

  $effect(() => {
    if (current?.brain.type === "built_in") void loadModels(current.brain.provider);
  });

  onMount(() => {
    const away = (e: PointerEvent) => open && !root.contains(e.target as Node) && (open = false);
    addEventListener("pointerdown", away, true);
    return () => removeEventListener("pointerdown", away, true);
  });
</script>

<div class="picker" bind:this={root}>
  {#if current}
    <button class="trigger" aria-haspopup="listbox" aria-expanded={open} onclick={() => (open = !open)}>
      <Icon name={current.harness ? "terminal" : "sparkle"} size={13} />
      <span class="lbl">{current.label}</span>
      {#if current.harness}<span class="pill accent">{t("ai.subscription")}</span>{/if}
      <Icon name="chevronDown" size={12} />
    </button>
    {#if current.brain.type === "built_in" && models[current.brain.provider]?.length}
      {@const pid = current.brain.provider}
      <select class="model mono" aria-label="Model" value={current.brain.model} onchange={(e) => agent.setModel(pid, (e.currentTarget as HTMLSelectElement).value)}>
        {#each models[pid] as m (m)}<option value={m}>{m}</option>{/each}
      </select>
    {/if}
  {:else}
    <button class="trigger empty" onclick={() => ui.openSettings("providers")}>{t("ai.noBrains")}</button>
  {/if}

  {#if open}
    <div
      class="pop"
      role="listbox"
      tabindex="-1"
      aria-label={t("ai.brain")}
      onkeydown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          open = false;
        }
      }}
    >
      {#if builtIn.length}
        <div class="grp">{t("ai.builtIn")}</div>
        {#each builtIn as o (o.key)}
          <button role="option" aria-selected={o.key === current?.key} onclick={() => pick(o)}>
            <span class="nm">{o.label}</span>
            <span class="dt mono">{o.detail}</span>
            {#if o.key === current?.key}<Icon name="check" size={13} />{/if}
          </button>
        {/each}
      {/if}
      {#if harness.length}
        <div class="grp">{t("ai.harness")}</div>
        {#each harness as o (o.key)}
          <button role="option" aria-selected={o.key === current?.key} onclick={() => pick(o)}>
            <span class="nm">{o.label}</span>
            <span class="pill accent">{t("ai.subscription")}</span>
            {#if o.key === current?.key}<Icon name="check" size={13} />{/if}
          </button>
        {/each}
      {/if}
      <div class="sep"></div>
      <button onclick={() => ((open = false), ui.openSettings("providers"))}><Icon name="settings" size={13} /><span class="nm">{t("ai.openSettings")}</span></button>
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .trigger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 8px;
    border-radius: var(--r);
    min-width: 0;
    color: var(--text);
    font-weight: 550;
  }
  .trigger :global(svg) {
    flex: none;
    color: var(--muted);
  }
  .trigger:hover {
    background: var(--hover);
  }
  .trigger.empty {
    font-weight: 400;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  .lbl {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .model {
    min-width: 0;
    max-width: 150px;
    height: 22px;
    padding: 0 6px;
    border-radius: var(--r-sm);
    border: 1px solid var(--border);
    background: var(--bg);
    font-size: 11px;
    color: var(--muted);
    outline: none;
  }
  .pop {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 30;
    width: 270px;
    padding: 4px;
    border-radius: var(--r-lg);
    background: var(--raised);
    box-shadow: var(--shadow-float);
    animation: pop var(--t-fast) var(--ease);
  }
  .grp {
    padding: 6px 8px 3px;
    font-size: var(--fs-small);
    color: var(--faint);
  }
  .pop button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 30px;
    padding: 0 8px;
    border-radius: var(--r-sm);
    text-align: left;
    color: var(--text-2);
  }
  .pop button:hover,
  .pop button:focus-visible {
    background: var(--hover);
    color: var(--text);
    outline: none;
  }
  .pop button :global(svg) {
    color: var(--accent-text);
    flex: none;
  }
  .nm {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dt {
    font-size: 10.5px;
    color: var(--faint);
  }
  .sep {
    height: 1px;
    margin: 4px 2px;
    background: var(--border);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-2px);
    }
  }
</style>
