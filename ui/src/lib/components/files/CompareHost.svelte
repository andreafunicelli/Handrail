<script lang="ts">
  import { onMount } from "svelte";
  import CompareDialog from "./CompareDialog.svelte";
  import { compare } from "./compare.svelte";

  /** Shows the "compare before overwrite" dialog when the store picked this host (see `compare`). */
  let { rank }: { rank: number } = $props();
  let anchor: HTMLSpanElement;
  let id = $state(0);

  onMount(() => {
    const h = compare.register(rank, () => anchor?.isConnected && anchor.getClientRects().length > 0);
    id = h.id;
    return h.leave;
  });
</script>

<span class="anchor" bind:this={anchor} aria-hidden="true"></span>
{#if compare.req && compare.req.host === id}
  {#key compare.req.id}
    <CompareDialog req={compare.req} onanswer={(ok) => compare.answer(ok)} />
  {/key}
{/if}

<style>
  /* Takes no room, yet has a box while its pane is shown (a hidden tab has none). */
  .anchor {
    position: absolute;
    display: block;
    width: 0;
    height: 0;
  }
</style>
