<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { agent, type ChatItem } from "../../stores/agent.svelte";

  let { item, sessionId }: { item: Extract<ChatItem, { kind: "approval" }>; sessionId: string } = $props();
  let editing = $state(false);
  // svelte-ignore state_referenced_locally
  let draft = $state(item.req.command);
  let card: HTMLDivElement;
  let area = $state<HTMLTextAreaElement>();

  const pending = $derived(!item.decision);
  const risk = $derived(item.req.risk);

  onMount(() => {
    // Take focus only if the user is already working in the sidebar (never steal it from a terminal).
    if (pending && card.closest(".ai")?.contains(document.activeElement)) card.focus();
  });

  const approve = () => agent.decide(sessionId, item.req.approval_id, { type: "approve" });
  const deny = () => agent.decide(sessionId, item.req.approval_id, { type: "deny", message: null });
  function runEdited() {
    const cmd = draft.trim();
    if (!cmd) return;
    agent.decide(sessionId, item.req.approval_id, cmd === item.req.command ? { type: "approve" } : { type: "approve_edited", command: cmd });
  }
  function startEdit() {
    editing = true;
    requestAnimationFrame(() => area?.focus());
  }

  function key(e: KeyboardEvent) {
    if (!pending) return;
    if (editing) {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey || !e.shiftKey)) {
        e.preventDefault();
        runEdited();
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        editing = false;
        card.focus();
      }
      return;
    }
    if (e.target !== card) return;
    if (e.key === "Enter") (e.preventDefault(), approve());
    else if (e.key.toLowerCase() === "e") (e.preventDefault(), startEdit());
    else if (e.key === "Escape") (e.preventDefault(), e.stopPropagation(), deny());
  }

  const decisionLabel = $derived(
    !item.decision
      ? ""
      : item.decision.type === "approve"
        ? t("approval.approved")
        : item.decision.type === "approve_edited"
          ? t("approval.approvedEdited")
          : t("approval.denied"),
  );
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="approval risk-{risk}"
  class:pending
  bind:this={card}
  tabindex={pending ? 0 : -1}
  role="group"
  aria-label={t("approval.title", { actor: item.req.actor_label })}
  onkeydown={key}
>
  <div class="head">
    <Icon name="shield" size={13} />
    <span class="who">{t("approval.title", { actor: item.req.actor_label })}</span>
    <span class="pill risk {risk === 'destructive' ? 'danger' : risk === 'write' ? 'warn' : ''}">{t(`approval.risk.${risk}`)}</span>
  </div>

  {#if editing && pending}
    <textarea class="textarea mono edit" rows="2" bind:this={area} bind:value={draft} spellcheck="false"></textarea>
  {:else}
    <pre class="cmd mono">{item.decision?.type === "approve_edited" ? item.decision.command : item.req.command}</pre>
  {/if}

  {#if item.req.reason}<p class="reason">{item.req.reason}</p>{/if}
  {#if item.req.policy_note}<p class="policy">{item.req.policy_note}</p>{/if}

  {#if pending}
    <div class="actions">
      {#if editing}
        <button class="btn sm primary" onclick={runEdited}>{t("approval.runEdited")} <span class="k">⏎</span></button>
        <button class="btn sm ghost" onclick={() => (editing = false)}>{t("common.cancel")} <span class="k">Esc</span></button>
      {:else}
        <button class="btn sm primary" onclick={approve}>{t("approval.approve")} <span class="k">⏎</span></button>
        <button class="btn sm" onclick={startEdit}>{t("approval.edit")} <span class="k">E</span></button>
        <button class="btn sm ghost" onclick={deny}>{t("approval.deny")} <span class="k">Esc</span></button>
      {/if}
    </div>
  {:else}
    <div class="resolved" class:denied={item.decision?.type === "deny"}>
      <Icon name={item.decision?.type === "deny" ? "x" : "check"} size={12} />{decisionLabel}
    </div>
  {/if}
</div>

<style>
  .approval {
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--panel);
    padding: 10px 12px 12px;
    outline: none;
  }
  .approval.pending {
    border-color: var(--accent-line);
    background: color-mix(in srgb, var(--accent) 5%, var(--panel));
  }
  .approval.pending.risk-destructive {
    border-color: var(--danger);
    background: color-mix(in srgb, var(--danger) 6%, var(--panel));
  }
  .approval:focus-visible {
    box-shadow: 0 0 0 2px var(--accent);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--accent-text);
    font-size: var(--fs-meta);
  }
  .risk-destructive .head {
    color: var(--danger);
  }
  .who {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cmd {
    margin: 9px 0 0;
    padding: 8px 10px;
    border-radius: var(--r);
    background: var(--bg);
    border: 1px solid var(--border);
    font-size: 12.5px;
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .edit {
    margin-top: 9px;
    min-height: 0;
    font-size: 12.5px;
  }
  .reason {
    margin: 8px 0 0;
    color: var(--text-2);
    font-size: var(--fs-meta);
  }
  .policy {
    margin: 4px 0 0;
    color: var(--faint);
    font-size: var(--fs-small);
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 11px;
    flex-wrap: wrap;
  }
  .k {
    font-family: var(--font-mono);
    font-size: 10px;
    opacity: 0.6;
  }
  .resolved {
    display: flex;
    align-items: center;
    gap: 5px;
    margin-top: 9px;
    font-size: var(--fs-small);
    color: var(--ok);
  }
  .resolved.denied {
    color: var(--muted);
  }
</style>
