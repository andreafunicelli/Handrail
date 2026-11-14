<script lang="ts">
  import { onMount } from "svelte";
  import HoldButton from "../common/HoldButton.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { agent, type ApprovalItem } from "../../stores/agent.svelte";
  import { fmtClock, targetOf } from "./target";
  import { armed, revealHidden, takesText } from "../../util/approval";

  let { item, sessionId }: { item: ApprovalItem; sessionId: string } = $props();
  let editing = $state(false);
  // svelte-ignore state_referenced_locally
  let draft = $state(item.req.command);
  let card: HTMLDivElement;
  let area = $state<HTMLTextAreaElement>();

  const pending = $derived(!item.decision);
  const risk = $derived(item.req.risk);
  const target = $derived(targetOf(item.req.session_id || sessionId));
  const prod = $derived(target?.env === "prod");
  /** A destructive command is approved by holding, never by a single click or keystroke. */
  const mustHold = $derived(risk === "destructive");
  let hold = $state<HoldButton>();
  let holdEdited = $state<HoldButton>();

  const shownAt = performance.now();
  onMount(() => {
    // Take focus only if the user is already working in the sidebar, and never from where they
    // type (the terminal, the chat box): their next Enter would land on the card.
    const at = document.activeElement;
    if (pending && card.closest(".ai")?.contains(at) && !takesText(at)) card.focus();
  });
  const shownCmd = $derived(revealHidden(item.decision?.type === "approve_edited" ? item.decision.command : item.req.command));

  const approve = () => agent.decide(sessionId, item.req.approval_id, { type: "approve" });
  const deny = () => agent.decide(sessionId, item.req.approval_id, { type: "deny", message: null });
  // The backend checks an edited command against the policy again (deny list, risk, mode);
  // nothing here decides whether it may run.
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
        if (e.repeat) return;
        // An edited destructive command still has to be held; the key's release lands on the
        // focused hold button.
        if (!mustHold) runEdited();
        else if (!e.repeat && holdEdited) (holdEdited.focus(), holdEdited.begin());
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        editing = false;
        card.focus();
      }
      return;
    }
    if (e.target !== card) return;
    if (e.key === "Enter") {
      e.preventDefault();
      // A held key, or one pressed as the card appeared, was not meant for it.
      if (e.repeat || !armed(shownAt)) return;
      if (!mustHold) approve();
      else if (hold) (hold.focus(), hold.begin());
    } else if (e.key.toLowerCase() === "e") (e.preventDefault(), startEdit());
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
  class:prod
  bind:this={card}
  data-approval={item.req.approval_id}
  tabindex={pending ? 0 : -1}
  role="group"
  aria-label={t("approval.title", { actor: item.req.actor_label })}
  onkeydown={key}
>
  <div class="head">
    <Icon name="shield" size={13} />
    <span class="who">{pending ? t("approval.heading") : t("approval.title", { actor: item.req.actor_label })}</span>
    {#if prod}<span class="env-prod-badge" title={t("env.prod")}>{t("ai.prodBadge")}</span>{/if}
    <span class="pill risk {risk === 'destructive' ? 'danger' : risk === 'write' ? 'warn' : ''}">{t(`approval.risk.${risk}`)}</span>
  </div>

  {#if editing && pending}
    <textarea class="textarea mono edit" rows="2" bind:this={area} bind:value={draft} spellcheck="false" aria-label={t("approval.edit")} aria-describedby="recheck-{item.id}"></textarea>
    <p class="recheck" id="recheck-{item.id}"><Icon name="shield" size={11} />{t("approval.editRecheck")}</p>
  {:else}
    <pre class="cmd mono">{shownCmd.text}</pre>
    {#if item.decision?.type === "approve_edited"}<pre class="cmd mono was"><del>{revealHidden(item.req.command).text}</del></pre>{/if}
    {#if shownCmd.found}<p class="prodwarn" role="note"><Icon name="alert" size={12} />{t("approval.invisible")}</p>{/if}
  {/if}

  <dl class="ctx">
    <dt>{t("approval.ctx.target")}</dt>
    <dd>
      {#if target}
        <span class="strong">{target.name}</span>
        {#if target.host && !target.local}<span class="mono dim gap">{target.host}</span>{/if}
        {#if target.local}<span class="dim gap">{t("approval.ctx.local")}</span>{/if}
        {#if target.env !== "none" && !prod}<span class="dim gap">{t(`env.${target.env}`)}</span>{/if}
      {:else}
        <span class="dim">{t("approval.ctx.unknown")}</span>
      {/if}
    </dd>
    <dt>{t("approval.ctx.user")}</dt>
    <dd class="mono">{target?.user ?? t("approval.ctx.unknown")}</dd>
    <dt>{t("approval.ctx.agent")}</dt>
    <dd>{item.req.actor_label}</dd>
    {#if item.req.reason}
      <dt>{t("approval.ctx.reason")}</dt>
      <dd class="reason">{item.req.reason}</dd>
    {/if}
    {#if item.req.policy_note}
      <dt>{t("approval.ctx.policy")}</dt>
      <dd class="policy">{item.req.policy_note}</dd>
    {/if}
  </dl>

  {#if prod && pending}<p class="prodwarn" role="note"><Icon name="alert" size={12} />{t("approval.ctx.prodWarn")}</p>{/if}

  <details class="tech">
    <summary>{t("approval.ctx.details")}</summary>
    <dl class="ctx">
      <dt>{t("approval.ctx.risk")}</dt>
      <dd>{t(`approval.risk.${risk}`)}</dd>
      <dt>{t("approval.ctx.approvalId")}</dt>
      <dd class="mono">{item.req.approval_id}</dd>
      <dt>{t("approval.ctx.session")}</dt>
      <dd class="mono">{item.req.session_id}</dd>
      {#if target?.connectionId}
        <dt>{t("approval.ctx.connection")}</dt>
        <dd class="mono">{target.connectionId}</dd>
      {/if}
      {#if target?.info}
        <dt>{t("approval.ctx.mode")}</dt>
        <dd>{t(`ai.mode.${target.info.approval_mode}`)}</dd>
        {#if target.info.shell}<dt>{t("approval.ctx.shell")}</dt><dd class="mono">{target.info.shell}</dd>{/if}
        {#if target.info.cwd}<dt>{t("approval.ctx.cwd")}</dt><dd class="mono">{target.info.cwd}</dd>{/if}
      {/if}
    </dl>
  </details>

  {#if pending}
    <div class="actions">
      {#if editing}
        {#if mustHold}
          <HoldButton bind:this={holdEdited} size="sm" onconfirm={runEdited} confirmLabel={t("approval.approvedEdited")} title={t("approval.holdHint")}
            >{t("approval.holdRunEdited")} <span class="k">⏎</span></HoldButton
          >
        {:else}
          <button class="btn sm primary" onclick={runEdited}>{t("approval.runEdited")} <span class="k">⏎</span></button>
        {/if}
        <button class="btn sm ghost" onclick={() => (editing = false)}>{t("common.cancel")} <span class="k">Esc</span></button>
      {:else}
        {#if mustHold}
          <HoldButton bind:this={hold} size="sm" onconfirm={approve} confirmLabel={t("approval.approved")} title={t("approval.holdHint")}
            >{t("approval.holdApprove")} <span class="k">⏎</span></HoldButton
          >
        {:else}
          <button class="btn sm primary" onclick={approve}>{t("approval.approve")} <span class="k">⏎</span></button>
        {/if}
        <button class="btn sm" onclick={startEdit}>{t("approval.edit")} <span class="k">E</span></button>
        <span class="grow"></span>
        <button class="btn sm ghost deny" onclick={deny}>{t("approval.deny")} <span class="k">Esc</span></button>
      {/if}
    </div>
  {:else if item.expired}
    <div class="resolved denied" role="status"><Icon name="clock" size={12} />{t("approval.expired")}</div>
  {:else}
    <div class="resolved" class:denied={item.decision?.type === "deny"} role="status">
      <Icon name={item.decision?.type === "deny" ? "x" : "check"} size={12} />
      {item.decidedAt ? t("approval.decidedAt", { decision: decisionLabel, time: fmtClock(item.decidedAt) }) : decisionLabel}
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
  .approval.pending.prod {
    border-color: var(--env-prod-line);
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
    font-weight: 550;
  }
  .approval:not(.pending) .head {
    color: var(--muted);
    font-weight: 450;
  }
  .risk-destructive.pending .head {
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
    font-size: var(--fs-meta);
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .cmd.was {
    margin-top: 4px;
    color: var(--faint);
  }
  .edit {
    margin-top: 9px;
    min-height: 0;
    font-size: var(--fs-meta);
  }
  .recheck {
    display: flex;
    align-items: flex-start;
    gap: 5px;
    margin: 6px 0 0;
    font-size: var(--fs-small);
    color: var(--muted);
  }
  .recheck :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  .ctx {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 3px 10px;
    margin: 9px 0 0;
    font-size: var(--fs-small);
  }
  .ctx dt {
    color: var(--faint);
  }
  .ctx dd {
    margin: 0;
    min-width: 0;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
  .ctx .mono {
    font-size: var(--fs-small);
  }
  .strong {
    color: var(--text);
    font-weight: 550;
  }
  .dim {
    color: var(--faint);
  }
  .gap {
    margin-left: 6px;
  }
  .reason {
    color: var(--text);
  }
  .policy {
    color: var(--muted);
  }
  .prodwarn {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 8px 0 0;
    padding: 5px 8px;
    border-radius: var(--r);
    background: var(--env-prod-soft);
    color: var(--env-prod);
    font-size: var(--fs-small);
  }
  .tech {
    margin-top: 6px;
    font-size: var(--fs-small);
    color: var(--muted);
  }
  .tech summary {
    cursor: pointer;
    width: fit-content;
  }
  .tech summary:hover {
    color: var(--text);
  }
  .tech .ctx {
    margin-top: 4px;
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 11px;
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
  }
  .k {
    font-family: var(--font-mono);
    font-size: var(--fs-tiny);
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
