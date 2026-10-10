<script lang="ts">
  import { toasts } from "../../stores/toasts.svelte";
  import { t } from "../../i18n";
  import Icon from "../icons/Icon.svelte";
</script>

<div class="toasts" role="region" aria-live="polite">
  {#each toasts.items as toast (toast.id)}
    <div class="toast {toast.kind}" role={toast.kind === "error" ? "alert" : "status"}>
      <Icon name={toast.kind === "error" ? "alert" : toast.kind === "success" ? "check" : "info"} size={14} />
      <span class="msg">{toast.message}</span>
      <button class="icon-btn" aria-label={t("common.dismiss")} onclick={() => toasts.dismiss(toast.id)}>
        <Icon name="x" size={13} />
      </button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 100;
    display: grid;
    gap: 8px;
    width: min(360px, calc(100vw - 32px));
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    padding: 9px 6px 9px 12px;
    border-radius: var(--r-lg);
    background: var(--raised);
    box-shadow: var(--shadow-float);
    animation: in var(--t) var(--ease);
    color: var(--text-2);
  }
  .toast > :global(svg) {
    margin-top: 2px;
    flex: none;
  }
  .toast.error > :global(svg) {
    color: var(--danger);
  }
  .toast.success > :global(svg) {
    color: var(--ok);
  }
  .msg {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    padding-top: 1px;
  }
  .icon-btn {
    width: 22px;
    height: 22px;
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
</style>
