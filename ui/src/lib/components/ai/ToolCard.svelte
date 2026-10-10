<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t, type MessageKey } from "../../i18n";
  import { agent, type ChatItem } from "../../stores/agent.svelte";
  import type { CommandResult } from "../../api/types";
  import { sessions } from "../../stores/sessions.svelte";
  import { terminals } from "../../terminal/host";

  let { item, sessionId }: { item: Extract<ChatItem, { kind: "tool" }>; sessionId: string } = $props();
  let open = $state(false);

  const isCommand = $derived(item.name === "run_command");
  const result = $derived(
    item.output && typeof item.output === "object" && "state" in (item.output as object) ? (item.output as CommandResult) : null,
  );
  const command = $derived(result?.command ?? (typeof item.input.command === "string" ? item.input.command : ""));
  const status = $derived(!item.done ? (agent.pendingApprovals(sessionId).length ? "waiting" : "running") : item.isError ? "error" : (result?.state ?? "completed"));
  const tone = $derived(
    status === "running" ? "accent" : status === "completed" && (result?.exit_code ?? 0) === 0 ? "ok" : status === "denied" ? "" : "warn",
  );
  const outputText = $derived(
    result ? result.output : item.output == null ? "" : typeof item.output === "string" ? item.output : JSON.stringify(item.output, null, 2),
  );

  function fmtMs(ms: number) {
    return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)} s`;
  }
  function jump() {
    sessions.focusSession(sessionId);
    requestAnimationFrame(() => terminals.peek(sessionId)?.jumpTo(command));
  }
</script>

<div class="tool" class:running={status === "running"}>
  <div class="top">
    <span class="glyph" aria-hidden="true">{isCommand ? "✦" : "⋯"}</span>
    {#if isCommand}
      <code class="cmd mono">{command}</code>
    {:else}
      <span class="tname mono">{item.name}</span>
    {/if}
    <span class="pill {tone}">{t(`tool.status.${status}` as MessageKey)}</span>
  </div>
  {#if result && status !== "denied"}
    <div class="meta">
      {#if result.exit_code != null}<span class:bad={result.exit_code !== 0}>{t("tool.exit", { code: result.exit_code })}</span>{/if}
      <span>{fmtMs(result.duration_ms)}</span>
      {#if result.truncated}<span>truncated</span>{/if}
      <span class="grow"></span>
      {#if outputText}
        <button class="link" onclick={() => (open = !open)} aria-expanded={open}>
          <Icon name={open ? "chevronDown" : "chevronRight"} size={11} />{open ? t("tool.hideOutput") : t("tool.output")}
        </button>
      {/if}
      {#if isCommand}
        <button class="link" onclick={jump}><Icon name="jump" size={11} />{t("tool.jump")}</button>
      {/if}
    </div>
  {:else if !isCommand && item.done && outputText}
    <div class="meta">
      <span class="grow"></span>
      <button class="link" onclick={() => (open = !open)} aria-expanded={open}>{open ? t("tool.hideOutput") : t("tool.output")}</button>
    </div>
  {/if}
  {#if result?.note}<p class="note">{result.note}</p>{/if}
  {#if open}
    <pre class="out mono">{outputText || t("tool.noOutput")}</pre>
  {/if}
</div>

<style>
  .tool {
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--panel);
    overflow: hidden;
  }
  .tool.running {
    border-color: var(--accent-line);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    min-width: 0;
  }
  .glyph {
    color: var(--accent);
    font-size: 10px;
    flex: none;
  }
  .running .glyph {
    animation: pulse 1.4s var(--ease) infinite;
  }
  .cmd,
  .tname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
    color: var(--text);
  }
  .tname {
    color: var(--muted);
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px 8px 28px;
    font-size: var(--fs-small);
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .meta .bad {
    color: var(--warn);
  }
  .grow {
    flex: 1;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--muted);
    font-size: var(--fs-small);
  }
  .link:hover {
    color: var(--text);
  }
  .note {
    margin: 0;
    padding: 0 10px 8px 28px;
    font-size: var(--fs-small);
    color: var(--muted);
  }
  .out {
    margin: 0;
    padding: 9px 11px;
    max-height: 260px;
    overflow: auto;
    font-size: 11.5px;
    line-height: 1.5;
    white-space: pre;
    background: var(--bg);
    border-top: 1px solid var(--border);
    color: var(--text-2);
  }
</style>
