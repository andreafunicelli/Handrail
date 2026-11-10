<script lang="ts">
  import { t } from "../../i18n";
  import type { AgentStatus } from "../../util/agentActivity";

  let { status, live = false }: { status: AgentStatus; live?: boolean } = $props();
  const busy = $derived(status === "thinking" || status === "running");
  const label = $derived(t(`ai.status.${status}`));
</script>

<!-- Agent state, not the terminal's connection state (that one is on the terminal). -->
<span
  class="chip s-{status}"
  class:busy
  title={t("ai.status.hint")}
  role={live ? "status" : undefined}
  aria-label={t("ai.status.label", { status: label })}
>
  <span class="dot" aria-hidden="true"></span>{label}
</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 8px;
    border-radius: 999px;
    font-size: var(--fs-small);
    font-weight: 550;
    white-space: nowrap;
    background: var(--hover);
    color: var(--muted);
    flex: none;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.8;
  }
  .busy .dot {
    animation: pulse 1.2s var(--ease) infinite;
  }
  .s-thinking,
  .s-running {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .s-waiting {
    background: var(--warn-soft);
    color: var(--warn);
  }
  .s-completed {
    background: var(--ok-soft);
    color: var(--ok);
  }
  .s-failed {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .s-limit {
    background: var(--warn-soft);
    color: var(--warn);
  }
</style>
