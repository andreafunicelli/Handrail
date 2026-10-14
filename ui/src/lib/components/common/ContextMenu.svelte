<script lang="ts" module>
  import type { IconName } from "../icons/Icon.svelte";
  export interface MenuItem {
    label: string;
    icon?: IconName;
    danger?: boolean;
    disabled?: boolean;
    shortcut?: string;
    /** A toggle: shows a check mark while on. */
    checked?: boolean;
    action: () => void;
  }
  export type MenuEntry = MenuItem | "sep";

  /** Whether the last input was a key (Shift+F10, the Menu key, Enter on a "…" button): such a
   *  menu opens with its first item focused, so Enter acts at once. */
  let keyboardLast = false;
  if (typeof window !== "undefined") {
    addEventListener("keydown", () => (keyboardLast = true), true);
    addEventListener("pointerdown", () => (keyboardLast = false), true);
  }
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";

  /**
   * Behaviour adapted from Interior's context-menu (github.com/ddoemonn/interior, MIT): it opens
   * beside the pointer, flipping left/up near the edges instead of covering it, and grows from
   * that point; hover and keys share one highlight; Home/End and typing a label's first letters
   * jump; Tab, Escape, scrolling, resizing or leaving the window close it; focus goes back where
   * it was unless the chosen item moved it on purpose (a dialog it opened).
   */
  let { x, y, items, onclose }: { x: number; y: number; items: MenuEntry[]; onclose: () => void } = $props();
  let el: HTMLDivElement;
  let pos = $state({ left: -9999, top: -9999, origin: "0 0" });
  let query = "";
  let queryTimer: ReturnType<typeof setTimeout> | undefined;

  const enabled = () => [...el.querySelectorAll<HTMLButtonElement>("button:not([disabled])")];

  onMount(() => {
    const restore = document.activeElement as HTMLElement | null;
    const r = el.getBoundingClientRect();
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;
    const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, hi));
    const left = clamp(x + r.width + 8 <= vw ? x : x - r.width, 8, vw - r.width - 8);
    const top = clamp(y + r.height + 8 <= vh ? y : y - r.height, 8, vh - r.height - 8);
    pos = { left, top, origin: `${clamp(x - left, 0, r.width)}px ${clamp(y - top, 0, r.height)}px` };
    // Opened by the pointer, nothing is highlighted until the pointer or a key picks something;
    // opened from the keyboard, the first item is.
    if (keyboardLast) (enabled()[0] ?? el).focus({ preventScroll: true });
    else el.focus({ preventScroll: true });
    const away = (e: Event) => !el.contains(e.target as Node) && onclose();
    const id = setTimeout(() => {
      addEventListener("pointerdown", away, true);
      addEventListener("scroll", away, { capture: true, passive: true });
      addEventListener("blur", onclose);
      addEventListener("resize", onclose);
    });
    return () => {
      clearTimeout(id);
      clearTimeout(queryTimer);
      removeEventListener("pointerdown", away, true);
      removeEventListener("scroll", away, { capture: true });
      removeEventListener("blur", onclose);
      removeEventListener("resize", onclose);
      // After the item's action has run: if it left focus nowhere, put it back.
      setTimeout(() => {
        const now = document.activeElement;
        if ((!now || now === document.body) && restore?.isConnected) restore.focus({ preventScroll: true });
      });
    };
  });

  function key(e: KeyboardEvent) {
    const btns = enabled();
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    const go = (j: number) => {
      e.preventDefault();
      btns[j]?.focus();
    };
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    } else if (e.key === "Tab") {
      e.preventDefault();
      onclose();
    } else if (e.key === "ArrowDown") go(i < 0 ? 0 : (i + 1) % btns.length);
    else if (e.key === "ArrowUp") go(i < 0 ? btns.length - 1 : (i - 1 + btns.length) % btns.length);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(btns.length - 1);
    else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && e.key !== " ") {
      // Typeahead: letters typed within 600 ms build a prefix; the search starts after the
      // current item, so pressing the same letter again cycles.
      query += e.key.toLowerCase();
      clearTimeout(queryTimer);
      queryTimer = setTimeout(() => (query = ""), 600);
      const label = (b: HTMLButtonElement) => b.querySelector(".l")?.textContent?.trim().toLowerCase() ?? "";
      const from = query.length === 1 ? i + 1 : Math.max(i, 0);
      for (let k = 0; k < btns.length; k++) {
        const j = (from + k) % btns.length;
        if (label(btns[j]).startsWith(query)) return go(j);
      }
    }
  }
</script>

<div
  class="menu"
  role="menu"
  tabindex="-1"
  bind:this={el}
  style:left="{pos.left}px"
  style:top="{pos.top}px"
  style:transform-origin={pos.origin}
  onkeydown={key}
  onpointerleave={() => el.focus({ preventScroll: true })}
>
  {#each items as item, i (i)}
    {#if item === "sep"}
      <div class="sep" role="separator"></div>
    {:else}
      <button
        role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
        aria-checked={item.checked}
        tabindex="-1"
        class:danger={item.danger}
        disabled={item.disabled}
        onpointermove={(e) => document.activeElement !== e.currentTarget && e.currentTarget.focus({ preventScroll: true })}
        onclick={() => {
          onclose();
          item.action();
        }}
      >
        {#if item.checked !== undefined}{#if item.checked}<Icon name="check" size={14} />{:else}<span class="sp"></span>{/if}{:else if item.icon}<Icon name={item.icon} size={14} />{:else}<span class="sp"></span>{/if}
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
    max-height: calc(100vh - 16px);
    overflow-y: auto;
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
  /* One highlight for pointer and keys: hovering focuses the item. */
  button:focus {
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
