<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  /**
   * A `.btn` that shows a spinner and `busyLabel` while `busy`, without changing width: both
   * faces share one grid cell, so the button is as wide as the wider one from the start. While
   * busy it ignores clicks but keeps focus (a disabled button would drop it). Ported from
   * Interior's loading-button (github.com/ddoemonn/interior, MIT).
   */
  let {
    busy = false,
    busyLabel,
    children,
    class: cls = "",
    disabled = false,
    onclick,
    ...rest
  }: HTMLButtonAttributes & { busy?: boolean; busyLabel: string; children: Snippet } = $props();
</script>

<button
  {...rest}
  class="btn busy-btn {cls}"
  disabled={disabled && !busy}
  aria-disabled={busy || undefined}
  aria-busy={busy || undefined}
  onclick={(e) => {
    if (busy) return e.preventDefault();
    onclick?.(e);
  }}
>
  <span class="face" class:off={busy}>{@render children()}</span>
  <span class="face" class:off={!busy} aria-hidden={!busy}>
    <svg class="spin" width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
      <circle cx="6" cy="6" r="4.5" stroke="currentColor" stroke-width="1.5" stroke-opacity="0.25" />
      <path d="M10.5 6A4.5 4.5 0 0 0 6 1.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
    </svg>{busyLabel}
  </span>
</button>

<style>
  .busy-btn {
    display: inline-grid;
  }
  .busy-btn[aria-busy="true"] {
    cursor: progress;
  }
  .face {
    grid-area: 1 / 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    white-space: nowrap;
    transition: opacity var(--t) var(--ease), visibility var(--t);
  }
  .face.off {
    opacity: 0;
    visibility: hidden;
  }
  .spin {
    flex: none;
    animation: spin 0.85s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
