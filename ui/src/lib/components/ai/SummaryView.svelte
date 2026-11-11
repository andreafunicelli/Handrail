<script lang="ts">
  import { segmented } from "../../actions/segmented";
  import Icon from "../icons/Icon.svelte";
  import Markdown from "../common/Markdown.svelte";
  import AgentStatusChip from "./AgentStatusChip.svelte";
  import { fmtDateTime, t, type MessageKey } from "../../i18n";
  import type { Chat } from "../../stores/agent.svelte";
  import { buildSummary, latestText, runStart, type AgentStatus } from "../../util/agentActivity";
  import { fmtMs, jumpToTerminal, type Target } from "./target";

  let {
    chat,
    sessionId,
    target,
    status,
    agentLabel,
    canAsk,
    onask,
    onactivity,
  }: {
    chat: Chat | undefined;
    sessionId: string;
    target: Target | null;
    status: AgentStatus;
    agentLabel: string | null;
    canAsk: boolean;
    onask: () => void;
    onactivity: () => void;
  } = $props();

  let scope = $state<"run" | "all">("run");
  const MAX_LIST = 8;

  const items = $derived(chat?.items ?? []);
  const from = $derived(scope === "all" ? 0 : runStart(items, [t("ai.continueMsg")]));
  const s = $derived(buildSummary(items, from));
  // The only derivation that reads streamed text: a backward scan to the last assistant item.
  const conclusion = $derived(latestText(items, from));
  const shownCommands = $derived(s.commands.slice(-MAX_LIST));
</script>

<div class="summary">
  {#if !items.length}
    <p class="empty">{t("ai.summary.none")}</p>
  {:else}
    <div class="seg scope" role="group" aria-label={t("ai.summary.scope")} use:segmented>
      <button aria-pressed={scope === "run"} onclick={() => (scope = "run")}>{t("ai.summary.scope.run")}</button>
      <button aria-pressed={scope === "all"} onclick={() => (scope = "all")}>{t("ai.summary.scope.all")}</button>
    </div>

    {#if s.objective}
      <section>
        <h3 title={t("ai.summary.objectiveHint")}>{t("ai.summary.objective")}</h3>
        <p class="objective">{s.objective}</p>
        {#if s.startedAt}<p class="sub">{t("ai.summary.started", { time: fmtDateTime(s.startedAt) })} · {t("ai.summary.counts", { prompts: s.prompts, tools: s.tools })}</p>{/if}
      </section>
    {/if}

    <dl class="facts">
      <dt>{t("ai.summary.target")}</dt>
      <dd>
        {#if target}
          <span class="strong">{target.name}</span>
          {#if target.env === "prod"}<span class="env-prod-badge">{t("ai.prodBadge")}</span>{:else if target.env !== "none"}<span class="dim">{t(`env.${target.env}`)}</span>{/if}
          {#if target.user || target.host}<span class="mono dim">{target.user ?? ""}{target.user && target.host ? "@" : ""}{target.host ?? ""}</span>{/if}
        {:else}—{/if}
      </dd>
      <dt>{t("ai.summary.agent")}</dt>
      <dd>{agentLabel ?? "—"}</dd>
      <dt>{t("ai.summary.status")}</dt>
      <dd><AgentStatusChip {status} /></dd>
    </dl>

    <section>
      <h3>{t("ai.summary.commands")} <span class="count">{s.commands.length}</span></h3>
      {#if s.commands.length}
        <ul class="list">
          {#each shownCommands as c (c.id)}
            <li>
              <button class="cmdrow" onclick={() => jumpToTerminal(sessionId, c.command)} title={t("tool.jump")}>
                <code class="mono cmd">{c.command ?? c.tool}</code>
                {#if c.exitCode != null}<span class="exit" class:bad={c.exitCode !== 0}>{c.exitCode}</span>
                {:else}<span class="exit dim">{t(`tool.status.${c.exec}` as MessageKey)}</span>{/if}
                {#if c.durationMs != null && c.item.done && c.exec !== "denied"}<span class="dur">{fmtMs(c.durationMs)}</span>{/if}
              </button>
            </li>
          {/each}
        </ul>
        {#if s.commands.length > MAX_LIST}<button class="link" onclick={onactivity}>{t("ai.summary.more", { n: s.commands.length - MAX_LIST })}</button>{/if}
      {:else}
        <p class="none">{t("ai.summary.noCommands")}</p>
      {/if}
    </section>

    <section>
      <h3>{t("ai.summary.errors")} <span class="count" class:bad={s.errors.length > 0}>{s.errors.length}</span></h3>
      {#if s.errors.length}
        <ul class="list">
          {#each s.errors.slice(-MAX_LIST) as e (e.id)}
            <li class="errrow">
              <Icon name="alert" size={12} />
              {#if e.entry}
                <span><code class="mono">{e.entry.command ?? e.entry.path ?? e.entry.tool}</code> — {t(`tool.status.${e.entry.exec}` as MessageKey)}{#if e.entry.exitCode != null}, {t("tool.exit", { code: e.entry.exitCode })}{/if}{#if e.message}: {e.message}{/if}</span>
              {:else}
                <span>{e.message}</span>
              {/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="none">{t("ai.summary.noErrors")}</p>
      {/if}
    </section>

    <section>
      <h3>{t("ai.summary.changes")} <span class="count">{s.changes.length}</span></h3>
      {#if s.changes.length}
        <ul class="list">
          {#each s.changes as c (c.id)}
            <li class="change">
              <Icon name={c.kind === "file" ? "file" : "terminal"} size={12} />
              <code class="mono cmd" title={c.text}>{c.text}</code>
              {#if c.kind === "file"}
                <span class="pill {c.applied ? 'ok' : c.detail === 'failed' ? 'warn' : ''}">{t(`ai.summary.file.${c.detail}` as MessageKey)}</span>
              {:else}
                <span class="pill {c.detail === 'destructive' ? 'danger' : 'warn'}">{t(`approval.risk.${c.detail}` as MessageKey)}</span>
                <span class="dim">{c.applied ? t("ai.summary.cmd.ran") : t("ai.summary.cmd.notRun")}</span>
              {/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="none">{t("ai.summary.noChanges")}</p>
      {/if}
      {#if s.unclassified}<p class="sub">{t("ai.summary.unclassified", { n: s.unclassified })}</p>{/if}
    </section>

    {#if conclusion}
      <section>
        <h3>{t("ai.summary.conclusion")}</h3>
        <p class="sub">{t("ai.summary.conclusionHint")}</p>
        <blockquote class="conclusion"><Markdown source={conclusion} /></blockquote>
      </section>
    {/if}
  {/if}

  <div class="ask">
    <button class="btn sm" disabled={!canAsk} onclick={onask}><Icon name="sparkle" size={12} />{t("ai.summary.ask")}</button>
    <p class="sub">{t("ai.summary.askHint")}</p>
  </div>
</div>

<style>
  .summary {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .scope {
    align-self: flex-start;
  }
  section {
    display: grid;
    gap: 6px;
  }
  h3 {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-small);
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .count {
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .count.bad {
    color: var(--warn);
  }
  .objective {
    margin: 0;
    color: var(--text);
    font-weight: 550;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .sub {
    margin: 0;
    font-size: var(--fs-small);
    color: var(--faint);
  }
  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 12px;
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    font-size: var(--fs-meta);
  }
  .facts dt {
    color: var(--faint);
  }
  .facts dd {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin: 0;
    min-width: 0;
    color: var(--text-2);
  }
  .strong {
    color: var(--text);
    font-weight: 550;
  }
  .dim {
    color: var(--faint);
  }
  .mono.dim {
    font-size: var(--fs-small);
  }
  .list {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .cmdrow {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 4px 6px;
    border-radius: var(--r-sm);
    text-align: left;
    font-variant-numeric: tabular-nums;
  }
  .cmdrow:hover {
    background: var(--hover);
  }
  .cmd {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-meta);
    color: var(--text);
  }
  .exit {
    font-size: var(--fs-small);
    color: var(--ok);
    font-family: var(--font-mono);
  }
  .exit.bad {
    color: var(--warn);
  }
  .exit.dim {
    font-family: inherit;
    color: var(--faint);
  }
  .dur {
    font-size: var(--fs-small);
    color: var(--faint);
  }
  .errrow {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 4px 6px;
    font-size: var(--fs-meta);
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
  .errrow :global(svg) {
    flex: none;
    margin-top: 3px;
    color: var(--warn);
  }
  .change {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 4px 6px;
    font-size: var(--fs-small);
  }
  .change :global(svg) {
    flex: none;
    color: var(--muted);
  }
  .none {
    margin: 0;
    padding: 0 6px;
    font-size: var(--fs-meta);
    color: var(--faint);
  }
  .conclusion {
    margin: 0;
    padding: 8px 12px;
    border-left: 2px solid var(--accent-line);
    background: var(--hover);
    border-radius: 0 var(--r) var(--r) 0;
    font-size: var(--fs-meta);
    max-height: 320px;
    overflow: auto;
  }
  .link {
    justify-self: start;
    color: var(--muted);
    font-size: var(--fs-small);
    padding: 0 6px;
  }
  .link:hover {
    color: var(--text);
  }
  .ask {
    display: grid;
    gap: 6px;
    justify-items: start;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .empty {
    margin: 0;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
</style>
