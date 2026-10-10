<script lang="ts">
  import { onMount } from "svelte";
  import AppShell from "./lib/components/shell/AppShell.svelte";
  import Onboarding from "./lib/components/vault/Onboarding.svelte";
  import Unlock from "./lib/components/vault/Unlock.svelte";
  import JoinVault from "./lib/components/vault/JoinVault.svelte";
  import Toasts from "./lib/components/common/Toasts.svelte";
  import { vault } from "./lib/stores/vault.svelte";
  import { toasts } from "./lib/stores/toasts.svelte";
  import "./lib/stores/settings.svelte"; // applies theme before first paint

  onMount(() => {
    vault.refresh().catch((e) => {
      toasts.error(e);
      vault.phase = "unlock";
    });
  });
</script>

{#if vault.phase === "ready"}
  <AppShell />
{:else if vault.phase === "create" || vault.phase === "kit"}
  <Onboarding />
{:else if vault.phase === "join"}
  <JoinVault />
{:else if vault.phase === "unlock"}
  <Unlock />
{/if}
<Toasts />
