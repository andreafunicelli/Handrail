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
    // No webview context menu ("Back", "Reload", ...): components that want a menu show their
    // own. Text fields and selected text keep the native one for cut/copy/paste; the terminal's
    // hidden input does not (it has its own menu).
    const nativeMenu = (e: MouseEvent) => {
      const el = e.target instanceof Element ? e.target : null;
      if (el?.closest(".xterm")) return e.preventDefault();
      if (el?.closest("input, textarea, [contenteditable]:not([contenteditable='false'])")) return;
      // Right-clicking selected page text (chat, notes): keep Copy. xterm mirrors its own
      // selection into the page's, so that one doesn't count.
      const sel = getSelection();
      const range = sel && sel.rangeCount && sel.toString() ? sel.getRangeAt(0) : null;
      const at = range?.commonAncestorContainer;
      const host = at instanceof Element ? at : at?.parentElement;
      if (host && el && !host.closest(".xterm") && host.contains(el)) return;
      e.preventDefault();
    };
    document.addEventListener("contextmenu", nativeMenu);
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
