<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  import { cubicOut } from "svelte/easing";
  import { fade } from "svelte/transition";

  /** Closing animates out as well as in (skipped under reduced motion), and the backdrop closes
   *  the dialog only for a press that starts and ends on it: dragging a text selection out of an
   *  input no longer dismisses the form. From Interior's modal (github.com/ddoemonn/interior,
   *  MIT). */

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
  let pressedBackdrop = false;
  const still = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

  function rise(_: Element, { out = false } = {}) {
    return {
      duration: still ? 0 : out ? 120 : 160,
      easing: cubicOut,
      css: (k: number) => `opacity: ${k}; transform: translateY(${(1 - k) * 6}px) scale(${0.99 + k * 0.01})`,
    };
  }

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
<div
  class="backdrop {tone}"
  transition:fade={{ duration: still ? 0 : 140 }}
  {onkeydown}
  onpointerdown={(e) => (pressedBackdrop = e.target === e.currentTarget)}
  onclick={(e) => {
    if (pressedBackdrop && e.target === e.currentTarget && dismissable) onclose?.();
    pressedBackdrop = false;
  }}
>
  <div class="box {tone}" role="dialog" aria-modal="true" aria-label={label} bind:this={box} style:--w={width} in:rise out:rise={{ out: true }}>
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
  }
  .box.danger {
    box-shadow: 0 0 0 1px var(--danger), var(--shadow-float);
  }
</style>
