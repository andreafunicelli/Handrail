<script lang="ts" module>
  import type { IconName } from "../icons/Icon.svelte";
  export interface MenuItem {
    label: string;
    icon?: IconName;
    danger?: boolean;
    disabled?: boolean;
    shortcut?: string;
    action: () => void;
  }
  export type MenuEntry = MenuItem | "sep";
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";

  let { x, y, items, onclose }: { x: number; y: number; items: MenuEntry[]; onclose: () => void } = $props();
  let el: HTMLDivElement;
  let pos = $state({ left: -9999, top: -9999 });

  onMount(() => {
    const r = el.getBoundingClientRect();
    pos = { left: Math.max(8, Math.min(x, innerWidth - r.width - 8)), top: Math.max(8, Math.min(y, innerHeight - r.height - 8)) };
    el.querySelector<HTMLElement>("button:not([disabled])")?.focus();
    const away = (e: Event) => !el.contains(e.target as Node) && onclose();
    const id = setTimeout(() => {
      addEventListener("pointerdown", away, true);
      addEventListener("blur", onclose);
      addEventListener("resize", onclose);
    });
    return () => {
      clearTimeout(id);
      removeEventListener("pointerdown", away, true);
      removeEventListener("blur", onclose);
      removeEventListener("resize", onclose);
    };
  });

  function key(e: KeyboardEvent) {
    const btns = [...el.querySelectorAll<HTMLButtonElement>("button:not([disabled])")];
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      btns[(i + 1) % btns.length]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      btns[(i - 1 + btns.length) % btns.length]?.focus();
    }
  }
</script>

<div class="menu" role="menu" tabindex="-1" bind:this={el} style:left="{pos.left}px" style:top="{pos.top}px" onkeydown={key}>
  {#each items as item, i (i)}
    {#if item === "sep"}
      <div class="sep" role="separator"></div>
    {:else}
      <button
        role="menuitem"
        class:danger={item.danger}
        disabled={item.disabled}
        onclick={() => {
          onclose();
          item.action();
        }}
      >
        {#if item.icon}<Icon name={item.icon} size={14} />{:else}<span class="sp"></span>{/if}
        <span class="l">{item.label}</span>
        {#if item.shortcut}<span class="sc">{item.shortcut}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 80;
    min-width: 190px;
    padding: 4px;
    border-radius: var(--r-lg);
    background: var(--raised);
    box-shadow: var(--shadow-float);
    animation: pop var(--t-fast) var(--ease);
    outline: none;
  }
  button {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border-radius: var(--r-sm);
    text-align: left;
    color: var(--text-2);
  }
  button :global(svg) {
    color: var(--muted);
  }
  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--hover);
    color: var(--text);
    outline: none;
  }
  button.danger,
  button.danger :global(svg) {
    color: var(--danger);
  }
  button:disabled {
    opacity: 0.4;
  }
  .sp {
    width: 14px;
  }
  .l {
    flex: 1;
  }
  .sc {
    font-size: var(--fs-small);
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
      transform: scale(0.98);
    }
  }
</style>
