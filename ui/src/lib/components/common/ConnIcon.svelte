<script lang="ts">
  import Icon, { isIconName } from "../icons/Icon.svelte";

  /** A connection's icon: a built-in glyph by name, otherwise the text itself (an emoji). */
  let { icon, size = 13 }: { icon: string | null | undefined; size?: number } = $props();
</script>

{#if icon}
  <span class="cicon" style:--s="{size}px" aria-hidden="true">
    {#if isIconName(icon)}<Icon name={icon} {size} />{:else}<span class="emoji">{icon}</span>{/if}
  </span>
{/if}

<style>
  .cicon {
    display: inline-grid;
    place-items: center;
    flex: none;
    width: calc(var(--s) + 3px);
    height: calc(var(--s) + 3px);
    color: var(--text-2);
  }
  .emoji {
    font-size: var(--s);
    line-height: 1;
  }
</style>
