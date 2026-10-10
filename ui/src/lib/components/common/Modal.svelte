<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";

  let {
    onclose,
    children,
    label,
    width = "440px",
    tone = "default",
    dismissable = true,
  }: {
    onclose?: () => void;
    children: Snippet;
    label: string;
    width?: string;
    tone?: "default" | "danger";
    dismissable?: boolean;
  } = $props();

  let box: HTMLDivElement;

  onMount(() => {
    const restore = document.activeElement as HTMLElement | null;
    const first =
      box.querySelector<HTMLElement>("[data-autofocus]") ?? box.querySelector<HTMLElement>("input, textarea, select, button:not([disabled])");
    first?.focus();
    return () => restore?.focus?.();
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && dismissable) {
      e.stopPropagation();
      onclose?.();
    }
    if (e.key === "Tab") {
      const items = [...box.querySelectorAll<HTMLElement>("button, input, select, textarea, [href], [tabindex]:not([tabindex='-1'])")].filter(
        (el) => !el.hasAttribute("disabled") && el.offsetParent !== null,
      );
      if (!items.length) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop {tone}" onkeydown={onkeydown} onmousedown={(e) => e.target === e.currentTarget && dismissable && onclose?.()}>
  <div class="box {tone}" role="dialog" aria-modal="true" aria-label={label} bind:this={box} style:--w={width}>
    {@render children()}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: grid;
    place-items: center;
    padding: 16px;
    background: rgba(5, 6, 8, 0.55);
    animation: fade var(--t) var(--ease);
  }
  :global([data-theme="light"]) .backdrop {
    background: rgba(30, 32, 40, 0.25);
  }
  .backdrop.danger,
  :global([data-theme="light"]) .backdrop.danger {
    background: rgba(36, 6, 8, 0.86);
  }
  .box {
    width: min(var(--w), 100%);
    max-height: calc(100vh - 32px);
    overflow: auto;
    border-radius: 10px;
    background: var(--panel);
    box-shadow: var(--shadow-float);
    animation: rise var(--t) var(--ease);
  }
  .box.danger {
    box-shadow: 0 0 0 1px var(--danger), var(--shadow-float);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(6px) scale(0.99);
    }
  }
</style>
